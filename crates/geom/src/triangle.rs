//! A triangle in space and what can be asked of it.

use crate::Vec3;

/// A triangle by its corners `a`, `b`, `c`, in that order (the order gives it a side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle(pub [Vec3; 3]);

impl Triangle {
    /// `(b − a) × (c − a)`: along the side the corners turn counter-clockwise around, twice the
    /// triangle's area long.
    pub fn normal_vector(&self) -> Vec3 {
        let [a, b, c] = self.0;
        (b - a).cross(c - a)
    }

    /// The unit normal ([`Triangle::normal_vector`] made unit); `None` without area.
    pub fn normal(&self) -> Option<Vec3> {
        self.normal_vector().normalized()
    }

    pub fn area(&self) -> f64 {
        self.normal_vector().len() * 0.5
    }

    /// The mean of the corners.
    pub fn centroid(&self) -> Vec3 {
        let [a, b, c] = self.0;
        (a + b + c) / 3.0
    }

    /// The point of the triangle nearest `p` (Ericson, Real-Time Collision Detection, 5.1.5). A
    /// zero-length edge (a cone's tip repeats its apex vertex) has no interior to project onto,
    /// so its region is skipped rather than dividing 0 by 0: the point then comes from the edges
    /// that have length, and a triangle collapsed to one point gives that point.
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        closest(p, self.0)
    }
}

impl std::ops::Deref for Triangle {
    type Target = [Vec3; 3];
    fn deref(&self) -> &[Vec3; 3] {
        &self.0
    }
}

impl IntoIterator for Triangle {
    type Item = Vec3;
    type IntoIter = std::array::IntoIter<Vec3, 3>;
    fn into_iter(self) -> std::array::IntoIter<Vec3, 3> {
        self.0.into_iter()
    }
}

impl From<[Vec3; 3]> for Triangle {
    fn from(corners: [Vec3; 3]) -> Triangle {
        Triangle(corners)
    }
}

fn closest(p: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && d1 - d3 > 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && d2 - d6 > 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 && (d4 - d3) + (d5 - d6) > 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let den = va + vb + vc;
    if den.abs() < 1e-300 {
        return a;
    }
    a + ab * (vb / den) + ac * (vc / den)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_triangle_knows_its_normal_area_and_centroid() {
        let t = Triangle([Vec3::new(0., 0., 1.), Vec3::new(4., 0., 1.), Vec3::new(0., 3., 1.)]);
        assert_eq!(t.normal_vector(), Vec3::new(0., 0., 12.));
        assert_eq!(t.normal(), Some(Vec3::Z));
        assert_eq!(t.area(), 6.0);
        assert_eq!(t.centroid(), Vec3::new(4. / 3., 1., 1.));
        let flat = Triangle([Vec3::ZERO, Vec3::X, Vec3::X * 2.0]);
        assert_eq!((flat.normal(), flat.area()), (None, 0.0));
    }

    #[test]
    fn the_nearest_point_is_inside_on_an_edge_or_at_a_corner() {
        let (a, b, c) = (Vec3::new(0., 0., 0.), Vec3::new(4., 0., 0.), Vec3::new(0., 4., 0.));
        assert_eq!(Triangle([a, b, c]).closest_point(Vec3::new(1., 1., 5.)), Vec3::new(1., 1., 0.));
        assert_eq!(Triangle([a, b, c]).closest_point(Vec3::new(2., -3., 0.)), Vec3::new(2., 0., 0.));
        assert_eq!(Triangle([a, b, c]).closest_point(Vec3::new(-1., -1., 1.)), a);
    }

    /// A cone's tip repeats its apex vertex, so the triangles there have a zero-length edge: the
    /// nearest point on one must still be a real point (it used to be NaN, which broke picking
    /// the tip), and the point on the edge that has length.
    #[test]
    fn closest_point_on_a_degenerate_triangle_is_finite() {
        let (a, c) = (Vec3::new(0., 0., 0.), Vec3::new(2., 0., 0.));
        for (b, c) in [(a, c), (c, c), (a, a)] {
            for p in [Vec3::new(1., 1., 0.), Vec3::new(-1., 0.5, 0.), Vec3::new(3., -2., 1.), a] {
                let q = Triangle([a, b, c]).closest_point(p);
                assert!(q.x.is_finite() && q.y.is_finite() && q.z.is_finite(), "{a:?} {b:?} {c:?} {p:?} -> {q:?}");
            }
        }
        assert_eq!(Triangle([a, a, c]).closest_point(Vec3::new(1., 1., 0.)), Vec3::new(1., 0., 0.));
    }
}
