//! Selecting a pair of axes from a pool of candidate measures, by balance
//! and independence. If no pair in the pool passes both criteria, no
//! axes are selected at all — this is a deliberate failure mode, not an
//! edge case to paper over (drafts/axmv2-deck-map.md: "если пары мер,
//! проходящей критерии, нет, слой стихий целиком не создаётся").

use crate::measure::Binarizer;
use std::collections::HashMap;
use std::hash::Hash;

/// Mutual information (in bits) between two paired boolean samples.
/// Near 0 means the two variables are close to independent; 1.0 means
/// one perfectly determines the other (for balanced 50/50 variables).
pub fn mutual_information(x: &[bool], y: &[bool]) -> f64 {
    assert_eq!(x.len(), y.len(), "paired samples must have the same length");
    let n = x.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let mut joint = [[0usize; 2]; 2]; // joint[x as usize][y as usize]
    for (&a, &b) in x.iter().zip(y.iter()) {
        joint[a as usize][b as usize] += 1;
    }
    let mut mi = 0.0;
    for xi in 0..2 {
        let px = (joint[xi][0] + joint[xi][1]) as f64 / n;
        for yi in 0..2 {
            let pxy = joint[xi][yi] as f64 / n;
            if pxy <= 0.0 {
                continue;
            }
            let py = (joint[0][yi] + joint[1][yi]) as f64 / n;
            mi += pxy * (pxy / (px * py)).log2();
        }
    }
    mi
}

/// One candidate measure: a named, per-id value map over the population
/// being evaluated this round.
pub struct Candidate<'a, Id> {
    pub name: &'a str,
    pub values: &'a HashMap<Id, f64>,
}

/// Thresholds a pair of measures must clear to be accepted as axes.
#[derive(Debug, Clone, Copy)]
pub struct SelectionCriteria {
    /// Maximum allowed deviation from a perfect 0.5 balance fraction.
    pub max_balance_deviation: f64,
    /// Maximum allowed mutual information (bits) between the two axes.
    pub max_mutual_information: f64,
}

impl Default for SelectionCriteria {
    fn default() -> Self {
        Self { max_balance_deviation: 0.15, max_mutual_information: 0.1 }
    }
}

/// The result of a successful selection.
#[derive(Debug, Clone)]
pub struct SelectedAxes<'a> {
    pub first_name: &'a str,
    pub second_name: &'a str,
    pub first_binarizer: Binarizer,
    pub second_binarizer: Binarizer,
    pub balance: (f64, f64),
    pub mutual_information: f64,
}

/// Evaluate every pair in `pool` over `ids` and return the pair with the
/// lowest mutual information among those that clear `criteria`. `None`
/// if no pair clears the criteria (deliberate — the caller is expected
/// to drop the whole axes layer for this cycle, not force a bad pair).
pub fn select_pair<'a, Id: Eq + Hash + Copy>(
    pool: &[Candidate<'a, Id>],
    ids: &[Id],
    criteria: &SelectionCriteria,
) -> Option<SelectedAxes<'a>> {
    let mut best: Option<SelectedAxes<'a>> = None;

    for i in 0..pool.len() {
        for j in (i + 1)..pool.len() {
            let a = &pool[i];
            let b = &pool[j];

            let a_values: Vec<f64> = match ids.iter().map(|id| a.values.get(id).copied()).collect() {
                Some(v) => v,
                None => continue, // not every id has a value for this measure — skip
            };
            let b_values: Vec<f64> = match ids.iter().map(|id| b.values.get(id).copied()).collect() {
                Some(v) => v,
                None => continue,
            };

            let bin_a = Binarizer::from_median(&a_values);
            let bin_b = Binarizer::from_median(&b_values);
            let bal_a = bin_a.balance_fraction(&a_values);
            let bal_b = bin_b.balance_fraction(&b_values);
            if (bal_a - 0.5).abs() > criteria.max_balance_deviation {
                continue;
            }
            if (bal_b - 0.5).abs() > criteria.max_balance_deviation {
                continue;
            }

            let bits_a: Vec<bool> = a_values.iter().map(|&v| bin_a.classify(v)).collect();
            let bits_b: Vec<bool> = b_values.iter().map(|&v| bin_b.classify(v)).collect();
            let mi = mutual_information(&bits_a, &bits_b);
            if mi > criteria.max_mutual_information {
                continue;
            }

            let candidate = SelectedAxes {
                first_name: a.name,
                second_name: b.name,
                first_binarizer: bin_a,
                second_binarizer: bin_b,
                balance: (bal_a, bal_b),
                mutual_information: mi,
            };

            best = Some(match best {
                None => candidate,
                Some(current) if candidate.mutual_information < current.mutual_information => candidate,
                Some(current) => current,
            });
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfectly_correlated_booleans_give_one_bit_of_mutual_information() {
        let x = vec![true, true, false, false];
        let y = vec![true, true, false, false];
        let mi = mutual_information(&x, &y);
        assert!((mi - 1.0).abs() < 1e-9, "got {mi}");
    }

    #[test]
    fn independent_balanced_booleans_give_near_zero_mutual_information() {
        let x = vec![true, true, false, false, true, true, false, false];
        let y = vec![true, false, true, false, true, false, true, false];
        let mi = mutual_information(&x, &y);
        assert!(mi.abs() < 1e-9, "got {mi}");
    }

    #[test]
    fn select_pair_picks_the_balanced_independent_pair_over_a_correlated_one() {
        let ids: Vec<u32> = (0..8).collect();

        // "a": 0,1,2,3 below median; 4,5,6,7 above — balanced.
        let a: HashMap<u32, f64> = ids.iter().map(|&id| (id, id as f64)).collect();
        // "b": correlated with "a" (same split).
        let b: HashMap<u32, f64> = ids.iter().map(|&id| (id, id as f64 + 100.0)).collect();
        // "c": independent of "a" (alternates regardless of a's split).
        let c: HashMap<u32, f64> = ids.iter().map(|&id| (id, if id % 2 == 0 { 0.0 } else { 1.0 })).collect();

        let pool = vec![
            Candidate { name: "a", values: &a },
            Candidate { name: "b", values: &b },
            Candidate { name: "c", values: &c },
        ];

        let criteria = SelectionCriteria::default();
        let selected = select_pair(&pool, &ids, &criteria).expect("a pair should be found");

        // "a" & "b" are perfectly correlated (both split 0-3/4-7) — must
        // lose to "a" & "c", which are independent.
        let names = [selected.first_name, selected.second_name];
        assert!(names.contains(&"a") && names.contains(&"c"), "expected a+c, got {names:?}");
    }

    #[test]
    fn select_pair_returns_none_when_nothing_clears_the_bar() {
        let ids: Vec<u32> = (0..8).collect();
        // Two measures that are both perfectly correlated with each other.
        let a: HashMap<u32, f64> = ids.iter().map(|&id| (id, id as f64)).collect();
        let b: HashMap<u32, f64> = ids.iter().map(|&id| (id, id as f64)).collect();
        let pool = vec![Candidate { name: "a", values: &a }, Candidate { name: "b", values: &b }];

        let criteria = SelectionCriteria::default();
        assert!(select_pair(&pool, &ids, &criteria).is_none());
    }
}
