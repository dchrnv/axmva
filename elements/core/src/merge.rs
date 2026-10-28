//! Слияние частых пар поколениями — жадный лево-ассоциативный разбор, O(N).
//!
//! Каданс поколениями (не непрерывная очередь): волна кандидатов (пар с
//! частотой >= threshold) снимается один раз на старте поколения и
//! замораживается — новые пары, родившиеся слияниями внутри этого же
//! поколения, ждут следующего. `intern()` кандидата вызывается безусловно
//! один раз на кандидата, до перебора его позиций: рождение токена не
//! зависит от того, сколько его вхождений реально доживёт до обработки
//! после перекрытия с более ранними кандидатами той же волны. Оба решения
//! повторяют отлаженную реализацию axmg (merge.rs) — там же разбор
//! конкретных багов, которые дала обратная последовательность решений.
//!
//! Тай-брейк при равной частоте: порядок рождения токена (u32 id) —
//! систематически предпочитает раньше открытые токены, не нейтральный шум.

use crate::store::{Store, TokenId};
use std::collections::{BTreeSet, HashMap};

pub struct Sequence {
    pub ids: Vec<TokenId>,
    pub levels: Vec<u32>,
}

impl Sequence {
    pub fn from_ids(store: &Store, ids: Vec<TokenId>) -> Self {
        let levels = ids.iter().map(|&id| store.level_of(id)).collect();
        Sequence { ids, levels }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

type Pair = (TokenId, TokenId);

/// Предохранитель на число поколений — каскад сходится сам, это не рабочий
/// механизм остановки.
pub const MAX_GENERATIONS: usize = 200;

fn remove_occurrence(occurrences: &mut HashMap<Pair, BTreeSet<usize>>, pair: Pair, pos: usize) {
    if let Some(set) = occurrences.get_mut(&pair) {
        set.remove(&pos);
    }
}

fn insert_occurrence(occurrences: &mut HashMap<Pair, BTreeSet<usize>>, pair: Pair, pos: usize) {
    occurrences.entry(pair).or_default().insert(pos);
}

fn alive_snapshot(ids: &[TokenId], alive: &[bool]) -> Vec<TokenId> {
    (0..ids.len()).filter(|&i| alive[i]).map(|i| ids[i]).collect()
}

/// Прогнать слияние до сходимости поколениями (поколение не рождает новых
/// токенов — births == 0) либо до MAX_GENERATIONS. Возвращает свёрнутую
/// последовательность.
pub fn run(store: &mut Store, seq: Sequence, threshold: u64) -> Sequence {
    run_traced(store, seq, threshold).0
}

/// Как `run`, но дополнительно возвращает births по каждому поколению —
/// для диагностики/наблюдения, не для управления алгоритмом.
pub fn run_traced(store: &mut Store, seq: Sequence, threshold: u64) -> (Sequence, Vec<usize>) {
    let mut births_per_generation = Vec::new();
    let n = seq.len();
    if n < 2 {
        return (seq, births_per_generation);
    }
    let threshold = threshold as usize;

    let mut ids = seq.ids;
    let mut levels = seq.levels;
    let mut prev: Vec<i64> = (0..n as i64).map(|i| i - 1).collect();
    let mut next: Vec<i64> = (0..n as i64)
        .map(|i| if i + 1 < n as i64 { i + 1 } else { -1 })
        .collect();
    let mut alive = vec![true; n];

    let mut occurrences: HashMap<Pair, BTreeSet<usize>> = HashMap::new();
    for i in 0..n - 1 {
        insert_occurrence(&mut occurrences, (ids[i], ids[i + 1]), i);
    }

    for _generation in 0..MAX_GENERATIONS {
        // Волна: снимок кандидатов (и их позиций) на момент старта поколения.
        let mut wave: Vec<(Pair, Vec<usize>)> = occurrences
            .iter()
            .filter(|(_, positions)| positions.len() >= threshold)
            .map(|(&pair, positions)| (pair, positions.iter().copied().collect()))
            .collect();
        if wave.is_empty() {
            break;
        }

        wave.sort_by(|(pa, posa), (pb, posb)| posb.len().cmp(&posa.len()).then_with(|| pa.cmp(pb)));

        let mut births = 0usize;

        for (pair, frozen_positions) in wave {
            let new_level = store.level_of(pair.0).max(store.level_of(pair.1)) + 1;
            let before = store.birth_log().len();
            let new_id = store.intern(new_level, &[pair.0, pair.1]);
            if store.birth_log().len() > before {
                births += 1;
            }

            for pos in frozen_positions {
                if !alive[pos] {
                    continue;
                }
                let j = next[pos];
                if j == -1 || !alive[j as usize] {
                    continue;
                }
                let j = j as usize;
                if ids[pos] != pair.0 || ids[j] != pair.1 {
                    continue;
                }

                remove_occurrence(&mut occurrences, pair, pos);

                let p = prev[pos];
                let q = next[j];

                store.touch(new_id);

                if p != -1 {
                    let p = p as usize;
                    let old_left = (ids[p], pair.0);
                    remove_occurrence(&mut occurrences, old_left, p);
                    insert_occurrence(&mut occurrences, (ids[p], new_id), p);
                }

                if q != -1 {
                    let qu = q as usize;
                    let old_right = (pair.1, ids[qu]);
                    remove_occurrence(&mut occurrences, old_right, j);
                    insert_occurrence(&mut occurrences, (new_id, ids[qu]), pos);
                }

                ids[pos] = new_id;
                levels[pos] = new_level;
                alive[j] = false;
                next[pos] = q;
                if q != -1 {
                    prev[q as usize] = pos as i64;
                }
            }
        }

        births_per_generation.push(births);
        if births == 0 {
            break;
        }
    }

    let out_ids = alive_snapshot(&ids, &alive);
    let out_levels: Vec<u32> = (0..n).filter(|&i| alive[i]).map(|i| levels[i]).collect();
    (
        Sequence { ids: out_ids, levels: out_levels },
        births_per_generation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intern_bytes(store: &mut Store, bytes: &[u8]) -> Vec<TokenId> {
        bytes.iter().map(|&b| store.intern(0, &[b as u32])).collect()
    }

    fn rendered(store: &Store, seq: &Sequence) -> Vec<Vec<u8>> {
        seq.ids.iter().map(|&id| store.bytes_of(id)).collect()
    }

    #[test]
    fn non_overlapping_pairs_merge() {
        let mut store = Store::new();
        store.init_factory();
        let ids = intern_bytes(&mut store, b"abab");
        let seq = Sequence::from_ids(&store, ids);
        let out = run(&mut store, seq, 2);
        assert_eq!(rendered(&store, &out), vec![b"ab".to_vec(), b"ab".to_vec()]);
    }

    #[test]
    fn cascading_merge_builds_higher_level_token_over_generations() {
        let mut store = Store::new();
        store.init_factory();
        let ids = intern_bytes(&mut store, b"abababab");
        let seq = Sequence::from_ids(&store, ids);
        let out = run(&mut store, seq, 2);
        assert_eq!(out.len(), 2);
        for &id in &out.ids {
            assert_eq!(store.bytes_of(id), b"abab");
            assert!(store.level_of(id) >= 2);
        }
    }

    #[test]
    fn below_threshold_pair_is_never_merged() {
        let mut store = Store::new();
        store.init_factory();
        let ids = intern_bytes(&mut store, b"ab");
        let seq = Sequence::from_ids(&store, ids);
        let out = run(&mut store, seq, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(store.level_of(out.ids[0]), 0);
        assert_eq!(store.level_of(out.ids[1]), 0);
    }

    #[test]
    fn dedup_same_composition_across_positions_yields_same_id() {
        let mut store = Store::new();
        store.init_factory();
        let ids = intern_bytes(&mut store, b"xy xy");
        let seq = Sequence::from_ids(&store, ids);
        let out = run(&mut store, seq, 2);
        let xy_positions: Vec<TokenId> = out
            .ids
            .iter()
            .copied()
            .filter(|&id| store.bytes_of(id) == b"xy")
            .collect();
        assert_eq!(xy_positions.len(), 2);
        assert_eq!(xy_positions[0], xy_positions[1]);
    }

    #[test]
    fn same_input_gives_same_ids_every_time_determinism() {
        let mut store_a = Store::new();
        store_a.init_factory();
        let ids_a = intern_bytes(&mut store_a, b"the cat sat on the mat, the cat sat again");
        let seq_a = Sequence::from_ids(&store_a, ids_a);
        let out_a = run(&mut store_a, seq_a, 2);

        let mut store_b = Store::new();
        store_b.init_factory();
        let ids_b = intern_bytes(&mut store_b, b"the cat sat on the mat, the cat sat again");
        let seq_b = Sequence::from_ids(&store_b, ids_b);
        let out_b = run(&mut store_b, seq_b, 2);

        assert_eq!(out_a.ids, out_b.ids);
        assert_eq!(out_a.levels, out_b.levels);
    }
}
