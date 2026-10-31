//! elements/port: единственная дверь между ядром (core+epoch+death+
//! surprise+recall) и внешним миром — по чертежу axmg (раздел 5: "Порт").
//!
//! НЕ честный порт `axmg::port.rs` — несколько осознанных расхождений,
//! найденных при чтении реального референса, не придуманных заранее:
//!
//! 1. **`PortSnapshot::recognize` делегирует в `StreamingRecognizer`**,
//!    не дублирует его собственным trie-распознавателем, как axmg. Два
//!    разных алгоритма распознавания на одном и том же `Store` могли бы
//!    расходиться — это реальный риск корректности, не стилистика.
//! 2. **Снимок для чтения — это просто борроу уже загруженного
//!    `EpochState`/`DeathLog`, не отдельный mmap-формат.** В axmg
//!    `PortSnapshot::open_mmap` мапит файл через `unsafe { Mmap::map }`,
//!    но затем ИГНОРИРУЕТ мапнутые байты и строит пустой
//!    `Store::init_factory()` — "снапшот" никогда реально не
//!    восстанавливается (мёртвый, но выглядящий рабочим код). Загрузка
//!    снимка здесь идёт через уже верифицированный `elements/persist`
//!    (`load_epoch_and_death` — проверки `DanglingChild`/`CorruptLog`),
//!    без единого `unsafe`.
//! 3. **`CandidateFilter` делает то, что заявляет, а не декорацию.**
//!    `axmg::port::Moderator` объявляет `min_context_links`/
//!    `max_saturation_ratio`, но ни одно поле не читается внутри
//!    `moderate()` — единственная реальная проверка (непустые дети у
//!    составного токена) структурно недостижима через `Store::intern`.
//!    Здесь `min_touch_count` реально используется (`Store::count_of`).
//!    Более богатая политика (по PPMI-насыщенности) требует бюджета
//!    роста связей — это Шаг 7 ROADMAP, не продублировано здесь заранее.
//! 4. **`receive` не обходит generational-GC Смерти.** `axmg::Port::
//!    receive_and_focus` пишет `Focus` в лог напрямую, минуя
//!    достижимость `mark()`/`weight_cut()` — второй, несогласованный
//!    путь входа в фокус. Здесь единственный мутирующий метод вызывает
//!    настоящие `epoch.turn_tagged` + `death.mark` + `death.weight_cut`,
//!    ничего не подменяет.
//! 5. **Имя "Moderator"/"Умеренность" не используется** — в чертеже
//!    deck-map карта 14 (Умеренность) уже означает перенос hot→cold
//!    счётчиков по α (Шаг 8 ROADMAP), другая механика. Использовать то
//!    же имя для фильтра кандидатов было бы путаницей в документах,
//!    поэтому здесь нейтральное `CandidateFilter`.
//! 6. **Тег источника теперь реально проверяется** — `external-input`'s
//!    контракт ("каждая порция несёт provenance-тег") раньше был только
//!    заявлен в паспорте, ничем не проверялся в коде. `receive` отвергает
//!    пустой/пробельный тег (`PortError::EmptySourceTag`), не принимает
//!    анонимный вход молча.

use axmva_core::{Store, StreamingRecognizer, TokenId};
use axmva_death::{DeathLog, MarkOutcome, WeightCutOutcome};
use axmva_epoch::{EpochState, TurnReport};
use axmva_persist::{LoadError, SaveError};
use axmva_recall::{recall, RecallConfig, RecallResult};
use axmva_surprise::{surprise, SurpriseReport};
use std::path::Path;

/// Отсев распознанных кандидатов перед тем, как их увидит вызывающий код.
/// Сильно сокращённая, но РЕАЛЬНАЯ версия того, что заявляет (но не
/// делает) `axmg::port::Moderator` — см. модульную документацию выше.
#[derive(Debug, Clone, Copy)]
pub struct CandidateFilter {
    /// Минимальное число фактических касаний (`Store::count_of`), чтобы
    /// токен прошёл фильтр. 0 — фильтр выключен, пропускает всё.
    pub min_touch_count: u64,
}

impl Default for CandidateFilter {
    fn default() -> Self {
        Self { min_touch_count: 0 }
    }
}

impl CandidateFilter {
    pub fn apply(&self, candidates: &[TokenId], store: &Store) -> Vec<TokenId> {
        candidates.iter().copied().filter(|&id| store.count_of(id) >= self.min_touch_count).collect()
    }
}

/// Ошибка приёма через порт.
#[derive(Debug, PartialEq, Eq)]
pub enum PortError {
    /// Тег источника пуст/из пробелов — нарушает контракт
    /// `elements/external-input` ("каждая порция несёт provenance-тег"),
    /// который до этого элемента нигде не проверялся в коде.
    EmptySourceTag,
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortError::EmptySourceTag => {
                write!(f, "тег источника пуст — порт требует непустой provenance-тег на каждую порцию")
            }
        }
    }
}

