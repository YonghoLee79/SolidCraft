//! An affine transform of 3D space.

use std::ops::{Index, IndexMut, Mul};

use serde::{Deserialize, Serialize};

use crate::{Plane, Vec3};

/// An affine transform: a column-major 4 × 4 matrix (`m[col][row]`), the translation in column 3.
/// Saved in designs as the bare array.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Transform(pub [[f64; 4]; 4]);

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    /// A move by `t`.
    pub fn translation(t: Vec3) -> Transform {
        Transform([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [t.x, t.y, t.z, 1.0]])
    }

    /// A turn by `angle` about the line through `origin` along `axis` (`Z` for an axis with no
    /// length).
    pub fn rotation(origin: Vec3, axis: Vec3, angle: f64) -> Transform {
        Transform::rigid(Vec3::ZERO, origin, axis, angle)
    }

    /// A rigid motion: a turn by `angle` about the line through `origin` along `axis` (`Z` for
    /// an axis with no length), then a move by `translate`.
    pub fn rigid(translate: Vec3, origin: Vec3, axis: Vec3, angle: f64) -> Transform {
        let k = axis.normalized().unwrap_or(Vec3::Z);
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        let r = [
            [t * k.x * k.x + c, t * k.x * k.y + s * k.z, t * k.x * k.z - s * k.y],
            [t * k.x * k.y - s * k.z, t * k.y * k.y + c, t * k.y * k.z + s * k.x],
            [t * k.x * k.z + s * k.y, t * k.y * k.z - s * k.x, t * k.z * k.z + c],
        ];
        let mut m = Transform::IDENTITY;
        for (mc, rc) in m.0.iter_mut().zip(r) {
            mc[..3].copy_from_slice(&rc);
        }
        let ro = m.vector(origin);
        let tr = origin - ro + translate;
        m.0[3] = [tr.x, tr.y, tr.z, 1.0];
        m
    }

    /// The reflection in `plane`.
    pub fn reflection(plane: &Plane) -> Transform {
        let n = plane.normal();
        let d = plane.origin.dot(n);
        let m = |i: usize, j: usize| {
            let (ni, nj) = ([n.x, n.y, n.z][i], [n.x, n.y, n.z][j]);
            (if i == j { 1.0 } else { 0.0 }) - 2.0 * ni * nj
        };
        // p' = p - 2 (p·n - d) n
        Transform([
            [m(0, 0), m(0, 1), m(0, 2), 0.0],
            [m(1, 0), m(1, 1), m(1, 2), 0.0],
            [m(2, 0), m(2, 1), m(2, 2), 0.0],
            [2.0 * d * n.x, 2.0 * d * n.y, 2.0 * d * n.z, 1.0],
        ])
    }

    /// Where it takes point `p`.
    pub fn point(&self, p: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z + m[3][0],
            m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z + m[3][1],
            m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z + m[3][2],
        )
    }

    /// Where it takes direction `v` (the translation left out).
    pub fn vector(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            m[0][0] * v.x + m[1][0] * v.y + m[2][0] * v.z,
            m[0][1] * v.x + m[1][1] * v.y + m[2][1] * v.z,
            m[0][2] * v.x + m[1][2] * v.y + m[2][2] * v.z,
        )
    }

    /// Its plane `p` taken along, or `p` itself where the image is no plane (a singular transform).
    pub fn plane(&self, p: &Plane) -> Plane {
        Plane::new(self.point(p.origin), self.vector(p.x), self.vector(p.y)).unwrap_or(*p)
    }

    /// The translation.
    pub fn translation_part(&self) -> Vec3 {
        Vec3::new(self.0[3][0], self.0[3][1], self.0[3][2])
    }

    /// Its sixteen entries, a column at a time.
    pub fn entries(&self) -> impl Iterator<Item = f64> + '_ {
        self.0.iter().flatten().copied()
    }

    /// Every entry finite.
    pub fn is_finite(&self) -> bool {
        self.entries().all(f64::is_finite)
    }

    /// Within 1e-12 of the identity in every entry.
    pub fn is_identity(&self) -> bool {
        self.0.iter().flatten().zip(Transform::IDENTITY.0.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-12)
    }

    /// A rigid motion: finite, its axes of length 1 and square to each other within `tolerance`,
    /// not mirrored, and an affine bottom row (within 1e-9).
    pub fn is_rigid(&self, tolerance: f64) -> bool {
        let m = &self.0;
        let column = |i: usize| Vec3::new(m[i][0], m[i][1], m[i][2]);
        let (x, y, z) = (column(0), column(1), column(2));
        m.iter().flatten().all(|v| v.is_finite())
            && [x, y, z].iter().all(|axis| (axis.dot(*axis) - 1.0).abs() < tolerance)
            && x.dot(y).abs() < tolerance
            && x.dot(z).abs() < tolerance
            && y.dot(z).abs() < tolerance
            && x.cross(y).dot(z) > 0.0
            && [m[0][3], m[1][3], m[2][3]].iter().all(|v| v.abs() < 1e-9)
            && (m[3][3] - 1.0).abs() < 1e-9
    }

    /// The inverse of an affine transform; `None` for a singular one.
    pub fn inverse(&self) -> Option<Transform> {
        let m = &self.0;
        let a = |r: usize, c: usize| m[c][r];
        let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
            + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
        if det.abs() < 1e-14 || !det.is_finite() {
            return None;
        }
        let inv = |r: usize, c: usize| -> f64 {
            // Cofactor of (c, r) / det.
            let (r0, r1) = match c {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let (c0, c1) = match r {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let minor = a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0);
            let sign = if (r + c).is_multiple_of(2) { 1.0 } else { -1.0 };
            sign * minor / det
        };
        let mut out = Transform::IDENTITY;
        for (c, col) in out.0.iter_mut().enumerate().take(3) {
            for (r, cell) in col.iter_mut().enumerate().take(3) {
                *cell = inv(r, c);
            }
        }
        let it = out.vector(self.translation_part());
        out.0[3] = [-it.x, -it.y, -it.z, 1.0];
        Some(out)
    }
}

/// `a * b` applies `b`, then `a`.
impl Mul for Transform {
    type Output = Transform;
    fn mul(self, b: Transform) -> Transform {
        let mut r = [[0.0; 4]; 4];
        for (c, col) in r.iter_mut().enumerate() {
            for (row, cell) in col.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.0[k][row] * b.0[c][k]).sum();
            }
        }
        Transform(r)
    }
}

impl Mul<&Transform> for &Transform {
    type Output = Transform;
    fn mul(self, b: &Transform) -> Transform {
        *self * *b
    }
}

impl Mul<&Transform> for Transform {
    type Output = Transform;
    fn mul(self, b: &Transform) -> Transform {
        self * *b
    }
}

impl Mul<Transform> for &Transform {
    type Output = Transform;
    fn mul(self, b: Transform) -> Transform {
        *self * b
    }
}

/// Column `i`.
impl Index<usize> for Transform {
    type Output = [f64; 4];
    fn index(&self, i: usize) -> &[f64; 4] {
        &self.0[i]
    }
}

impl IndexMut<usize> for Transform {
    fn index_mut(&mut self, i: usize) -> &mut [f64; 4] {
        &mut self.0[i]
    }
}

#[cfg(test)]
#[path = "transform_tests.rs"]
mod tests;
