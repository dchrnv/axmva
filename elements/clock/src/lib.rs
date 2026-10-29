//! elements/clock: домен-специфичные "часы" — связывает elements/epoch
//! (оборот, last_touched), elements/surprise (S-growth этого оборота) и
//! axtime (обобщённые виды времени) для `TokenId`.
//!
//! Раньше эта стыковка жила только в tools/time-integration-demo (не
//! формальном элементе) — age_inner (вид 3 axtime) был настоящим только
//! там, вручную. Здесь это настоящая, протестированная часть пайплайна:
//! один вызов [`Clock::turn`] = один оборот `epoch`, с S-growth, посчитанным
//! на ЗАМОРОЖЕННОМ (до слияния) сторе этого чанка, и реальными касаниями,
//! переданными в `axtime::Time`.
//!
//! Явный дизайн-выбор: `epoch` и `surprise` остаются независимыми друг от
//! друга (см. их паспорта) — зависимость от обоих собрана здесь, не внутри
//! одного из них. `axtime` остаётся полностью обобщённым (depends_on: []) —
//! он не знает про `TokenId`, этот элемент инстанцирует `axtime::Time<TokenId>`
//! снаружи.

use axmva_core::TokenId;
use axmva_epoch::{EpochState, TurnReport as EpochTurnReport};
use axtime::Time;

/// Отчёт об обороте часов: оборот `epoch` плюс S-growth, посчитанный на
/// входе в этот оборот.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockTurnReport {
    pub revolution: u32,
    pub bytes_ingested: usize,
    pub new_tokens_count: usize,
    pub truncated_count: usize,
    pub growth: f64,
}

pub struct Clock {
    epoch: EpochState,
    time: Time<TokenId>,
}

impl Clock {
    pub fn new() -> Self {
        Self { epoch: EpochState::new(), time: Time::new() }
    }

    /// Ограничить длину рабочей последовательности epoch — см.
    /// `EpochState::with_max_sequence_len`.
    pub fn with_max_sequence_len(mut self, max: usize) -> Self {
        self.epoch = self.epoch.with_max_sequence_len(max);
        self
    }

    pub fn epoch(&self) -> &EpochState {
        &self.epoch
    }

    pub fn store(&self) -> &axmva_core::Store {
        &self.epoch.store
    }

    /// Один оборот: S-growth этого чанка считается на замороженном сторе
    /// ДО merge (axtime::Time::advance), затем epoch.turn() реально
    /// слит, затем каждый токен, реально тронутый В ЭТОМ обороте
    /// (last_touched_revolution == новый revolution), передаётся в
    /// axtime::Time::touch.
    pub fn turn(&mut self, chunk: &[u8], threshold: u64) -> ClockTurnReport {
        let growth = axmva_surprise::s_growth(&self.epoch.store, chunk, threshold);
        self.time.advance(growth);

        let report: EpochTurnReport = self.epoch.turn(chunk, threshold);

        for &id in &self.epoch.sequence.ids {
            if self.epoch.last_touched_revolution.get(&id) == Some(&report.revolution) {
                self.time.touch(id);
            }
        }

        ClockTurnReport {
            revolution: report.revolution,
            bytes_ingested: report.bytes_ingested,
            new_tokens_count: report.new_tokens_count,
            truncated_count: report.truncated_count,
            growth,
        }
    }

    /// Прогнать весь вход оборотами по `REVOLUTION_CHUNK_BYTES`.
    pub fn run_corpus(&mut self, bytes: &[u8], threshold: u64) -> Vec<ClockTurnReport> {
        bytes
            .chunks(axmva_core::REVOLUTION_CHUNK_BYTES)
            .map(|chunk| self.turn(chunk, threshold))
            .collect()
    }

    /// Вид 1 (axtime): порядковое время. Всегда совпадает с
    /// `epoch.revolution` — это проверенный инвариант, не совпадение.
    pub fn stamp(&self) -> u32 {
        self.time.stamp()
    }

