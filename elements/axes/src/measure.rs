//! Median-based binarization — the shared mechanism behind splitting any
//! candidate measure into two balanced halves.

/// Median of a slice (sorts a local copy; does not mutate the input).
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("NaN in measure values"));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// A fixed threshold, established once (typically from a reference
/// population's median) and then applied to classify new values. Keeping
/// the threshold fixed instead of recomputing it per observation is what
/// makes balance something you can actually lose on new data, not a
/// tautology.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Binarizer {
    threshold: f64,
}

impl Binarizer {
    pub fn from_threshold(threshold: f64) -> Self {
        Self { threshold }
    }

    /// Establish the threshold from a reference population's median.
    pub fn from_median(reference: &[f64]) -> Self {
        Self { threshold: median(reference) }
    }

    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// `true` if `value` is strictly above the threshold.
    pub fn classify(&self, value: f64) -> bool {
        value > self.threshold
    }

    /// Fraction of `values` classified `true` — how close to 50/50 this
    /// binarizer's threshold is on a (possibly different) population.
    pub fn balance_fraction(&self, values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let above = values.iter().filter(|&&v| self.classify(v)).count();
        above as f64 / values.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_length() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
    }

    #[test]
    fn median_of_even_length_averages_middle_two() {
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
    }

    #[test]
    fn median_of_empty_is_zero() {
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn binarizer_from_median_splits_its_own_population_close_to_half() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let bin = Binarizer::from_median(&values);
        let frac = bin.balance_fraction(&values);
        assert!((frac - 0.5).abs() < 1e-9, "got {frac}");
    }

    #[test]
    fn fixed_threshold_can_be_unbalanced_on_different_data() {
        let bin = Binarizer::from_threshold(100.0);
        let skewed = vec![1.0, 2.0, 3.0, 4.0]; // all below the threshold
        assert_eq!(bin.balance_fraction(&skewed), 0.0, "a fixed threshold is not guaranteed to give 50/50 on new data");
    }
}
