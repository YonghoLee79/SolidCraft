//! A range of parameters along a curve, from `lo` to `hi`.

/// The parameters of a curve from `lo` to `hi`. Fractions run from `lo` (0) to `hi` (1);
/// [`contains`](Self::contains) and [`clamp`](Self::clamp) take the ends in either order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
}

impl From<(f64, f64)> for Interval {
    fn from((lo, hi): (f64, f64)) -> Self {
        Interval { lo, hi }
    }
}

impl Interval {
    pub fn new(lo: f64, hi: f64) -> Self {
        Interval { lo, hi }
    }
    /// `hi - lo`.
    pub fn span(&self) -> f64 {
        self.hi - self.lo
    }
    /// The parameter a fraction `f` of the way from `lo` to `hi`.
    pub fn at(&self, f: f64) -> f64 {
        self.lo + self.span() * f
    }
    /// The parameter `k` steps of `n` from `lo` (`at(k / n)`, rounded as `span * k / n`).
    pub fn step(&self, k: usize, n: usize) -> f64 {
        self.lo + self.span() * k as f64 / n as f64
    }
    /// `n + 1` parameters evenly from `lo` to `hi`, both included.
    pub fn samples(self, n: usize) -> impl Iterator<Item = f64> {
        (0..=n).map(move |k| self.step(k, n))
    }
    pub fn mid(&self) -> f64 {
        (self.lo + self.hi) * 0.5
    }
    /// Whether `t` lies between the ends, both included.
    pub fn contains(&self, t: f64) -> bool {
        self.lo.min(self.hi) <= t && t <= self.lo.max(self.hi)
    }
    /// The ends in order, each moved out by `e`.
    pub fn inflated(&self, e: f64) -> Interval {
        Interval { lo: self.lo.min(self.hi) - e, hi: self.lo.max(self.hi) + e }
    }
    /// `t` moved onto the nearer end when it lies outside; never panics, even on NaN ends.
    pub fn clamp(&self, t: f64) -> f64 {
        t.max(self.lo.min(self.hi)).min(self.lo.max(self.hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_run_from_lo_to_hi() {
        let r = Interval::new(2.0, 6.0);
        assert_eq!(r.span(), 4.0);
        assert_eq!(r.at(0.25), 3.0);
        assert_eq!(r.mid(), 4.0);
        assert_eq!(r.samples(4).collect::<Vec<_>>(), [2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(Interval::new(6.0, 2.0).at(0.25), 5.0);
    }

    #[test]
    fn contains_and_clamp_take_the_ends_in_either_order() {
        for r in [Interval::new(1.0, 3.0), Interval::new(3.0, 1.0)] {
            assert!(r.contains(1.0) && r.contains(3.0) && !r.contains(3.5));
            assert_eq!(r.clamp(0.0), 1.0);
            assert_eq!(r.clamp(5.0), 3.0);
            assert_eq!(r.clamp(2.0), 2.0);
            assert!(r.inflated(0.5).contains(3.5) && !r.contains(3.5));
        }
        assert!(Interval::new(f64::NAN, f64::NAN).clamp(1.0).is_finite());
    }

    #[test]
    fn step_rounds_as_the_open_formula() {
        let (t0, t1) = (0.1, 0.7);
        let r = Interval::from((t0, t1));
        for k in 0..=32 {
            assert_eq!(r.step(k, 32), t0 + (t1 - t0) * k as f64 / 32.0);
        }
    }
}
