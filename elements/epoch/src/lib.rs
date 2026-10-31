//! elements/epoch: оборот как единица времени — циклическая функция шага,
//! вызываемая раз за оборот. Переименован из "wheel" (метафора «Колесо»
//! остаётся только в документах/комментариях, не в коде) — механически
//! это просто шаг цикла, и код-имя "epoch" уже было записано в Словаре
//! deck-map (строка 10, Колесо), хотя сам axmg этому своему правилу не
//! последовал.
//!
//! Один оборот = одна атомарная итерация: принять порцию байт, дописать в
//! конец накопленной последовательности, прогнать слияние на ВСЕЙ текущей
//! последовательности (уже свёрнутый префикс просто не даёт новых слияний
//! сам в себе — это не лишняя работа, это наблюдаемая скорость). Время
//! внутри системы это номер оборота, не час — штамп ставит только этот шаг,
//! на границе между оборотами (оборот не прерывается).
//!
//! Поведение повторяет проверенную axmg (wheel.rs — файл реального
//! референса не переименован, только наш код), переписано как своя
//! реализация. Отличие: здесь нет Surprise (s_static/s_growth/verdict) —
//! это отдельный элемент поверх, не часть контракта. См. passport.yaml
//! этого элемента.

use axmva_core::{merge, Sequence, Store, TokenId};
use std::collections::{HashMap, HashSet};

/// Отчёт об обороте: что произошло с этой порцией, без оценки surprise —
/// это дело отдельного элемента, читающего выходы Колеса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnReport {
    pub revolution: u32,
    pub bytes_ingested: usize,
    pub new_tokens_count: usize,
    pub sequence_len: usize,
    /// Сколько самых старых элементов срезано с начала рабочей
    /// последовательности в этом обороте из-за `max_sequence_len`. 0, если
    /// лимит не задан или не достигнут — обрезка видна в отчёте, не тиха.
    pub truncated_count: usize,
}

pub struct EpochState {
    pub store: Store,
    pub sequence: Sequence,
    /// Номер последнего завершённого оборота (0 — ничего ещё не подано).
    pub revolution: u32,
    /// token -> оборот, на выходе которого токен последний раз был прямым
    /// элементом свёрнутой последовательности. НЕ обновляется, если токен
    /// просто продолжает неподвижно лежать в последовательности без
    /// реального касания (рост счётчика) в этом обороте — иначе окно
    /// "возраста" никогда никого не состарит.
    pub last_touched_revolution: HashMap<TokenId, u32>,
    /// token -> множество различных тегов источника, под которыми токен
    /// реально был тронут (то же условие, что у `last_touched_revolution`
    /// — не простое присутствие в последовательности). Заполняется только
    /// через `turn_tagged`/`run_corpus_tagged`; обычный `turn` его не
    /// трогает, остаётся пустым для токенов, тронутых только без тега.
    source_tags: HashMap<TokenId, HashSet<String>>,
    /// Верхняя граница длины рабочей последовательности (`sequence`), не
    /// полки (`store` — тела токенов никогда не удаляются). `None` —
    /// без ограничения (поведение по умолчанию, как раньше): измерено
    /// (tools/epoch-cost-bench), что стоимость оборота растёт линейно от
    /// длины последовательности, поэтому суммарная стоимость по всем
    /// оборотам квадратична от размера непрерывного потока — не проблема
    /// на разовых файлах в единицы МБ, которые мы тестировали, но станет
    /// проблемой на многодневном непрерывном потоке без лимита.
    max_sequence_len: Option<usize>,
}

impl EpochState {
    pub fn new() -> Self {
        let mut store = Store::new();
        store.init_factory();
        Self::with_store(store)
    }

    pub fn with_store(store: Store) -> Self {
        EpochState {
            store,
            sequence: Sequence { ids: Vec::new(), levels: Vec::new() },
            revolution: 0,
            last_touched_revolution: HashMap::new(),
            source_tags: HashMap::new(),
            max_sequence_len: None,
        }
    }

