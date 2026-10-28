//! elements/surprise (Отшельник): две метрики удивления рядом, полка от
//! них не растёт по-настоящему ни от одной.
//!
//! **S-static** — коэффициент сжатия входа ЗАМОРОЖЕННОЙ полкой, без права
//! рождения (`StreamingRecognizer`, `Store::lookup`, не `intern`). Выше —
//! вход лучше узнаётся существующим словарём.
//!
//! **S-growth** — биты словарного прироста при холостом прогоне рождения
//! на КЛОНЕ Store (реальный Store не трогается). Разница размера словаря
//! до/после × log2(словарь после).
//!
//! Диагностическое правило, не автоматический классификатор: обе сырые
//! величины возвращаются всегда, интерпретация "шум/рост/знакомо" — решение
//! читающего отчёт, не зашитый порог. Повторяет axmg (surprise.rs).

use axmva_core::{merge, Sequence, Store, StreamingRecognizer, TokenId};
use std::collections::HashSet;

/// S-static: коэффициент свёртки `input` замороженным `store` (без права
/// рождения). `store` не изменяется.
pub fn s_static(store: &Store, input: &[u8]) -> f64 {
    s_static_focused(store, input, None)
}

/// Как `s_static`, но свёртка ограничена `focused` (например, выходом
/// elements/death) — composite вне фокуса недоступен для свёртки, даже
/// если его тело ещё существует в Store.
pub fn s_static_focused(store: &Store, input: &[u8], focused: Option<&HashSet<TokenId>>) -> f64 {
    if input.is_empty() {
        return 1.0;
    }
    let out = StreamingRecognizer::recognize_focused(store, input, focused);
    let vocab_size = store.len();
    if vocab_size <= 1 || out.is_empty() {
        return 1.0;
    }
    let bits_per_token = (vocab_size as f64).log2();
    let compressed_bytes = out.len() as f64 * bits_per_token / 8.0;
    if compressed_bytes <= 0.0 {
        return 1.0;
    }
    input.len() as f64 / compressed_bytes
}

/// S-growth: биты словарного прироста на клоне `store`, прогнанном
/// `merge::run` (с правом рождения) на `input`. Реальный `store` не
/// трогается.
pub fn s_growth(store: &Store, input: &[u8], threshold: u64) -> f64 {
    let mut probe = store.clone();
    let vocab_before = probe.len();
    let ids: Vec<TokenId> = input
        .iter()
        .map(|&b| {
            let id = probe.intern(0, &[b as u32]);
            probe.touch(id);
            id
        })
        .collect();
    let seq = Sequence::from_ids(&probe, ids);
    let _ = merge::run(&mut probe, seq, threshold);
    let vocab_after = probe.len();
    if vocab_after <= vocab_before {
        return 0.0;
    }
    let new_tokens = (vocab_after - vocab_before) as f64;
    new_tokens * (vocab_after as f64).log2()
}

/// Вердикт о характере входящих данных — диагностика, не гейт.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurpriseVerdict {
    /// Высокая сжимаемость, нет прироста словаря (знакомый контекст).
    Familiar,
    /// Положительный прирост словаря (рождение новых паттернов).
    NovelGrowth,
    /// Низкая сжимаемость и отсутствие роста (бессистемный шум).
    Noise,
}

pub fn compute_surprise_score(s_static: f64, s_growth: f64) -> f64 {
    let static_factor = if s_static > 0.0 { 1.0 / s_static } else { 1.0 };
    let growth_factor = if s_growth > 0.0 { s_growth / (1.0 + s_growth) } else { 0.0 };
    static_factor + growth_factor
}

pub fn classify_surprise(s_static: f64, s_growth: f64) -> SurpriseVerdict {
    if s_growth > 0.0 {
        SurpriseVerdict::NovelGrowth
    } else if s_static > 1.25 {
        SurpriseVerdict::Familiar
    } else {
        SurpriseVerdict::Noise
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurpriseReport {
    pub s_static: f64,
    pub s_growth: f64,
    pub surprise_score: f64,
    pub verdict: SurpriseVerdict,
}

/// Обе метрики за один вызов.
pub fn surprise(store: &Store, input: &[u8], threshold: u64) -> SurpriseReport {
    let s_stat = s_static(store, input);
    let s_grow = s_growth(store, input, threshold);
    SurpriseReport {
        s_static: s_stat,
        s_growth: s_grow,
        surprise_score: compute_surprise_score(s_stat, s_grow),
        verdict: classify_surprise(s_stat, s_grow),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axmva_wheel::WheelState;

    #[test]
    fn s_static_perfectly_recognizes_frozen_vocabulary_input() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab cd cd ab cd ab cd", 2);
        let s = s_static(&wheel.store, b"ab ab cd cd ab cd ab cd");
        assert!(s > 1.0, "уже виденный текст обязан сжаться лучше 1.0, получили {s}");
    }

    #[test]
    fn s_static_does_not_mutate_the_store() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab cd cd", 2);
        let len_before = wheel.store.len();
        let _ = s_static(&wheel.store, b"totally novel gibberish qzx jkv");
        assert_eq!(wheel.store.len(), len_before, "s_static обязан быть read-only");
    }

    #[test]
    fn s_growth_does_not_mutate_the_real_store() {
        let mut wheel = WheelState::new();
        wheel.turn(b"ab ab cd cd", 2);
        let len_before = wheel.store.len();
        let _ = s_growth(&wheel.store, b"ef ef gh gh ef gh", 2);
        assert_eq!(wheel.store.len(), len_before, "s_growth обязан работать на клоне");
    }

    #[test]
    fn s_growth_is_positive_when_novel_repeated_pattern_would_be_born() {
        let mut wheel = WheelState::new();
        wheel.turn(b"the cat sat on the mat", 2);
        let g = s_growth(&wheel.store, b"xy xy xy xy", 2);
        assert!(g > 0.0, "новый повторяющийся паттерн обязан дать положительный S-growth, получили {g}");
    }

    #[test]
    fn s_growth_is_zero_when_nothing_new_would_be_born() {
        let mut wheel = WheelState::new();
        wheel.turn(b"aaaa bbbb", 2);
        let g = s_growth(&wheel.store, b"q", 2);
        assert_eq!(g, 0.0);
    }
}
