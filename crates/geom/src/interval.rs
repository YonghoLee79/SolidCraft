//! A range of parameters along a curve.

/// Parameters from `lo` to `hi`, held in order (`lo <= hi`) from however they were given.
/// Fractions run from `lo` (0) to `hi` (1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    lo: f64,
    hi: f64,
}

impl From<(f64, f64)> for Interval {
    fn from((a, b): (f64, f64)) -> Self {
        Interval::new(a, b)
    }
}

impl Interval {
    /// The parameters between `a` and `b`, in either order. A NaN end takes the other end's value.
    pub fn new(a: f64, b: f64) -> Self {
        Interval { lo: a.min(b), hi: a.max(b) }
    }
    pub fn lo(&self) -> f64 {
        self.lo
    }
    pub fn hi(&self) -> f64 {
        self.hi
    }
    /// `hi - lo`.
    pub fn span(&self) -> f64 {
        self.hi - self.lo
    }
    /// The parameter a fraction `f` of the way from `lo` to `hi`.
    pub fn at(&self, f: f64) -> f64 {
        self.lo + self.span() * f
    }
    /// How far `t` lies from `lo` to `hi` as a fraction (`at`'s inverse); `None` for a span of 0.
    pub fn fraction(&self, t: f64) -> Option<f64> {
        let f = (t - self.lo) / self.span();
        (self.span() > 0.0 && f.is_finite()).then_some(f)
    }
    /// The parameter `k` steps of `n` from `lo` (`at(k / n)`, rounded as `span * k / n`); `lo` when `n` is 0.
    pub fn step(&self, k: usize, n: usize) -> f64 {
        if n == 0 {
            return self.lo;
        }
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
        self.lo <= t && t <= self.hi
    }
    /// Each end moved out by `e`.
    pub fn inflated(&self, e: f64) -> Interval {
        Interval::new(self.lo - e, self.hi + e)
    }
    /// `t` moved onto the nearer end when it lies outside.
    pub fn clamp(&self, t: f64) -> f64 {
        t.max(self.lo).min(self.hi)
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
        assert_eq!(r.fraction(3.0), Some(0.25));
        assert_eq!(r.mid(), 4.0);
        assert_eq!(r.samples(4).collect::<Vec<_>>(), [2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(Interval::new(5.0, 5.0).fraction(5.0), None);
    }

    #[test]
    fn the_ends_are_held_in_order() {
        let r = Interval::new(3.0, 1.0);
        assert_eq!((r.lo(), r.hi()), (1.0, 3.0));
        assert_eq!(r, Interval::from((1.0, 3.0)));
        assert!(r.contains(1.0) && r.contains(3.0) && !r.contains(3.5));
        assert!(r.inflated(0.5).contains(3.5));
        assert_eq!((r.clamp(0.0), r.clamp(5.0), r.clamp(2.0)), (1.0, 3.0, 2.0));
        let nan = Interval::new(f64::NAN, 2.0);
        assert_eq!((nan.lo(), nan.hi()), (2.0, 2.0));
    }

    #[test]
    fn no_steps_give_lo() {
        let r = Interval::new(1.0, 3.0);
        assert_eq!(r.samples(0).collect::<Vec<_>>(), [1.0]);
        assert_eq!(r.step(1, 0), 1.0);
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
