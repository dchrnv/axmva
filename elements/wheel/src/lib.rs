//! elements/wheel: Колесо — оборот как единица времени.
//!
//! Один оборот = одна атомарная итерация: принять порцию байт, дописать в
//! конец накопленной последовательности, прогнать слияние на ВСЕЙ текущей
//! последовательности (уже свёрнутый префикс просто не даёт новых слияний
//! сам в себе — это не лишняя работа, это наблюдаемая скорость). Время
//! внутри системы это номер оборота, не час — штамп ставит только Колесо,
//! на границе между оборотами (оборот не прерывается).
//!
//! Поведение повторяет проверенную axmg (wheel.rs), переписано как своя
//! реализация. Отличие: здесь нет Surprise (s_static/s_growth/verdict) —
//! это отдельный элемент поверх Колеса, не часть его контракта. См.
//! passport.yaml этого элемента.

use axmva_core::{merge, Sequence, Store, TokenId};
use std::collections::HashMap;

/// Отчёт об обороте: что произошло с этой порцией, без оценки surprise —
/// это дело отдельного элемента, читающего выходы Колеса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnReport {
    pub revolution: u32,
    pub bytes_ingested: usize,
    pub new_tokens_count: usize,
    pub sequence_len: usize,
}

pub struct WheelState {
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
}

impl WheelState {
    pub fn new() -> Self {
        let mut store = Store::new();
        store.init_factory();
        Self::with_store(store)
    }

    pub fn with_store(store: Store) -> Self {
        WheelState {
            store,
            sequence: Sequence { ids: Vec::new(), levels: Vec::new() },
            revolution: 0,
            last_touched_revolution: HashMap::new(),
        }
    }

    /// Один оборот: атомарно (Р5 в источнике) — внешний код не видит
    /// промежуточного состояния, пока волна слияний для этой порции не
    /// сойдётся.
    pub fn turn(&mut self, chunk: &[u8], threshold: u64) -> TurnReport {
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
            }
        }

        let final_levels = out.ids.iter().map(|&id| self.store.level_of(id)).collect();
        let sequence_len = out.ids.len();
        self.sequence = Sequence { ids: out.ids, levels: final_levels };

        let new_tokens_count = self.store.len().saturating_sub(vocab_before);

        TurnReport {
            revolution: self.revolution,
            bytes_ingested: chunk.len(),
            new_tokens_count,
            sequence_len,
        }
    }

    /// Прогнать весь вход оборотами по `REVOLUTION_CHUNK_BYTES`.
    pub fn run_corpus(&mut self, bytes: &[u8], threshold: u64) -> Vec<TurnReport> {
        bytes
            .chunks(axmva_core::REVOLUTION_CHUNK_BYTES)
            .map(|chunk| self.turn(chunk, threshold))
            .collect()
    }

    /// Возраст токена в оборотах: сколько оборотов прошло с последнего
    /// прямого касания. `None`, если токен ни разу не был в свёртке.
    pub fn age(&self, token: TokenId) -> Option<u32> {
        self.last_touched_revolution
            .get(&token)
            .map(|&last| self.revolution.saturating_sub(last))
    }
}

impl Default for WheelState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revolution_count_matches_chunking() {
        let mut wheel = WheelState::new();
        let bytes = vec![b'a'; axmva_core::REVOLUTION_CHUNK_BYTES * 2 + 1];
        let reports = wheel.run_corpus(&bytes, 2);
        assert_eq!(reports.len(), 3);
        assert_eq!(wheel.revolution, 3);
    }

    #[test]
    fn sequence_grows_and_collapses_across_revolutions() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab cd cd ab cd ab cd ", 2);
        let after_first = wheel.sequence.ids.len();
        assert!(after_first > 0);
        wheel.turn(b"ab ab cd cd ab cd ab cd ", 2);
        assert!(
            wheel.sequence.ids.len() < after_first + 24,
            "вторая порция обязана частично слиться с хвостом первой, не просто дописаться"
        );
    }

    #[test]
    fn last_touched_revolution_tracks_direct_fold_presence() {
        let mut wheel = WheelState::new();
        wheel.turn(b"xyz", 2);
        assert_eq!(wheel.revolution, 1);
        for &id in &wheel.sequence.ids {
            assert_eq!(wheel.last_touched_revolution.get(&id), Some(&1));
        }
    }

    #[test]
    fn stale_token_sitting_unchanged_does_not_get_timestamp_refreshed() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab", 2);
        let ab = wheel
            .sequence
            .ids
            .iter()
            .copied()
            .find(|&id| wheel.store.bytes_of(id) == b"ab")
            .expect("'ab' должен был слиться в композит");
        assert_eq!(wheel.last_touched_revolution.get(&ab), Some(&1));

        wheel.turn(b"zzz", 2);
        wheel.turn(b"zzz", 2);
        assert!(
            wheel.sequence.ids.contains(&ab),
            "'ab' обязан физически оставаться в последовательности"
        );
        assert_eq!(
            wheel.last_touched_revolution.get(&ab),
            Some(&1),
            "штамп времени не должен переписываться, пока токен просто лежит нетронутым"
        );
        assert_eq!(wheel.age(ab), Some(2), "возраст = текущий оборот (3) - last_touched (1)");
    }
}