    /// Вид 2 (axtime): возраст в оборотах с последнего касания.
    pub fn age(&self, token: TokenId) -> Option<u32> {
        self.time.age(token)
    }

    /// Вид 3 (axtime): возраст в накопленном S-growth с последнего
    /// касания — настоящий, не из демо-инструмента.
    pub fn age_inner(&self, token: TokenId) -> Option<f64> {
        self.time.age_inner(token)
    }

    /// Вид 5 (axtime): средний период появления токена по оборотам.
    pub fn period(&self, token: TokenId) -> Option<f64> {
        self.time.period(token)
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIRTH_THRESHOLD: u64 = 2;

    #[test]
    fn stamp_always_matches_epoch_revolution() {
        let mut clock = Clock::new();
        clock.turn(b"ab ab cd cd", BIRTH_THRESHOLD);
        clock.turn(b"ef ef gh gh", BIRTH_THRESHOLD);
        assert_eq!(clock.stamp(), clock.epoch().revolution);
    }

    #[test]
    fn age_is_zero_right_after_a_turn_touches_the_token() {
        let mut clock = Clock::new();
        clock.turn(b"xyz", BIRTH_THRESHOLD);
        for &id in &clock.epoch().sequence.ids {
            assert_eq!(clock.age(id), Some(0));
        }
    }

    #[test]
    fn age_inner_is_real_not_a_demo_only_number() {
        let mut clock = Clock::new();
        clock.turn(b"the quick brown fox", BIRTH_THRESHOLD); // рост словаря > 0
        let touched_id = clock.epoch().sequence.ids[0];
        clock.turn(b"zzz zzz zzz zzz zzz", BIRTH_THRESHOLD); // другой текст, но тоже что-то рождает
        clock.turn(b"zzz zzz zzz zzz zzz", BIRTH_THRESHOLD); // второй раз — почти не растёт

        // age_inner для застрявшего с первого оборота токена обязан быть
        // Some (накопленный рост с его последнего касания), не ошибкой.
        assert!(clock.age_inner(touched_id).is_some());
    }

    #[test]
    fn stale_token_age_inner_grows_slower_when_later_growth_is_small() {
        let mut clock = Clock::new();
        clock.turn(b"the quick brown fox jumps over the lazy dog repeatedly repeatedly", BIRTH_THRESHOLD);
        let stale_id = clock
            .epoch()
            .sequence
            .ids
            .iter()
            .copied()
            .find(|&id| clock.store().level_of(id) >= 1)
            .expect("должен быть хотя бы один составной токен");
        let age_inner_after_one_turn = clock.age_inner(stale_id).unwrap();
        assert_eq!(age_inner_after_one_turn, 0.0, "сразу после касания внутренний возраст = 0");

        // Повторный ввод уже виденного текста — почти не рождает новых
        // токенов (малый рост), значит age_inner растёт МЕДЛЕННО, даже
        // если age (в тиках) растёт на 1 каждый раз.
        clock.turn(b"the quick brown fox jumps over the lazy dog repeatedly repeatedly", BIRTH_THRESHOLD);
        let age_ticks = clock.age(stale_id);
        let age_inner_after_two_turns = clock.age_inner(stale_id).unwrap();
        assert_eq!(age_ticks, Some(1), "один оборот прошёл");
        assert!(age_inner_after_two_turns < 50.0, "второй прогон того же текста почти не рождает нового — внутренний возраст обязан остаться малым, получили {age_inner_after_two_turns}");
    }

    #[test]
    fn run_corpus_chunks_by_revolution_chunk_bytes() {
        let mut clock = Clock::new();
        let bytes = vec![b'a'; axmva_core::REVOLUTION_CHUNK_BYTES * 2 + 1];
        let reports = clock.run_corpus(&bytes, BIRTH_THRESHOLD);
        assert_eq!(reports.len(), 3);
        assert_eq!(clock.stamp(), 3);
    }
}