    /// Задать верхнюю границу длины рабочей последовательности — явный
    /// выбор вызывающего кода, не умолчание. При превышении самые старые
    /// элементы (с начала) срезаются в конце каждого оборота; `Store` не
    /// затрагивается — тела токенов остаются, срезается только рабочий
    /// буфер. Величина — решение вызывающего кода: вывести её можно из
    /// бюджета времени на оборот (cost в passport.yaml), а не скопировать
    /// готовое число.
    pub fn with_max_sequence_len(mut self, max: usize) -> Self {
        self.max_sequence_len = Some(max);
        self
    }

    pub fn max_sequence_len(&self) -> Option<usize> {
        self.max_sequence_len
    }

    /// Один оборот: атомарно (Р5 в источнике) — внешний код не видит
    /// промежуточного состояния, пока волна слияний для этой порции не
    /// сойдётся.
    pub fn turn(&mut self, chunk: &[u8], threshold: u64) -> TurnReport {
        self.turn_impl(chunk, threshold, None)
    }

    /// Как `turn`, но помечает каждый реально тронутый в этом обороте
    /// токен тегом источника (кто подал порцию — корпус, диалог, сенсор;
    /// назначается транспортом, не содержимым). `source_count` потом
    /// читает число РАЗНЫХ тегов на токен. См.
    /// drafts/axmv2-truth-vs-derived.md — тег это часть новой истины
    /// (поток порций), не отдельная колонка сама по себе.
    pub fn turn_tagged(&mut self, chunk: &[u8], threshold: u64, tag: &str) -> TurnReport {
        self.turn_impl(chunk, threshold, Some(tag))
    }

    fn turn_impl(&mut self, chunk: &[u8], threshold: u64, tag: Option<&str>) -> TurnReport {
        let vocab_before = self.store.len();

        let mut before_counts: HashMap<TokenId, u64> = HashMap::new();
        for &id in &self.sequence.ids {
            before_counts.entry(id).or_insert_with(|| self.store.count_of(id));
        }

        let mut ids = std::mem::take(&mut self.sequence.ids);
        ids.reserve(chunk.len());
        for &b in chunk {
            let id = self.store.intern(0, &[b as u32]);
            self.store.touch(id);
            ids.push(id);
        }

        let seq = Sequence::from_ids(&self.store, ids);
        let out = merge::run(&mut self.store, seq, threshold);

        self.revolution += 1;
        for &id in &out.ids {
            let touched_this_turn = match before_counts.get(&id) {
                Some(&old_count) => self.store.count_of(id) > old_count,
                None => true, // не было в перенесённом префиксе -> новое в этом обороте
            };
            if touched_this_turn {
                self.last_touched_revolution.insert(id, self.revolution);
                if let Some(t) = tag {
                    self.source_tags.entry(id).or_default().insert(t.to_string());
                }
            }
        }

        let mut final_ids = out.ids;
        let truncated_count = match self.max_sequence_len {
            Some(max) if final_ids.len() > max => {
                let excess = final_ids.len() - max;
                final_ids.drain(0..excess);
                excess
            }
            _ => 0,
        };

        let final_levels = final_ids.iter().map(|&id| self.store.level_of(id)).collect();
        let sequence_len = final_ids.len();
        self.sequence = Sequence { ids: final_ids, levels: final_levels };

        let new_tokens_count = self.store.len().saturating_sub(vocab_before);

        TurnReport {
            revolution: self.revolution,
            bytes_ingested: chunk.len(),
            new_tokens_count,
            sequence_len,
            truncated_count,
        }
    }

    /// Прогнать весь вход оборотами по `REVOLUTION_CHUNK_BYTES`.
    pub fn run_corpus(&mut self, bytes: &[u8], threshold: u64) -> Vec<TurnReport> {
        bytes
            .chunks(axmva_core::REVOLUTION_CHUNK_BYTES)
            .map(|chunk| self.turn(chunk, threshold))
            .collect()
    }

    /// Как `run_corpus`, но каждый оборот помечен тем же тегом источника
    /// (весь вход пришёл из одного транспорта за один вызов).
    pub fn run_corpus_tagged(&mut self, bytes: &[u8], threshold: u64, tag: &str) -> Vec<TurnReport> {
        bytes
            .chunks(axmva_core::REVOLUTION_CHUNK_BYTES)
            .map(|chunk| self.turn_tagged(chunk, threshold, tag))
            .collect()
    }

