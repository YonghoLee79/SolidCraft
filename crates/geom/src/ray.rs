//! A ray: an origin and a unit direction.

use crate::{UnitVec3, Vec3};

/// A ray from `origin` along `dir`. Its direction has length 1, so a parameter along it is a
/// distance: [`Ray::at`] is `t` mm from the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: UnitVec3,
}

impl Ray {
    pub fn new(origin: Vec3, dir: UnitVec3) -> Ray {
        Ray { origin, dir }
    }

    /// The ray from `origin` along `dir` scaled to length 1; `None` when `dir` has no length (or
    /// is not finite).
    pub fn towards(origin: Vec3, dir: Vec3) -> Option<Ray> {
        dir.unit().map(|dir| Ray { origin, dir })
    }

    /// The point `t` along it.
    pub fn at(&self, t: f64) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Its origin finite (its direction always is).
    pub fn is_finite(&self) -> bool {
        self.origin.is_finite()
    }
}

#[cfg(test)]
#[path = "ray_tests.rs"]
mod tests;
