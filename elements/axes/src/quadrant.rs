//! Classifying an item into one of four cells from two binary axes, with
//! hysteresis against flapping, plus a population-level interaction
//! matrix (object's cell × its surroundings' cell).

use std::collections::HashMap;
use std::hash::Hash;

/// A classification on two independent binary axes. Field names are
/// neutral by design — metaphor (стихии: земля/вода/огонь/воздух) lives
/// only in documentation, never in code identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quadrant {
    pub first: bool,
    pub second: bool,
}

impl Quadrant {
    pub fn classify(first: bool, second: bool) -> Self {
        Self { first, second }
    }

    /// A dense 0..=3 index, for use as a matrix/array position.
    pub fn as_index(&self) -> usize {
        (self.first as usize) * 2 + (self.second as usize)
    }

    pub fn from_index(index: usize) -> Self {
        Self { first: index & 0b10 != 0, second: index & 0b01 != 0 }
    }
}

/// Debounced (hysteresis-protected) boolean classification per id. A new
/// raw observation only flips the stable value after `required`
/// consecutive observations disagree with it — a single contrary
/// reading does not flip anything. The very first observation for an id
/// establishes its initial stable value immediately (there is nothing
/// to debounce against yet).
pub struct Debounce<Id> {
    required: u32,
    stable: HashMap<Id, bool>,
    pending: HashMap<Id, (bool, u32)>,
}

impl<Id: Eq + Hash + Copy> Debounce<Id> {
    /// `required` is how many consecutive disagreeing observations it
    /// takes to flip the stable value. Must be at least 1.
    pub fn new(required: u32) -> Self {
        assert!(required >= 1, "required must be at least 1");
        Self { required, stable: HashMap::new(), pending: HashMap::new() }
    }

    /// Feed one raw observation for `id`; returns the current stable
    /// (debounced) value after processing it.
    pub fn observe(&mut self, id: Id, raw: bool) -> bool {
        match self.stable.get(&id).copied() {
            None => {
                self.stable.insert(id, raw);
                raw
            }
            Some(current) if current == raw => {
                self.pending.remove(&id);
                current
            }
            Some(current) => {
                let (candidate, streak) = self.pending.get(&id).copied().unwrap_or((raw, 0));
                let streak = if candidate == raw { streak + 1 } else { 1 };
                if streak >= self.required {
                    self.stable.insert(id, raw);
                    self.pending.remove(&id);
                    raw
                } else {
                    self.pending.insert(id, (raw, streak));
                    current
                }
            }
        }
    }

    pub fn stable(&self, id: Id) -> Option<bool> {
        self.stable.get(&id).copied()
    }
}

/// Population-level aggregate: for how many (object, surroundings) cell
/// pairs did we see each combination, as a share of the total.
#[derive(Debug, Default)]
pub struct InteractionMatrix {
    counts: [[u64; 4]; 4],
    total: u64,
}

impl InteractionMatrix {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, own: Quadrant, surrounding: Quadrant) {
        self.counts[own.as_index()][surrounding.as_index()] += 1;
        self.total += 1;
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn count(&self, own: Quadrant, surrounding: Quadrant) -> u64 {
        self.counts[own.as_index()][surrounding.as_index()]
    }

    /// Share of all recorded pairs that fell in this (own, surrounding)
    /// cell. 0.0 if nothing has been recorded yet.
    pub fn share(&self, own: Quadrant, surrounding: Quadrant) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.count(own, surrounding) as f64 / self.total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrant_index_round_trips_and_covers_all_four_cells() {
        let all = [
            Quadrant::classify(false, false),
            Quadrant::classify(false, true),
            Quadrant::classify(true, false),
            Quadrant::classify(true, true),
        ];
        let mut indices: Vec<usize> = all.iter().map(Quadrant::as_index).collect();
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 1, 2, 3]);
        for q in all {
            assert_eq!(Quadrant::from_index(q.as_index()), q);
        }
    }

    #[test]
    fn debounce_first_observation_is_immediately_stable() {
        let mut d: Debounce<&str> = Debounce::new(2);
        assert_eq!(d.observe("a", true), true);
        assert_eq!(d.stable("a"), Some(true));
    }

    #[test]
    fn debounce_single_contrary_observation_does_not_flip() {
        let mut d: Debounce<&str> = Debounce::new(2);
        d.observe("a", true);
        assert_eq!(d.observe("a", false), true, "one contrary reading must not flip yet");
        assert_eq!(d.observe("a", true), true, "back to agreement — still stable true");
    }

    #[test]
    fn debounce_flips_after_required_consecutive_disagreements() {
        let mut d: Debounce<&str> = Debounce::new(2);
        d.observe("a", true);
        assert_eq!(d.observe("a", false), true, "1st disagreement — not yet");
        assert_eq!(d.observe("a", false), false, "2nd consecutive disagreement — flips");
        assert_eq!(d.stable("a"), Some(false));
    }

    #[test]
    fn debounce_tracks_ids_independently() {
        let mut d: Debounce<&str> = Debounce::new(1);
        d.observe("a", true);
        d.observe("b", false);
        assert_eq!(d.stable("a"), Some(true));
        assert_eq!(d.stable("b"), Some(false));
    }

    #[test]
    fn interaction_matrix_shares_are_consistent_with_counts() {
        let mut m = InteractionMatrix::new();
        let earth = Quadrant::classify(false, false);
        let fire = Quadrant::classify(true, false);

        m.record(earth, earth);
        m.record(earth, earth);
        m.record(earth, fire);
        m.record(fire, fire);

        assert_eq!(m.total(), 4);
        assert_eq!(m.count(earth, earth), 2);
        assert!((m.share(earth, earth) - 0.5).abs() < 1e-9);
        assert!((m.share(earth, fire) - 0.25).abs() < 1e-9);
        assert!((m.share(fire, fire) - 0.25).abs() < 1e-9);
        assert_eq!(m.share(fire, earth), 0.0);
    }
}
