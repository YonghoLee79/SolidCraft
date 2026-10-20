//! A straight segment between two points, in the plane or in space, and polylines made of them.

use std::ops::{Add, Mul, Sub};

use crate::{Vec2, Vec3};

/// A point a [`Segment`] runs between: a [`Vec2`] or a [`Vec3`].
pub trait Point: Copy + Add<Output = Self> + Sub<Output = Self> + Mul<f64, Output = Self> {
    fn dot(self, o: Self) -> f64;
}

impl Point for Vec2 {
    fn dot(self, o: Vec2) -> f64 {
        Vec2::dot(self, o)
    }
}

impl Point for Vec3 {
    fn dot(self, o: Vec3) -> f64 {
        Vec3::dot(self, o)
    }
}

/// The straight segment from `a` to `b`. One with no length (or not finite) answers as its
/// point `a`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment<P = Vec3> {
    pub a: P,
    pub b: P,
}

impl<P: Point> Segment<P> {
    pub fn new(a: P, b: P) -> Segment<P> {
        Segment { a, b }
    }

    pub fn len(&self) -> f64 {
        let d = self.b - self.a;
        d.dot(d).sqrt()
    }

    /// The point a fraction `t` of the way from `a` to `b`.
    pub fn lerp(&self, t: f64) -> P {
        self.a + (self.b - self.a) * t
    }

    /// The fraction of the way from `a` to `b` of the point nearest `p`.
    pub fn param(&self, p: P) -> f64 {
        let d = self.b - self.a;
        let l2 = d.dot(d);
        if l2 > 0.0 && l2.is_finite() { ((p - self.a).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The point of the segment nearest `p`.
    pub fn closest_point(&self, p: P) -> P {
        self.lerp(self.param(p))
    }

    /// How far `p` is from the segment.
    pub fn dist(&self, p: P) -> f64 {
        let d = self.closest_point(p) - p;
        d.dot(d).sqrt()
    }

    /// The nearest points of this segment and `o` (Ericson, Real-Time Collision Detection,
    /// 5.1.9).
    pub fn closest_points(&self, o: &Segment<P>) -> (P, P) {
        let (d1, d2, r) = (self.b - self.a, o.b - o.a, self.a - o.a);
        let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
        const TINY: f64 = 1e-300;
        let (s, t) = if a <= TINY && e <= TINY {
            (0.0, 0.0)
        } else if a <= TINY {
            (0.0, (f / e).clamp(0.0, 1.0))
        } else {
            let c = d1.dot(r);
            if e <= TINY {
                ((-c / a).clamp(0.0, 1.0), 0.0)
            } else {
                let b = d1.dot(d2);
                let den = a * e - b * b;
                let mut s = if den > TINY * (a * e).max(1.0) { ((b * f - c * e) / den).clamp(0.0, 1.0) } else { 0.0 };
                let mut t = (b * s + f) / e;
                if t < 0.0 {
                    t = 0.0;
                    s = (-c / a).clamp(0.0, 1.0);
                } else if t > 1.0 {
                    t = 1.0;
                    s = ((b - c) / a).clamp(0.0, 1.0);
                }
                (s, t)
            }
        };
        (self.lerp(s), o.lerp(t))
    }
}

/// Points in order, each joined to the next by a [`Segment`]. A slice is an open polyline; a
/// [`Ring`](crate::Ring) also joins its last point back to its first.
pub trait Polyline<P: Point> {
    /// Its segments, in order.
    fn segments(&self) -> impl Iterator<Item = Segment<P>> + '_;

    /// Its length.
    fn length(&self) -> f64 {
        self.segments().map(|s| s.len()).sum()
    }

    /// How far `p` is from it (infinite with no segment).
    fn distance_to(&self, p: P) -> f64 {
        self.segments().map(|s| s.dist(p)).fold(f64::INFINITY, f64::min)
    }
}

impl<P: Point> Polyline<P> for [P] {
    fn segments(&self) -> impl Iterator<Item = Segment<P>> + '_ {
        self.windows(2).filter_map(|w| Some(Segment::new(*w.first()?, *w.get(1)?)))
    }
}

#[cfg(test)]
#[path = "segment_tests.rs"]
mod tests;
