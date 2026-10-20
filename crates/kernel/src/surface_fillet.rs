//! Fillet between two faces that share no edge (#769): faces of separate surface bodies, or a
//! surface and a solid's face, that meet or cross. A ball of the radius rolls touching both, on
//! the side of each face the other face's pick is on; its centre traces the spine and its
//! contacts the curves where the blend meets each face. The blend runs until a contact leaves
//! its face, and is a rational B-spline through the circular sections (as `curveblend`'s).

use solidcraft_geom::{Mesh, Vec3};
use truck_modeling as mt;

use crate::body::Body;
use crate::curveblend::{Section, fit_strip, nearest_on_seam, project, section_controls};
use crate::surfaces::surface_body;
use crate::{KernelError, Result, guard};

/// Most steps of the ball along the spine, each way.
const MAX_STEPS: usize = 4096;
/// Newton iterations for one section.
const ITERATIONS: usize = 60;
/// Halvings of the last step to find where a contact leaves its face.
const BISECTIONS: usize = 80;
/// Sections per radius of spine in the fitted blend, and their bounds.
const PER_RADIUS: f64 = 2.0;
const MIN_SECTIONS: usize = 8;
const MAX_SECTIONS: usize = 512;

/// One of the two faces a fillet runs between: a face of `body`, picked by a point on it.
#[derive(Clone, Copy, Debug)]
pub struct FilletFace<'a> {
    pub body: &'a Body,
    pub pick: Vec3,
}

/// The picked face with the faces on its surface joined to it (the halves of a cylinder's
/// wall), their body's mesh for telling whether a point lies on them, and which side of them the
/// ball is on.
struct Side {
    faces: Vec<mt::Face>,
    indices: Vec<u32>,
    /// The edges round the faces, not those between two of them.
    boundary: Vec<mt::Edge>,
    mesh: Mesh,
    /// How far past the faces' boundary, along the surface, a point may lie and still be on them.
    reach: f64,
    /// +1 when the ball sits on the side the faces' normal points to, −1 on the other.
    sign: f64,
}

impl Side {
    fn new(f: FilletFace<'_>, mesh_tol: f64, reach: f64) -> Result<Side> {
        let none = || KernelError::Invalid("no face at the pick".into());
        if !f.pick.is_finite() {
            return Err(KernelError::Invalid("a fillet pick is not finite".into()));
        }
        let mesh = f.body.tessellate(mesh_tol)?;
        let picked = nearest_face(&mesh, f.pick).ok_or_else(none)?;
        let all: Vec<mt::Face> = f.body.solid.face_iter().cloned().collect();
        let group = crate::offset::with_same_surface(f.body, &[picked as usize]);
        let faces = group.iter().map(|i| all.get(*i).cloned().ok_or_else(none)).collect::<Result<Vec<_>>>()?;
        let indices = group.iter().map(|i| u32::try_from(*i).map_err(|_| none())).collect::<Result<Vec<_>>>()?;
        let edges: Vec<mt::Edge> = faces.iter().flat_map(|f| f.boundaries()).flat_map(|w| w.edge_iter().cloned().collect::<Vec<_>>()).collect();
        let boundary = edges.iter().filter(|e| edges.iter().filter(|x| x.id() == e.id()).count() == 1).cloned().collect();
        Ok(Side { faces, indices, boundary, mesh, reach, sign: 0.0 })
    }

    /// The nearest point of the faces' surface to `p` and the normal there.
    fn near(&self, p: Vec3) -> Option<(Vec3, Vec3)> {
        self.faces.iter().filter_map(|f| project(f, p, None).map(|(q, n, _)| (q, n))).min_by(|a, b| a.0.dist(p).total_cmp(&b.0.dist(p)))
    }

    /// Whether `q`, a point on the faces' surface with normal `n`, lies on the faces: how far it
    /// is from their mesh along the surface (the mesh's chords stand off a curved surface along
    /// its normal, which says nothing about the boundary).
    fn on_face(&self, q: Vec3, n: Vec3) -> bool {
        let Some(x) = closest_on_faces(&self.mesh, &self.indices, q) else { return false };
        let v = q - x;
        (v - n * v.dot(n)).len() <= self.reach
    }
}

