//! The last resort for a face the planner cannot plan however finely its edges are divided: a
//! leftover of the file's own modelling, such as a cusp where two edges leave a point at a zero
//! angle, a sliver whose two sides sag across each other, or a small cap round a pole. Its
//! boundary is filled flat, so the rest of the body keeps its plan rather than the whole body
//! falling back to the reference mesh.
//!
//! The boundary nodes (the shared edge nodes, so the face still meets its neighbours exactly)
//! are laid onto the plane that best fits them and triangulated there: every boundary side is
//! kept as a constraint unless it crosses one already kept (the defect itself), and a triangle
//! is in the face when it lies inside an odd number of loops. The triangles turn the way the
//! surface does, and each node keeps its parameters on the surface for its normal.

use std::collections::HashMap;

use solidcraft_geom::Vec3;
use solidcraft_geom::tess::{EvalSurface, FacePlan};
use spade::handles::FixedVertexHandle;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use super::uv;

/// Most boundary nodes a face is filled at.
const MAX_NODES: usize = 50_000;

/// The face `fi` of surface `s`, bounded by the loops `loops` (in space), filled flat; `None`
/// when its nodes do not lie on the surface within `accept`, span no plane, enclose nothing, or
/// the flat fill strays from the surface by more than the chord tolerance `tol` (a real curved
/// face, which the body's reference mesh draws better).
pub(crate) fn fill(fi: usize, s: &EvalSurface, loops: &[Vec<Vec3>], accept: f64, tol: f64) -> Option<FacePlan> {
    let fixed: Vec<Vec3> = loops.iter().flatten().copied().collect();
    if fixed.len() < 3 || fixed.len() > MAX_NODES {
        return None;
    }
    // Each node's parameters, for its normal (a projection from the previous node's).
    let dom = uv::domain(s);
    let mut params: Vec<[f64; 2]> = Vec::with_capacity(fixed.len());
    for l in loops {
        let mut prev = None;
        for p in l {
            let q = uv::project(s, &dom, *p, prev, accept)?;
            params.push(q);
            prev = Some(q);
        }
    }
    // The plane: Newell's normal of the loops (holes count against the outer loop) through
    // their centroid.
    let mut normal = Vec3::ZERO;
    for l in loops {
        let n = l.len();
        for k in 0..n {
            let (a, b) = (l.get(k)?, l.get((k + 1) % n)?);
            normal += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
    }
    let length = normal.len();
    if !(length > 0.0 && length.is_finite()) {
        return None;
    }
    let normal = normal * (1.0 / length);
    let centre = fixed.iter().fold(Vec3::ZERO, |c, p| c + *p) * (1.0 / fixed.len() as f64);
    // Any direction in the plane: the normal crossed with the axis it is least along.
    let axis = if normal.x.abs() <= normal.y.abs() && normal.x.abs() <= normal.z.abs() {
        Vec3::X
    } else if normal.y.abs() <= normal.z.abs() {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let e1 = normal.cross(axis);
    let e1 = e1 * (1.0 / e1.len());
    let e2 = normal.cross(e1);
    let flat: Vec<[f64; 2]> = fixed.iter().map(|p| [(*p - centre).dot(e1), (*p - centre).dot(e2)]).collect();
    if flat.iter().flatten().any(|x| !x.is_finite()) {
        return None;
    }
    let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
    let mut index: HashMap<FixedVertexHandle, u32> = HashMap::new();
    let mut handles = Vec::with_capacity(flat.len());
    for (i, q) in flat.iter().enumerate() {
        let h = cdt.insert(spade::mitigate_underflow(Point2::new(q[0], q[1]))).ok()?;
        index.entry(h).or_insert(u32::try_from(i).ok()?);
        handles.push(h);
    }
    let mut start = 0usize;
    let mut ranges = Vec::with_capacity(loops.len());
    for l in loops {
        let n = l.len();
        for k in 0..n {
            let (a, b) = (*handles.get(start + k)?, *handles.get(start + (k + 1) % n)?);
            if a != b && !cdt.exists_constraint(a, b) && cdt.can_add_constraint(a, b) {
                cdt.add_constraint(a, b);
            }
        }
        ranges.push(start..start + n);
        start += n;
    }
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    for face in cdt.inner_faces() {
        let vs = face.vertices();
        let pos = vs.map(|v| v.position());
        let mid = [(pos[0].x + pos[1].x + pos[2].x) / 3.0, (pos[0].y + pos[1].y + pos[2].y) / 3.0];
        if inside(mid, &flat, &ranges) {
            let t = [index.get(&vs[0].fix()), index.get(&vs[1].fix()), index.get(&vs[2].fix())];
            if let [Some(a), Some(b), Some(c)] = t {
                triangles.push([*a, *b, *c]);
            }
        }
    }
    if triangles.is_empty() {
        return None;
    }
    // Flat only where flat is close enough: each triangle's middle within the chord tolerance of
    // the surface (found from its first corner's parameters).
    let off = |t: &[u32; 3]| -> Option<f64> {
        let [a, b, c] = t.map(|i| fixed.get(i as usize).copied());
        let mid = (a? + b? + c?) * (1.0 / 3.0);
        let q = uv::project(s, &dom, mid, params.get(t[0] as usize).copied(), f64::INFINITY)?;
        Some(s.derivs(q[0], q[1])?.0.dist(mid))
    };
    if !triangles.iter().all(|t| off(t).is_some_and(|d| d <= tol)) {
        return None;
    }
    // Counter-clockwise about `normal`; turned over where the surface faces the other way.
    let along: f64 = params.iter().filter_map(|q| s.derivs(q[0], q[1])).map(|(_, su, sv)| su.cross(sv).dot(normal)).sum();
    if along < 0.0 {
        for t in &mut triangles {
            t.swap(1, 2);
        }
    }
    Some(FacePlan { face: u32::try_from(fi).unwrap_or(u32::MAX), surface: None, uv: params, fixed, normals: Vec::new(), flip: false, triangles })
}

/// Whether `p` lies inside an odd number of the loops `ranges` of `pts`.
fn inside(p: [f64; 2], pts: &[[f64; 2]], ranges: &[std::ops::Range<usize>]) -> bool {
    let mut odd = false;
    for r in ranges {
        let n = r.len();
        for k in 0..n {
            let (Some(a), Some(b)) = (pts.get(r.start + k), pts.get(r.start + (k + 1) % n)) else { continue };
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                odd = !odd;
            }
        }
    }
    odd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane face whose right side folds back over itself at a zero angle (a cusp): no side
    /// crossing another may be kept, yet the face is filled and turns the plane's way.
    #[test]
    fn a_cusp_is_filled_the_surfaces_way() {
        let s = EvalSurface::Plane { origin: Vec3::ZERO, du: Vec3::X, dv: Vec3::Y };
        let corners = [[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [3.0, 4.2], [3.8, 3.6], [0.0, 4.0]];
        let l: Vec<Vec3> = corners.iter().map(|q| Vec3::new(q[0], q[1], 0.0)).collect();
        let Some(f) = fill(0, &s, &[l], 1e-9, 1e-9) else { panic!("filled") };
        assert!(!f.triangles.is_empty());
        let area: f64 = f
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| f.fixed[i as usize]);
                (b - a).cross(c - a).z * 0.5
            })
            .sum();
        assert!(area > 14.0 && area < 16.5, "area {area}");
        assert_eq!(f.uv.len(), f.fixed.len());
    }

    #[test]
    fn a_boundary_enclosing_nothing_is_not_filled() {
        let s = EvalSurface::Plane { origin: Vec3::ZERO, du: Vec3::X, dv: Vec3::Y };
        let l = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
        assert!(fill(0, &s, &[l], 1e-3, 1e-3).is_none());
    }
}
