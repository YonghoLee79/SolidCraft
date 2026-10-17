//! Extrudes from a whole planar face of a body (#2040). The profile of a face is its outline
//! projected into a sketch, and an edge that is no line or arc comes in fitted: the elliptic
//! arcs where a fillet meets a slanted face are cubic splines a hair off the body's edges. A
//! join's tool then touches the body along a face it does not match, and a cut's walls lie a
//! hair inside or outside the body all round: the boolean finds pieces of no area and gives up.
//!
//! When the profile is the face's outline (to within the fit), the face itself is used: a join
//! sweeps the body's own face, which shares its edges with the body and is sewn on along it, and
//! a cut whose walls take nothing from the body (all of it within the cut's depth lies over the
//! face) is the body cut by a slab beyond the depth plane, which meets it only across that plane.

use solidcraft_geom::{Mesh, Plane, Region2, Vec2, Vec3};
use truck_modeling as mt;

use crate::body::{Body, v3};
use crate::history::{Change, History};
use crate::{KernelError, Result};

/// Most boundary samples compared per outline.
const MAX_SAMPLES: usize = 20_000;

/// A planar face of `target` and where it lies.
#[derive(Clone, Copy, Debug)]
pub struct WholeFace {
    /// The face's index in `target`.
    pub face: usize,
    /// Its outward normal.
    pub normal: Vec3,
    /// Its height along the sketch normal, from the sketch plane's origin.
    pub height: f64,
}

/// How far a fitted outline may lie from the face's edges: the projection fits a smooth chain to
/// within 1e-4 of its extent (`doc::project`), and the mesh the edges are sampled from sags.
fn outline_tol(target: &Body) -> f64 {
    (target.size() * 5e-4).max(1e-4)
}

/// The planar face of `target` at one of `heights` along `plane`'s normal (from its origin)
/// whose outline is `region`'s outer loop, to within the projection's fit, with no holes on
/// either side.
pub fn matching_face(target: &Body, plane: &Plane, region: &Region2, heights: &[f64]) -> Option<WholeFace> {
    if !region.holes.is_empty() {
        return None;
    }
    let n = plane.normal();
    let tol = outline_tol(target);
    let mesh = target.tessellate(tol * 0.2).ok()?;
    let ring: Vec<Vec2> = region.outer.polyline(tol * 0.1);
    if ring.len() < 3 || ring.len() > MAX_SAMPLES {
        return None;
    }
    let faces: Vec<&mt::Face> = target.solid.face_iter().collect();
    for (i, f) in faces.iter().enumerate() {
        let Some(m) = crate::blend::plane_normal(f) else { continue };
        if m.dot(n).abs() < 1.0 - 1e-9 || f.boundaries().len() != 1 {
            continue;
        }
        let edges = face_outline(&mesh, i);
        let Some(height) = edges.first().and_then(|e| e.first()).map(|p| (*p - plane.origin).dot(n)) else { continue };
        if !heights.iter().any(|h| (h - height).abs() <= tol) {
            continue;
        }
        // The outline in the sketch's plane, at the face's height.
        let at = |p: Vec2| plane.to_world(p) + n * height;
        let ring3: Vec<Vec3> = ring.iter().map(|p| at(*p)).collect();
        let segs: Vec<(Vec3, Vec3)> = edges.iter().flat_map(|e| e.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?)))).collect();
        if segs.is_empty() || segs.len() > MAX_SAMPLES {
            continue;
        }
        let near_edges = |p: Vec3| segs.iter().any(|(a, b)| p.dist_to_segment(*a, *b) <= tol);
        let ring_segs: Vec<(Vec3, Vec3)> = (0..ring3.len()).filter_map(|k| Some((*ring3.get(k)?, *ring3.get((k + 1) % ring3.len())?))).collect();
        let near_ring = |p: Vec3| ring_segs.iter().any(|(a, b)| p.dist_to_segment(*a, *b) <= tol);
        let off_plane = segs.iter().any(|(a, _)| ((*a - plane.origin).dot(n) - height).abs() > tol);
        if !off_plane && ring3.iter().all(|p| near_edges(*p)) && segs.iter().all(|(a, _)| near_ring(*a)) {
            return Some(WholeFace { face: i, normal: m, height });
        }
    }
    None
}

/// The boundary polylines of face `face` of `mesh`, seams left out.
fn face_outline(mesh: &Mesh, face: usize) -> Vec<Vec<Vec3>> {
    let Ok(f) = u32::try_from(face) else { return Vec::new() };
    mesh.face_edges(f).into_iter().filter(|e| !mesh.seams.get(*e).copied().unwrap_or(false)).filter_map(|e| mesh.edges.get(e).cloned()).collect()
}