impl Side {
    /// How far `q`, a point on the faces' surface, lies inside their boundary along the surface
    /// (negative outside), from the nearest boundary edge: a face's loops run counter-clockwise
    /// about its normal, so `n × d` along an edge points into the face.
    fn inside_by(&self, q: Vec3) -> Option<f64> {
        let (_, n) = self.near(q)?;
        let (x, d) = self.boundary.iter().filter_map(|e| nearest_on_seam(e, q)).min_by(|a, b| a.0.dist(q).total_cmp(&b.0.dist(q)))?;
        Some((q - x).dot(n.cross(d).normalized()?))
    }
}

/// The face of `mesh` nearest `p`.
fn nearest_face(mesh: &Mesh, p: Vec3) -> Option<u32> {
    mesh.triangles
        .iter()
        .zip(&mesh.tri_face)
        .filter_map(|(t, f)| Some((closest_on_triangle(mesh, t, p)?.dist(p), *f)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f)| f)
}

/// The point of the triangles of `faces` in `mesh` nearest `p`.
fn closest_on_faces(mesh: &Mesh, faces: &[u32], p: Vec3) -> Option<Vec3> {
    mesh.triangles
        .iter()
        .zip(&mesh.tri_face)
        .filter(|(_, f)| faces.contains(f))
        .filter_map(|(t, _)| closest_on_triangle(mesh, t, p))
        .min_by(|a, b| a.dist(p).total_cmp(&b.dist(p)))
}

fn closest_on_triangle(mesh: &Mesh, t: &[u32; 3], p: Vec3) -> Option<Vec3> {
    let [a, b, c] = t.map(|i| mesh.positions.get(i as usize).copied());
    Some(solidcraft_geom::compare::closest_on_triangle(p, a?, b?, c?))
}

/// The ball touching both faces with its centre in the plane through `p` square to `t`,
/// starting from `c`.
fn ball(a: &Side, b: &Side, r: f64, p: Vec3, t: Vec3, mut c: Vec3, tol: f64) -> Option<Section> {
    for _ in 0..ITERATIONS {
        let (qa, na) = a.near(c)?;
        let (qb, nb) = b.near(c)?;
        let ga = (c - qa).dot(na) - a.sign * r;
        let gb = (c - qb).dot(nb) - b.sign * r;
        let gt = (c - p).dot(t);
        let x = crate::polyhedron::solve3_pub([[na.x, na.y, na.z], [nb.x, nb.y, nb.z], [t.x, t.y, t.z]], [-ga, -gb, -gt])?;
        let dc = Vec3::new(x[0], x[1], x[2]);
        c += dc;
        if dc.len() < tol * 1e-3 && ga.abs() < tol && gb.abs() < tol && gt.abs() < tol {
            let (q1, _) = a.near(c)?;
            let (q2, _) = b.near(c)?;
            return Some(Section { c, q1, q2 });
        }
    }
    None
}

/// The spine's direction at a section: along both faces, square to both contact normals.
fn tangent(a: &Side, b: &Side, s: &Section) -> Option<Vec3> {
    let (_, na) = a.near(s.q1)?;
    let (_, nb) = b.near(s.q2)?;
    na.cross(nb).normalized()
}

/// A section whose contacts both lie on their faces.
fn on_both(a: &Side, b: &Side, s: &Section) -> bool {
    let (Some((_, na)), Some((_, nb))) = (a.near(s.q1), b.near(s.q2)) else { return false };
    a.on_face(s.q1, na) && b.on_face(s.q2, nb)
}

/// The section a step `h` from `s` along `dir`, if the ball is there and touches both faces.
fn step(a: &Side, b: &Side, r: f64, s: &Section, dir: Vec3, h: f64, tol: f64) -> Option<Section> {
    reach(a, b, r, s, dir, h, tol).filter(|n| on_both(a, b, n))
}

