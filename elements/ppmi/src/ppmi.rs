//! PPMI — фильтр значимости связи.
//!
//! `PPMI(a,b) = max(0, log2( p(a,b) / (p(a)·p(b)) ))`. PMI ≈ 0 — "часто
//! вместе, потому что оба часты по отдельности" — мусор, фильтруется
//! порогом. Частотный пол: пары с сырым счётом ниже `frequency_floor` НЕ
//! оцениваются вовсе (`None`, не `PPMI = 0`) — PMI на редких парах
//! ненадёжен по построению (маленький числитель — шумная оценка).

use crate::cooc::Csr;
use axmva_core::TokenId;

/// Пара с сырым счётом ниже этого порога не оценивается.
pub const DEFAULT_FREQUENCY_FLOOR: u64 = 3;
/// Порог валидности связи.
pub const DEFAULT_PPMI_THRESHOLD: f64 = 2.0;

/// PPMI одной пары. `None`, если сырой счёт ниже частотного пола, либо
/// один из маргиналов отсутствует/нулевой.
pub fn ppmi(csr: &Csr, token: TokenId, context: TokenId, frequency_floor: u64) -> Option<f64> {
    let w = csr.weight_of(token, context);
    if w < frequency_floor {
        return None;
    }
    let row = *csr.row_marginal.get(&token)?;
    let col = *csr.col_marginal.get(&context)?;
    if row == 0 || col == 0 || csr.total_weight == 0 {
        return None;
    }
    let pmi = ((w as f64) * (csr.total_weight as f64) / ((row as f64) * (col as f64))).log2();
    Some(pmi.max(0.0))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PpmiEntry {
    pub token: TokenId,
    pub context: TokenId,
    pub raw_weight: u64,
    pub ppmi: f64,
}

/// Пройти весь CSR в детерминированном порядке, оценить PPMI на парах,
/// прошедших частотный пол, вернуть только валидные (PPMI >= threshold).
pub fn valid_links(csr: &Csr, frequency_floor: u64, threshold: f64) -> Vec<PpmiEntry> {
    let mut out = Vec::new();
    for &token in &csr.row_index {
        for (context, weight) in csr.row(token) {
            if let Some(p) = ppmi(csr, token, context, frequency_floor) {
                if p >= threshold {
                    out.push(PpmiEntry { token, context, raw_weight: weight, ppmi: p });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cooc::CooAccumulator;

    fn correlated_csr() -> Csr {
        let mut acc = CooAccumulator::new();
        acc.accumulate(&[(1, 2, 40), (1, 8, 5), (9, 2, 5), (9, 8, 40)]);
        acc.bake_to_csr()
    }

    #[test]
    fn correlated_pair_has_positive_ppmi() {
        let csr = correlated_csr();
        let p = ppmi(&csr, 1, 2, 3).expect("вес 40 >= пола 3");
        assert!((p - 0.830_074_998_557_687_5).abs() < 1e-9, "got {p}");
        assert!(p > 0.0);
    }

    #[test]
    fn anticorrelated_pair_floors_to_zero_not_negative() {
        let csr = correlated_csr();
        let p = ppmi(&csr, 1, 8, 3).expect("вес 5 >= пола 3");
        assert_eq!(p, 0.0, "анти-корреляция должна схлопнуться в 0 через max(0,·)");
    }

    #[test]
    fn below_frequency_floor_is_not_evaluated_at_all() {
        let mut acc = CooAccumulator::new();
        acc.accumulate(&[(1, 2, 2)]);
        let csr = acc.bake_to_csr();
        assert_eq!(
            ppmi(&csr, 1, 2, DEFAULT_FREQUENCY_FLOOR),
            None,
            "пара ниже пола обязана вернуть None, не PPMI=0"
        );
    }

    #[test]
    fn valid_links_respects_threshold_and_floor() {
        let csr = correlated_csr();
        let valid = valid_links(&csr, 3, 0.5);
        assert_eq!(valid.len(), 2);
        assert!(valid.iter().any(|e| e.token == 1 && e.context == 2));
        assert!(valid.iter().any(|e| e.token == 9 && e.context == 8));
    }
}