/// The prism `built` (made from a profile that [`matching_face`] matched to `face`) made again
/// from the face itself: the face swept `length` along its outward normal, `outward` or back
/// into the body. Each of its faces comes from the face of `built` it lies on, so the feature
/// names its faces as it named `built`'s.
pub fn face_prism(target: &Body, face: WholeFace, length: f64, outward: bool, built: &Body) -> Result<Body> {
    if !length.is_finite() || length <= 0.0 {
        return Err(KernelError::Invalid("a prism from a face needs a positive length".into()));
    }
    let f = target.solid.face_iter().nth(face.face).cloned().ok_or_else(|| KernelError::Invalid("no such face".into()))?;
    let along = if outward { face.normal * length } else { face.normal * -length };
    let swept = crate::guard("prism from a face", || Body::new(mt::builder::tsweep(&f, v3(along))))?;
    let tol = outline_tol(target);
    let (mine, theirs) = (swept.tessellate(tol)?, built.tessellate(tol)?);
    let mut faces = Vec::with_capacity(swept.face_count());
    for i in 0..swept.face_count() {
        let probe = face_probe(&mine, i).ok_or_else(|| KernelError::Failed("prism from a face: a face with no mesh".into()))?;
        let from = nearest_face(&theirs, probe).ok_or_else(|| KernelError::Failed("prism from a face: no face under it".into()))?;
        faces.push(crate::record::from_face(0, from, Change::Modified));
    }
    let history: History = crate::record::from_faces(&swept.solid, 1, faces);
    swept.with_history_counts(history, &crate::record::counts(&[built]))
}

/// The centroid of face `face`'s largest triangle.
fn face_probe(mesh: &Mesh, face: usize) -> Option<Vec3> {
    let f = u32::try_from(face).ok()?;
    mesh.triangles
        .iter()
        .zip(&mesh.tri_face)
        .filter(|(_, tf)| **tf == f)
        .filter_map(|(t, _)| mesh.tri(t))
        .max_by(|x, y| (x[1] - x[0]).cross(x[2] - x[0]).len().total_cmp(&(y[1] - y[0]).cross(y[2] - y[0]).len()))
        .map(|[a, b, c]| (a + b + c) / 3.0)
}

/// The face of `mesh` nearest `p`.
fn nearest_face(mesh: &Mesh, p: Vec3) -> Option<usize> {
    mesh.triangles
        .iter()
        .zip(&mesh.tri_face)
        .filter_map(|(t, f)| mesh.tri(t).map(|[a, b, c]| (crate::polyhedron::point_tri(p, a, b, c), *f)))
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .and_then(|(_, f)| usize::try_from(f).ok())
}

/// Whether a cut `depth` into `target` from `face` takes only what lies over the face: nothing of
/// the body stands beyond the face's plane, and everything of it less than `depth` below lies
/// over the face (on its outline at most). Then the prism over the face and a slab over the whole
/// body take the same material, and the slab meets the body only across its depth plane.
pub fn cut_takes_only_the_face(target: &Body, face: WholeFace, depth: f64) -> bool {
    if !depth.is_finite() || depth <= 0.0 {
        return false;
    }
    let tol = outline_tol(target);
    let Ok(mesh) = target.tessellate(tol * 0.2) else { return false };
    let Some(origin) = face_outline(&mesh, face.face).first().and_then(|e| e.first().copied()) else { return false };
    let n = face.normal;
    // The face's plane with the outline in it, flattened.
    let Some(x) = (if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y }).cross(n).normalized() else { return false };
    let y = n.cross(x);
    let flat = |p: Vec3| Vec2::new((p - origin).dot(x), (p - origin).dot(y));
    let segs: Vec<(Vec2, Vec2)> = face_outline(&mesh, face.face)
        .iter()
        .flat_map(|e| e.windows(2).filter_map(|w| Some((flat(*w.first()?), flat(*w.get(1)?)))).collect::<Vec<_>>())
        .collect();
    if segs.len() < 3 || segs.len() > MAX_SAMPLES {
        return false;
    }
    let over = |q: Vec2| {
        // On the outline, or inside it (even-odd over its segments, in any order).
        let on = segs.iter().any(|(a, b)| {
            let ab = *b - *a;
            let t = if ab.len2() > 0.0 { ((q - *a).dot(ab) / ab.len2()).clamp(0.0, 1.0) } else { 0.0 };
            q.dist(*a + ab * t) <= tol
        });
        on || segs.iter().filter(|(a, b)| (a.y > q.y) != (b.y > q.y) && q.x < a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x)).count() % 2 == 1
    };
    let mut points = mesh.positions.clone();
    points.extend(mesh.triangles.iter().filter_map(|t| mesh.tri(t)).map(|[a, b, c]| (a + b + c) / 3.0));
    points.iter().all(|p| {
        let h = (*p - origin).dot(n);
        h <= tol && (h <= -depth + tol || over(flat(*p)))
    })
}

#[cfg(test)]
#[path = "whole_face_tests.rs"]
mod tests;
