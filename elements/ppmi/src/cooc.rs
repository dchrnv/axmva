//! Накопление со-встречаемости (COO) и запекание в CSR.
//!
//! COO — растущий hashmap-аккумулятор (вставка/суммирование амортизированно
//! O(1), порядок поступления не важен — суммирование коммутативно). CSR —
//! испечённая форма для чтения, детерминированная: строки отсортированы по
//! TokenId, колонки внутри строки — тоже. Два запекания одного и того же
//! накопленного состояния дают побитово идентичный CSR независимо от
//! порядка накопления.

use axmva_core::TokenId;
use std::collections::HashMap;

#[derive(Default)]
pub struct CooAccumulator {
    counts: HashMap<(TokenId, TokenId), u64>,
}

impl CooAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accumulate(&mut self, events: &[(TokenId, TokenId, u64)]) {
        for &(token, context, weight) in events {
            *self.counts.entry((token, context)).or_insert(0) += weight;
        }
    }

    pub fn total_weight(&self) -> u64 {
        self.counts.values().sum()
    }

    pub fn distinct_pairs(&self) -> usize {
        self.counts.len()
    }

    pub fn weight_of(&self, token: TokenId, context: TokenId) -> u64 {
        self.counts.get(&(token, context)).copied().unwrap_or(0)
    }

    /// Испечь в CSR: строки — токены с хотя бы одной связью, отсортированы
    /// по возрастанию; колонки внутри строки — тоже. Заодно маргиналы
    /// (частоты токена/контекста — знаменатель PMI).
    pub fn bake_to_csr(&self) -> Csr {
        let mut by_row: HashMap<TokenId, Vec<(TokenId, u64)>> = HashMap::new();
        let mut row_marginal: HashMap<TokenId, u64> = HashMap::new();
        let mut col_marginal: HashMap<TokenId, u64> = HashMap::new();
        let mut total_weight: u64 = 0;

        for (&(token, context), &weight) in self.counts.iter() {
            by_row.entry(token).or_default().push((context, weight));
            *row_marginal.entry(token).or_insert(0) += weight;
            *col_marginal.entry(context).or_insert(0) += weight;
            total_weight += weight;
        }

        let mut row_index: Vec<TokenId> = by_row.keys().copied().collect();
        row_index.sort_unstable();

        let mut row_ptr: Vec<u32> = Vec::with_capacity(row_index.len() + 1);
        let mut col_idx: Vec<TokenId> = Vec::new();
        let mut values: Vec<u64> = Vec::new();

        row_ptr.push(0);
        for &row in &row_index {
            let mut entries = by_row.remove(&row).expect("row_index построен из ключей by_row");
            entries.sort_unstable_by_key(|&(ctx, _)| ctx);
            for (ctx, w) in entries {
                col_idx.push(ctx);
                values.push(w);
            }
            row_ptr.push(col_idx.len() as u32);
        }

        Csr { row_index, row_ptr, col_idx, values, row_marginal, col_marginal, total_weight }
    }
}

/// Compressed Sparse Row — испечённая, только для чтения форма.
pub struct Csr {
    pub row_index: Vec<TokenId>,
    pub row_ptr: Vec<u32>,
    pub col_idx: Vec<TokenId>,
    pub values: Vec<u64>,
    /// `row_marginal[t]` = сумма весов всех связей (t, *) — p(a) числитель.
    pub row_marginal: HashMap<TokenId, u64>,
    /// `col_marginal[t]` = сумма весов всех связей (*, t) — p(b) числитель.
    pub col_marginal: HashMap<TokenId, u64>,
    pub total_weight: u64,
}

impl Csr {
    pub fn row(&self, token: TokenId) -> impl Iterator<Item = (TokenId, u64)> + '_ {
        let range = match self.row_index.binary_search(&token) {
            Ok(i) => self.row_ptr[i] as usize..self.row_ptr[i + 1] as usize,
            Err(_) => 0..0,
        };
        self.col_idx[range.clone()].iter().copied().zip(self.values[range].iter().copied())
    }

    pub fn weight_of(&self, token: TokenId, context: TokenId) -> u64 {
        self.row(token).find(|&(ctx, _)| ctx == context).map(|(_, w)| w).unwrap_or(0)
    }

    pub fn row_count(&self) -> usize {
        self.row_index.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_sums_repeated_pairs() {
        let mut acc = CooAccumulator::new();
        acc.accumulate(&[(1, 2, 3), (1, 2, 4), (2, 3, 1)]);
        assert_eq!(acc.weight_of(1, 2), 7);
        assert_eq!(acc.weight_of(2, 3), 1);
        assert_eq!(acc.distinct_pairs(), 2);
        assert_eq!(acc.total_weight(), 8);
    }

    #[test]
    fn bake_matches_hand_computed_csr() {
        let mut acc = CooAccumulator::new();
        acc.accumulate(&[(1, 2, 5), (1, 3, 2), (2, 1, 1)]);
        let csr = acc.bake_to_csr();

        assert_eq!(csr.row_index, vec![1, 2]);
        let row1: Vec<(TokenId, u64)> = csr.row(1).collect();
        assert_eq!(row1, vec![(2, 5), (3, 2)]);
        let row2: Vec<(TokenId, u64)> = csr.row(2).collect();
        assert_eq!(row2, vec![(1, 1)]);
        assert_eq!(csr.row(99).count(), 0);
    }

    #[test]
    fn bake_is_deterministic_regardless_of_accumulation_order() {
        let events_forward: Vec<(TokenId, TokenId, u64)> =
            vec![(5, 1, 2), (1, 5, 3), (3, 3, 1), (5, 1, 4), (2, 8, 1)];
        let mut events_shuffled = events_forward.clone();
        events_shuffled.reverse();
        events_shuffled.swap(0, 2);

        let mut acc_a = CooAccumulator::new();
        for e in &events_forward {
            acc_a.accumulate(std::slice::from_ref(e));
        }
        let mut acc_b = CooAccumulator::new();
        for e in &events_shuffled {
            acc_b.accumulate(std::slice::from_ref(e));
        }

        let csr_a = acc_a.bake_to_csr();
        let csr_b = acc_b.bake_to_csr();
        assert_eq!(csr_a.row_index, csr_b.row_index);
        assert_eq!(csr_a.row_ptr, csr_b.row_ptr);
        assert_eq!(csr_a.col_idx, csr_b.col_idx);
        assert_eq!(csr_a.values, csr_b.values);
    }
}