impl std::error::Error for PortError {}

/// Отчёт об одном вызове `Port::receive` — оборот + mark + weight_cut
/// целиком видны вызывающему коду, ничего не скрыто внутри.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortReceiveReport {
    pub turn: TurnReport,
    pub mark: MarkOutcome,
    pub weight_cut: WeightCutOutcome,
}

/// Read-only снимок состояния — весь внешний read-доступ идёт через эти
/// методы, не через прямые поля `EpochState`/`DeathLog`.
pub struct PortSnapshot<'a> {
    epoch: &'a EpochState,
    death_log: &'a DeathLog,
}

impl<'a> PortSnapshot<'a> {
    pub fn store(&self) -> &Store {
        &self.epoch.store
    }

    pub fn sequence(&self) -> &[TokenId] {
        &self.epoch.sequence.ids
    }

    /// Распознавание байт в существующие токены, ограничено текущим
    /// фокусом Смерти — read-only, делегирует в `StreamingRecognizer`
    /// (zero-mutation, доказано отдельно в elements/core).
    pub fn recognize(&self, bytes: &[u8]) -> Vec<TokenId> {
        StreamingRecognizer::recognize_focused(&self.epoch.store, bytes, Some(self.death_log.focused_set()))
    }

    /// Ассоциативное вспоминание, ограниченное текущим фокусом.
    pub fn recall(&self, query: &[u8], config: &RecallConfig) -> RecallResult {
        recall(&self.epoch.store, &self.epoch.sequence.ids, query, Some(self.death_log.focused_set()), config)
    }

    pub fn evaluate_surprise(&self, input: &[u8], threshold: u64) -> SurpriseReport {
        surprise(&self.epoch.store, input, threshold)
    }

    pub fn age(&self, token: TokenId) -> Option<u32> {
        self.epoch.age(token)
    }

    pub fn source_count(&self, token: TokenId) -> usize {
        self.epoch.source_count(token)
    }

    pub fn is_focused(&self, token: TokenId) -> bool {
        self.death_log.is_focused(token)
    }
}

/// Единственная дверь: владеет `EpochState` + `DeathLog`, даёт read-only
/// снимок (`snapshot`) и один мутирующий вход (`receive`).
pub struct Port {
    epoch: EpochState,
    death_log: DeathLog,
    filter: CandidateFilter,
}

impl Port {
    pub fn new(epoch: EpochState, death_log: DeathLog, filter: CandidateFilter) -> Self {
        Self { epoch, death_log, filter }
    }

    pub fn epoch(&self) -> &EpochState {
        &self.epoch
    }

    pub fn death_log(&self) -> &DeathLog {
        &self.death_log
    }

    pub fn snapshot(&self) -> PortSnapshot<'_> {
        PortSnapshot { epoch: &self.epoch, death_log: &self.death_log }
    }

    /// Распознать байты через текущий снимок и отфильтровать результат —
    /// READ-ONLY запрос, не трогает фокус. В отличие от
    /// `axmg::Port::receive_and_focus`, который совмещал распознавание,
    /// фильтр и запись в фокус в одном мутирующем вызове, здесь
    /// единственный способ реально что-то сфокусировать — пройти через
    /// `receive()`, не через побочный путь.
    pub fn recognize_candidates(&self, bytes: &[u8]) -> Vec<TokenId> {
        let raw = self.snapshot().recognize(bytes);
        self.filter.apply(&raw, &self.epoch.store)
    }

    /// Единственный мутирующий вход: оборот Колеса (с тегом источника) +
    /// mark + weight_cut Смерти, в этом порядке — тот же порядок, что
    /// уже проверен в tools/death-focus-bench. Отвергает пустой/
    /// пробельный тег вместо того, чтобы молча принять анонимный вход
    /// (см. модульную документацию, пункт 6).
    pub fn receive(&mut self, chunk: &[u8], threshold: u64, tag: &str) -> Result<PortReceiveReport, PortError> {
        if tag.trim().is_empty() {
            return Err(PortError::EmptySourceTag);
        }
        let turn = self.epoch.turn_tagged(chunk, threshold, tag);
        let mark = self.death_log.mark(&self.epoch);
        let weight_cut = self.death_log.weight_cut(&self.epoch);
        Ok(PortReceiveReport { turn, mark, weight_cut })
    }

    /// Сохранить Store + кэш epoch-состояния + лог Смерти в одной
    /// транзакции (`elements/persist::save_epoch_and_death`).
    pub fn save(&self, path: &Path) -> Result<(), SaveError> {
        axmva_persist::save_epoch_and_death(&self.epoch, &self.death_log, path)
    }

    /// Восстановить Port с диска (`elements/persist::load_epoch_and_death`).
    /// `filter` — настройка вызывающего кода, не персистится, как
    /// `max_sequence_len` у `EpochState`.
    pub fn load(path: &Path, filter: CandidateFilter) -> Result<Self, LoadError> {
        let (epoch, death_log) = axmva_persist::load_epoch_and_death(path)?;
        Ok(Self { epoch, death_log, filter })
    }
}

