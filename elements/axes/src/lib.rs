//! axaxes: a generic two-axis classifier — pick a pair of balanced,
//! independent measures out of a candidate pool, binarize each with
//! hysteresis against flapping, and classify items into one of four
//! cells. No knowledge of tokens, a Store, or PPMI inside.
//!
//! Mirrors the design in drafts/axmv2-deck-map.md ("Стихии и матрица",
//! "Паспорт стихии"): two domain-independent binary axes split the
//! population into four cells (аналогия: "земля/вода/огонь/воздух" —
//! that metaphor stays in documentation, the code uses neutral names
//! throughout, per that same draft's own dictionary).
//!
//! Pieces:
//! - [`measure`]: median-based [`measure::Binarizer`] — a fixed threshold
//!   applied to classify values, separate from computing that threshold.
//! - [`select`]: [`select::select_pair`] picks the best pair of measures
//!   from a pool by balance and independence (mutual information). If
//!   no pair clears the bar, it returns `None` — a deliberate failure
//!   mode: the whole axes layer gets dropped for this round, not forced.
//! - [`quadrant`]: [`quadrant::Quadrant`] (the classification itself),
//!   [`quadrant::Debounce`] (hysteresis — a value needs several
//!   consecutive disagreeing observations before it flips), and
//!   [`quadrant::InteractionMatrix`] (population-level aggregate: an
//!   item's own cell × its surroundings' cell).
//!
//! What this crate deliberately does not decide: which concrete measures
//! to put in the candidate pool, what "surroundings" means for a given
//! item, or whether a semantic label (a human word for a cell) tracks
//! the behavioral classification at all — drafts/axmv2-deck-map.md is
//! explicit that it usually won't, and that correlating them is an
//! empirical question to test, not something to assume.

pub mod measure;
pub mod quadrant;
pub mod select;

pub use measure::{median, Binarizer};
pub use quadrant::{Debounce, InteractionMatrix, Quadrant};
pub use select::{mutual_information, select_pair, Candidate, SelectedAxes, SelectionCriteria};