/// The ball a step `h` from `s` along `dir`, on the faces' surfaces wherever it lands.
fn reach(a: &Side, b: &Side, r: f64, s: &Section, dir: Vec3, h: f64, tol: f64) -> Option<Section> {
    let t = tangent(a, b, s)?;
    let t = if t.dot(dir) < 0.0 { -t } else { t };
    let p = s.c + t * h;
    ball(a, b, r, p, t, p, tol)
}

/// The sections from `start` along `dir` up to where a contact leaves its face: the last one on
/// the boundary to within `tol`.
fn run(a: &Side, b: &Side, r: f64, start: Section, dir: Vec3, h: f64, tol: f64) -> Result<Vec<Section>> {
    let mut out = Vec::new();
    let mut s = start;
    let mut dir = dir;
    for _ in 0..MAX_STEPS {
        if let Some(n) = step(a, b, r, &s, dir, h, tol) {
            dir = n.c - s.c;
            if n.c.dist(start.c) < h * 0.5 && out.len() > 2 {
                return Err(KernelError::Failed("not supported yet: a fillet that closes on itself".into()));
            }
            out.push(n);
            s = n;
            continue;
        }
        // Off a face within this step: the step whose contacts reach the faces' boundary, to
        // within `tol` along the surface.
        let inside = |n: &Section| -> Option<f64> { Some(a.inside_by(n.q1)?.min(b.inside_by(n.q2)?)) };
        let (mut lo, mut hi) = (0.0, h);
        let mut last = None;
        for _ in 0..BISECTIONS {
            let mid = 0.5 * (lo + hi);
            match reach(a, b, r, &s, dir, mid, tol).filter(|n| inside(n).is_some_and(|d| d >= 0.0)) {
                Some(n) => {
                    lo = mid;
                    last = Some(n);
                }
                None => hi = mid,
            }
            if hi - lo < tol * 1e-3 {
                break;
            }
        }
        out.extend(last);
        return Ok(out);
    }
    Err(KernelError::Failed("the fillet runs on too far: the faces may meet in a loop".into()))
}

/// Sections spaced evenly along the spine of `sections` (in order), re-solved there.
fn evenly(a: &Side, b: &Side, r: f64, sections: &[Section], tol: f64) -> Result<Vec<Section>> {
    let fail = || KernelError::Failed("the fillet's sections could not be spaced along it".into());
    let mut lengths = vec![0.0];
    for w in sections.windows(2) {
        lengths.push(lengths.last().copied().unwrap_or(0.0) + w[0].c.dist(w[1].c));
    }
    let total = lengths.last().copied().unwrap_or(0.0);
    if !(total > tol) {
        return Err(KernelError::Invalid("the fillet has no length: the faces meet only at a point".into()));
    }
    let n = ((total / r * PER_RADIUS).ceil() as usize).clamp(MIN_SECTIONS, MAX_SECTIONS);
    let (first, last) = (sections.first().ok_or_else(fail)?, sections.last().ok_or_else(fail)?);
    let mut out = vec![*first];
    for k in 1..n {
        let at = total * k as f64 / n as f64;
        let i = lengths.partition_point(|l| *l <= at).clamp(1, sections.len() - 1);
        let (s0, s1) = (sections.get(i - 1).ok_or_else(fail)?, sections.get(i).ok_or_else(fail)?);
        let span = lengths.get(i).copied().unwrap_or(total) - lengths.get(i - 1).copied().unwrap_or(0.0);
        let f = if span > 0.0 { (at - lengths.get(i - 1).copied().unwrap_or(0.0)) / span } else { 0.0 };
        let p = s0.c + (s1.c - s0.c) * f;
        let t = (s1.c - s0.c).normalized().ok_or_else(fail)?;
        out.push(ball(a, b, r, p, t, p, tol).ok_or_else(fail)?);
    }
    out.push(*last);
    Ok(out)
}

