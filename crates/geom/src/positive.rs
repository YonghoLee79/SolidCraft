//! A positive length: a number above zero and finite.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

/// A finite number above zero: a radius, a distance, a tolerance. Made only by [`Positive::new`],
/// so a function taking one need not check it again. It reads as the `f64` it holds (`*r`,
/// `r.get()`).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(into = "f64", try_from = "f64")]
pub struct Positive(f64);

impl Positive {
    /// `x` when it is finite and above zero.
    pub const fn new(x: f64) -> Option<Positive> {
        if x.is_finite() && x > 0.0 { Some(Positive(x)) } else { None }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Deref for Positive {
    type Target = f64;
    fn deref(&self) -> &f64 {
        &self.0
    }
}

impl From<Positive> for f64 {
    fn from(x: Positive) -> f64 {
        x.0
    }
}

impl TryFrom<f64> for Positive {
    type Error = &'static str;
    fn try_from(x: f64) -> Result<Positive, Self::Error> {
        Positive::new(x).ok_or("a finite number above zero")
    }
}

impl fmt::Display for Positive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_finite_numbers_above_zero() {
        assert_eq!(Positive::new(2.5).map(Positive::get), Some(2.5));
        for x in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Positive::new(x), None, "{x}");
        }
        assert_eq!(serde_json::from_str::<Positive>("3.0").ok(), Positive::new(3.0));
        assert!(serde_json::from_str::<Positive>("-3.0").is_err());
    }
}