    /// Число РАЗНЫХ тегов источника, под которыми токен был реально
    /// тронут (не просто присутствовал в последовательности) — 0, если
    /// токен никогда не касался через `turn_tagged`/`run_corpus_tagged`.
    pub fn source_count(&self, token: TokenId) -> usize {
        self.source_tags.get(&token).map(|tags| tags.len()).unwrap_or(0)
    }

    /// Возраст токена в оборотах: сколько оборотов прошло с последнего
    /// прямого касания. `None`, если токен ни разу не был в свёртке.
    pub fn age(&self, token: TokenId) -> Option<u32> {
        self.last_touched_revolution
            .get(&token)
            .map(|&last| self.revolution.saturating_sub(last))
    }
}

impl Default for EpochState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revolution_count_matches_chunking() {
        let mut epoch = EpochState::new();
        let bytes = vec![b'a'; axmva_core::REVOLUTION_CHUNK_BYTES * 2 + 1];
        let reports = epoch.run_corpus(&bytes, 2);
        assert_eq!(reports.len(), 3);
        assert_eq!(epoch.revolution, 3);
    }

    #[test]
    fn sequence_grows_and_collapses_across_revolutions() {
        let mut epoch = EpochState::new();
        epoch.turn(b"ab ab cd cd ab cd ab cd ", 2);
        let after_first = epoch.sequence.ids.len();
        assert!(after_first > 0);
        epoch.turn(b"ab ab cd cd ab cd ab cd ", 2);
        assert!(
            epoch.sequence.ids.len() < after_first + 24,
            "вторая порция обязана частично слиться с хвостом первой, не просто дописаться"
        );
    }

    #[test]
    fn last_touched_revolution_tracks_direct_fold_presence() {
        let mut epoch = EpochState::new();
        epoch.turn(b"xyz", 2);
        assert_eq!(epoch.revolution, 1);
        for &id in &epoch.sequence.ids {
            assert_eq!(epoch.last_touched_revolution.get(&id), Some(&1));
        }
    }

    #[test]
    fn stale_token_sitting_unchanged_does_not_get_timestamp_refreshed() {
        let mut epoch = EpochState::new();
        epoch.turn(b"ab ab", 2);
        let ab = epoch
            .sequence
            .ids
            .iter()
            .copied()
            .find(|&id| epoch.store.bytes_of(id) == b"ab")
            .expect("'ab' должен был слиться в композит");
        assert_eq!(epoch.last_touched_revolution.get(&ab), Some(&1));

        epoch.turn(b"zzz", 2);
        epoch.turn(b"zzz", 2);
        assert!(
            epoch.sequence.ids.contains(&ab),
            "'ab' обязан физически оставаться в последовательности"
        );
        assert_eq!(
            epoch.last_touched_revolution.get(&ab),
            Some(&1),
            "штамп времени не должен переписываться, пока токен просто лежит нетронутым"
        );
        assert_eq!(epoch.age(ab), Some(2), "возраст = текущий оборот (3) - last_touched (1)");
    }

    #[test]
    fn unbounded_by_default_sequence_can_exceed_any_arbitrary_length() {
        // Поведение по умолчанию не изменилось — это регрессионный тест
        // на старое поведение, важный именно потому, что мы только что
        // добавили лимит, который легко было бы включить по умолчанию.
        let mut epoch = EpochState::new();
        assert_eq!(epoch.max_sequence_len(), None);
        let report = epoch.turn(b"the quick brown fox jumps over the lazy dog, repeated: the quick brown fox jumps over the lazy dog", 2);
        assert_eq!(report.truncated_count, 0);
    }

    #[test]
    fn with_max_sequence_len_truncates_from_the_front() {
        let mut epoch = EpochState::new().with_max_sequence_len(5);
        assert_eq!(epoch.max_sequence_len(), Some(5));

        let report = epoch.turn(b"abcdefghij", 100); // порог высокий -> почти ничего не слипается
        assert_eq!(epoch.sequence.ids.len(), 5, "длина обязана быть срезана до лимита");
        assert_eq!(report.sequence_len, 5);
        assert!(report.truncated_count > 0, "обрезка обязана быть видна в отчёте");

        // Срезаются САМЫЕ СТАРЫЕ (голова), не хвост: последние байты входа
        // ('g'..'j') должны остаться, первые ('a'..'c') — уйти.
        let remaining: Vec<u8> = epoch.sequence.ids.iter().map(|&id| epoch.store.bytes_of(id)[0]).collect();
        assert_eq!(remaining, vec![b'f', b'g', b'h', b'i', b'j']);
    }

    #[test]
    fn truncation_does_not_touch_store_bodies_only_the_working_sequence() {
        let mut epoch = EpochState::new().with_max_sequence_len(3);
        let before = epoch.store.len();
        epoch.turn(b"abcdefghij", 100);
        // Тела токенов (в т.ч. срезанных из sequence) остаются в Store —
        // обрезается только рабочий буфер, не полка.
        assert!(epoch.store.len() >= before, "Store только растёт, обрезка sequence не удаляет тела");
        assert_eq!(epoch.sequence.ids.len(), 3);
    }

    #[test]
    fn truncation_keeps_working_across_multiple_turns() {
        let mut epoch = EpochState::new().with_max_sequence_len(10);
        for _ in 0..5 {
            epoch.turn(b"xyzxyzxyzxyzxyzxyzxyzxyzxyzxyz", 2);
            assert!(epoch.sequence.ids.len() <= 10, "лимит обязан держаться на каждом обороте, не только на первом");
        }
    }

    #[test]
    fn plain_turn_never_populates_source_count() {
        let mut epoch = EpochState::new();
        epoch.turn(b"ab ab ab", 2);
        for &id in &epoch.sequence.ids {
            assert_eq!(epoch.source_count(id), 0, "обычный turn() не должен трогать тег источника вообще");
        }
    }

    #[test]
    fn turn_tagged_records_the_tag_only_for_tokens_really_touched_this_turn() {
        let mut epoch = EpochState::new();
        epoch.turn_tagged(b"ab ab", 2, "corpus:a");
        let ab = epoch.sequence.ids[0];
        assert_eq!(epoch.source_count(ab), 1);

        // Тот же тег повторно -> множество не растёт (это РАЗНЫЕ теги,
        // не счётчик касаний).
        epoch.turn_tagged(b"ab ab", 2, "corpus:a");
        assert_eq!(epoch.source_count(ab), 1, "повторный тот же тег не увеличивает число РАЗНЫХ источников");

        epoch.turn_tagged(b"ab ab", 2, "corpus:b");
        assert_eq!(epoch.source_count(ab), 2, "второй, другой тег обязан увеличить счёт");
    }

    #[test]
    fn stale_token_sitting_unchanged_does_not_gain_a_new_source_tag() {
        // Тот же критерий, что у last_touched_revolution: присутствие в
        // sequence без реального роста счётчика не считается касанием.
        let mut epoch = EpochState::new();
        epoch.turn_tagged(b"ab", 2, "corpus:a");
        let ab = epoch.sequence.ids[0];
        assert_eq!(epoch.source_count(ab), 1);

        // Следующий оборот не содержит 'ab' вовсе — токен просто лежит
        // нетронутым (если бы лежал) под другим тегом; здесь проверяем,
        // что чужой оборот без касания этого токена не меняет его счёт.
        epoch.turn_tagged(b"zz zz zz", 2, "corpus:b");
        assert_eq!(epoch.source_count(ab), 1, "токен не тронутый в этом обороте не получает новый тег");
    }

    #[test]
    fn run_corpus_tagged_tags_every_turn_with_the_same_source() {
        let mut epoch = EpochState::new();
        let bytes = vec![b'x'; axmva_core::REVOLUTION_CHUNK_BYTES + 10];
        epoch.run_corpus_tagged(&bytes, 2, "corpus:big");
        let touched: Vec<TokenId> = epoch.sequence.ids.clone();
        assert!(!touched.is_empty());
        for id in touched {
            assert!(epoch.source_count(id) >= 1, "каждый реально тронутый токен обязан получить тег");
        }
    }
}