/// The rolling-ball fillet of radius `radius` between the faces `a` and `b` pick, as a sheet of
/// one face: the ball sits on the side of each face the other face's pick is on, and the
/// fillet runs both ways from where it starts, near the picks, until a contact leaves its face.
pub fn surface_fillet(a: FilletFace<'_>, b: FilletFace<'_>, radius: f64) -> Result<Body> {
    if !(radius.is_finite() && radius > 0.0) {
        return Err(KernelError::Invalid("the fillet radius must be a positive number".into()));
    }
    let size = a.body.size().max(b.body.size()).max(radius);
    let tol = (size * 1e-6).max(1e-9);
    guard("surface fillet", || {
        let mesh_tol = (size * 1e-4).min(radius * 0.05);
        let mut fa = Side::new(a, mesh_tol, tol * 10.0)?;
        let mut fb = Side::new(b, mesh_tol, tol * 10.0)?;
        let refuse = |m: &str| KernelError::Invalid(m.into());
        let (qa, na) = fa.near(a.pick).ok_or_else(|| refuse("the first pick is off its face's surface"))?;
        let (qb, nb) = fb.near(b.pick).ok_or_else(|| refuse("the second pick is off its face's surface"))?;
        fa.sign = if (b.pick - qa).dot(na) >= 0.0 { 1.0 } else { -1.0 };
        fb.sign = if (a.pick - qb).dot(nb) >= 0.0 { 1.0 } else { -1.0 };
        let t = na.cross(nb).normalized().ok_or_else(|| refuse("the faces are parallel at the picks: a ball between them touches no edge"))?;
        // Start between the picks' offsets, in the plane square to the faces' meeting there.
        let c0 = ((qa + na * (fa.sign * radius)) + (qb + nb * (fb.sign * radius))) * 0.5;
        let start = ball(&fa, &fb, radius, c0, t, c0, tol)
            .filter(|s| on_both(&fa, &fb, s))
            .ok_or_else(|| refuse("a ball of this radius can't touch both faces near the picks"))?;
        let h = radius * 0.5;
        let back = run(&fa, &fb, radius, start, -t, h, tol)?;
        let ahead = run(&fa, &fb, radius, start, t, h, tol)?;
        let mut sections: Vec<Section> = back.into_iter().rev().collect();
        sections.push(start);
        sections.extend(ahead);
        let sections = evenly(&fa, &fb, radius, &sections, tol)?;
        blend_sheet(&sections)
    })
}

/// The one-face sheet through `sections`: its contact curves and the arcs at both ends.
fn blend_sheet(sections: &[Section]) -> Result<Body> {
    let fail = |m: &str| KernelError::Failed(format!("surface fillet: {m}"));
    let fit = fit_strip(sections, &[], &[], true)?;
    let (first, last) = (sections.first().ok_or_else(|| fail("no sections"))?, sections.last().ok_or_else(|| fail("no sections"))?);
    let arc = |s: &Section| -> Result<mt::Curve> {
        let ctrl = section_controls(s, true).ok_or_else(|| fail("an end section"))?;
        let bs = mt::BSplineCurve::try_new(mt::KnotVec::bezier_knot(2), ctrl.to_vec()).map_err(|e| fail(&e.to_string()))?;
        Ok(mt::Curve::NurbsCurve(mt::NurbsCurve::new(bs)))
    };
    let v = |p: Vec3| mt::Vertex::new(crate::body::p3(p));
    let (a0, b0, a1, b1) = (v(first.q1), v(first.q2), v(last.q1), v(last.q2));
    let [c1, c2] = fit.contacts;
    let edges = vec![
        mt::Edge::new(&a0, &b0, arc(first)?),
        mt::Edge::new(&b0, &b1, c2),
        mt::Edge::new(&a1, &b1, arc(last)?).inverse(),
        mt::Edge::new(&a0, &a1, c1).inverse(),
    ];
    let face = mt::Face::try_new(vec![mt::Wire::from(edges)], fit.surface).map_err(|e| fail(&e.to_string()))?;
    Ok(surface_body(vec![face]))
}