impl Default for Port {
    fn default() -> Self {
        Self::new(EpochState::new(), DeathLog::new(), CandidateFilter::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIRTH_THRESHOLD: u64 = 2;

    #[test]
    fn receive_rejects_empty_or_whitespace_source_tag() {
        let mut port = Port::default();
        assert_eq!(port.receive(b"abc", BIRTH_THRESHOLD, ""), Err(PortError::EmptySourceTag));
        assert_eq!(port.receive(b"abc", BIRTH_THRESHOLD, "   "), Err(PortError::EmptySourceTag));
        assert!(port.receive(b"abc", BIRTH_THRESHOLD, "corpus:a").is_ok());
    }

    #[test]
    fn receive_runs_turn_then_mark_then_weight_cut_every_time() {
        let mut port = Port::default();
        let report = port.receive(b"the cat sat on the mat, the cat sat again", BIRTH_THRESHOLD, "corpus:a").unwrap();
        assert_eq!(report.turn.revolution, 1);
        // mark() сразу после первого оборота обязан что-то сфокусировать —
        // иначе это не настоящий mark, а заглушка.
        assert!(report.mark.newly_focused > 0 || !port.death_log().focused_set().is_empty());
    }

    #[test]
    fn snapshot_recognize_matches_streaming_recognizer_directly() {
        let mut port = Port::default();
        port.receive(b"hello world hello world", BIRTH_THRESHOLD, "corpus:a").unwrap();

        let via_snapshot = port.snapshot().recognize(b"hello");
        let direct = StreamingRecognizer::recognize_focused(
            &port.epoch().store,
            b"hello",
            Some(port.death_log().focused_set()),
        );
        assert_eq!(via_snapshot, direct, "снимок обязан делегировать в тот же StreamingRecognizer, не в свой алгоритм");
    }

    #[test]
    fn candidate_filter_drops_tokens_below_min_touch_count() {
        let mut port = Port::new(EpochState::new(), DeathLog::new(), CandidateFilter { min_touch_count: 1_000_000 });
        port.receive(b"abc abc abc", BIRTH_THRESHOLD, "corpus:a").unwrap();
        let candidates = port.recognize_candidates(b"abc");
        assert!(candidates.is_empty(), "ни один токен не мог набрать миллион касаний на трёх байтах");
    }

    #[test]
    fn candidate_filter_default_passes_everything_recognized() {
        let mut port = Port::default();
        port.receive(b"abc abc abc", BIRTH_THRESHOLD, "corpus:a").unwrap();
        let via_filter = port.recognize_candidates(b"abc");
        let via_snapshot = port.snapshot().recognize(b"abc");
        assert_eq!(via_filter, via_snapshot, "фильтр по умолчанию (min_touch_count=0) не должен менять результат");
    }

    #[test]
    fn save_and_load_round_trip_preserves_everything_needed_for_recall() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("axmva_port_test_{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut port = Port::default();
        let corpus = b"the cat sat on the mat. the dog sat on the rug. the cat chased the dog.";
        port.receive(corpus, BIRTH_THRESHOLD, "corpus:cats-and-dogs").unwrap();

        let config = RecallConfig::default();
        let recall_before = port.snapshot().recall(b"cat", &config);

        port.save(&path).expect("save");
        let loaded = Port::load(&path, CandidateFilter::default()).expect("load");
        let recall_after = loaded.snapshot().recall(b"cat", &config);

        assert_eq!(recall_before.query_tokens, recall_after.query_tokens);
        let before_tokens: std::collections::HashSet<TokenId> = recall_before.candidates.iter().map(|c| c.token).collect();
        let after_tokens: std::collections::HashSet<TokenId> = recall_after.candidates.iter().map(|c| c.token).collect();
        assert_eq!(before_tokens, after_tokens, "recall через Port обязан находить те же кандидаты после перезагрузки");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn receive_after_load_continues_the_same_history_not_a_fresh_one() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("axmva_port_test_continue_{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut port = Port::default();
        port.receive(b"first chunk of real text here", BIRTH_THRESHOLD, "corpus:a").unwrap();
        port.save(&path).expect("save");

        let mut loaded = Port::load(&path, CandidateFilter::default()).expect("load");
        let revolution_before = loaded.epoch().revolution;
        loaded.receive(b"second chunk continues the same history", BIRTH_THRESHOLD, "corpus:a").unwrap();
        assert_eq!(loaded.epoch().revolution, revolution_before + 1, "revolution обязан продолжиться, не начаться с нуля");

        let _ = std::fs::remove_file(&path);
    }
}
