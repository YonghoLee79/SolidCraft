//! Fillet between two faces that share no edge (#769): faces of separate surface bodies, or a
//! surface and a solid's face, that meet or cross. A ball of the radius rolls touching both, on
//! the side of each face the other face's pick is on; its centre traces the spine and its
//! contacts the curves where the blend meets each face. The blend runs until a contact leaves
//! its face, and is a rational B-spline through the circular sections (as `curveblend`'s).

use solidcraft_geom::{Mesh, Vec3};
use truck_modeling as mt;

use crate::body::{Body, p3};
use crate::curveblend::{Section, fit_strip, project, section_controls, solve};
use crate::distance::DistanceShape;
use crate::surfaces::surface_body;
use crate::{KernelError, Result, guard};

/// Most steps of the ball along the spine, each way.
const MAX_STEPS: usize = 4096;
/// Halvings of the last step to find where a contact leaves its face.
const BISECTIONS: usize = 60;
/// Sections per radius of spine in the fitted blend, and their bounds.
const PER_RADIUS: f64 = 2.0;
const MIN_SECTIONS: usize = 8;
const MAX_SECTIONS: usize = 512;

/// Which of the two faces the fillet trims back to where it meets them (stitching the blend
/// to what is left), or none: the blend alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilletTrim {
    #[default]
    Both,
    First,
    Second,
    None,
}

impl FilletTrim {
    pub fn is_both(&self) -> bool {
        *self == FilletTrim::Both
    }

    /// The parameter word.
    pub fn as_str(self) -> &'static str {
        match self {
            FilletTrim::Both => "both",
            FilletTrim::First => "first",
            FilletTrim::Second => "second",
            FilletTrim::None => "none",
        }
    }

    /// Whether side `k` (0: the first face, 1: the second) is trimmed.
    pub fn trims(self, k: usize) -> bool {
        matches!((self, k), (FilletTrim::Both, _) | (FilletTrim::First, 0) | (FilletTrim::Second, 1))
    }
}

/// One of the two faces a fillet runs between: a face of `body`, picked by a point on it.
#[derive(Clone, Copy, Debug)]
pub struct FilletFace<'a> {
    pub body: &'a Body,
    pub pick: Vec3,
}

/// The picked face with the faces on its surface joined to it (the halves of a cylinder's
/// wall), and which side of them the ball is on.
struct Side<'m> {
    faces: Vec<mt::Face>,
    indices: Vec<u32>,
    shape: DistanceShape<'m>,
    /// +1 when the ball sits on the side the faces' normal points to, −1 on the other.
    sign: f64,
}

impl<'m> Side<'m> {
    /// The faces `f` picks, in `mesh` (its body's, within `chord`); the ball is on the side of
    /// them the other face's pick `other` is on, where it lies over them.
    fn new(f: FilletFace<'m>, mesh: &'m Mesh, chord: f64, other: Vec3) -> Result<Side<'m>> {
        let none = || KernelError::Invalid("no face at the pick".into());
        if !(f.pick.is_finite() && other.is_finite()) {
            return Err(KernelError::Invalid("a fillet pick is not finite".into()));
        }
        let picked = nearest_face(mesh, f.pick).ok_or_else(none)?;
        let all: Vec<mt::Face> = f.body.solid.face_iter().cloned().collect();
        let group = crate::offset::with_same_surface(f.body, &[picked as usize]);
        let faces = group.iter().map(|i| all.get(*i).cloned().ok_or_else(none)).collect::<Result<Vec<_>>>()?;
        let indices = group.iter().map(|i| u32::try_from(*i).map_err(|_| none())).collect::<Result<Vec<_>>>()?;
        let mut side = Side { faces, indices, shape: DistanceShape::new(f.body, mesh, chord), sign: 1.0 };
        let (q, n) = side.near(other).ok_or_else(|| KernelError::Invalid("a pick is off its face's surface".into()))?;
        side.sign = if (other - q).dot(n) >= 0.0 { 1.0 } else { -1.0 };
        Ok(side)
    }

    /// The nearest point of the faces' surface to `p` and the normal there.
    fn near(&self, p: Vec3) -> Option<(Vec3, Vec3)> {
        self.faces.iter().filter_map(|f| project(f, p, None).map(|(q, n, _)| (q, n))).min_by(|a, b| a.0.dist(p).total_cmp(&b.0.dist(p)))
    }

    /// Whether `q`, a point on the faces' surface, lies on one of them.
    fn on_face(&self, q: Vec3) -> bool {
        self.indices.iter().any(|i| self.shape.on_face(*i, q))
    }
}

/// The face of `mesh` nearest `p`.
fn nearest_face(mesh: &Mesh, p: Vec3) -> Option<u32> {
    mesh.triangles
        .iter()
        .zip(&mesh.tri_face)
        .filter_map(|(t, f)| mesh.tri(t).map(|[a, b, c]| (solidcraft_geom::compare::closest_on_triangle(p, a, b, c).dist(p), *f)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f)| f)
}

/// A ball of radius `r` rolling between two sides, to within `tol`.
struct Roll<'m> {
    a: Side<'m>,
    b: Side<'m>,
    r: f64,
    tol: f64,
}

impl Roll<'_> {
    /// The ball touching both sides with its centre in the plane through `p` square to `t`.
    fn ball(&self, p: Vec3, t: Vec3) -> Option<Section> {
        let (fa, fb): (Vec<&mt::Face>, Vec<&mt::Face>) = (self.a.faces.iter().collect(), self.b.faces.iter().collect());
        solve(&fa, &fb, p, t, self.r, [self.a.sign, self.b.sign], 0.0, None, self.tol)
    }

    /// The spine's direction at a section, the way `dir` goes: square to both contact normals.
    fn tangent(&self, s: &Section, dir: Vec3) -> Option<Vec3> {
        let (_, na) = self.a.near(s.q1)?;
        let (_, nb) = self.b.near(s.q2)?;
        let t = na.cross(nb).normalized()?;
        Some(if t.dot(dir) < 0.0 { -t } else { t })
    }

    /// The ball a step `h` along `t` from `s`, when both its contacts lie on their faces.
    fn step(&self, s: &Section, t: Vec3, h: f64) -> Option<Section> {
        self.ball(s.c + t * h, t).filter(|n| self.a.on_face(n.q1) && self.b.on_face(n.q2))
    }

    /// The sections from `start` along `dir` up to where a contact leaves its face, the last
    /// at the faces' boundary to within `tol`.
    fn run(&self, start: Section, dir: Vec3, h: f64) -> Result<Vec<Section>> {
        let mut out = Vec::new();
        let (mut s, mut dir) = (start, dir);
        for _ in 0..MAX_STEPS {
            let t = self.tangent(&s, dir).ok_or_else(|| KernelError::Failed("the fillet's spine has no direction".into()))?;
            let Some(n) = self.step(&s, t, h) else {
                out.extend(self.to_boundary(&s, t, h));
                return Ok(out);
            };
            if n.c.dist(start.c) < h * 0.5 && out.len() > 2 {
                return Err(KernelError::Failed("not supported yet: a fillet that closes on itself".into()));
            }
            (dir, s) = (n.c - s.c, n);
            out.push(n);
        }
        Err(KernelError::Failed("the fillet runs on too far: the faces may meet in a loop".into()))
    }

    /// The farthest step within `h` along `t` from `s` whose contacts lie on their faces.
    fn to_boundary(&self, s: &Section, t: Vec3, h: f64) -> Option<Section> {
        let (mut lo, mut hi, mut last) = (0.0, h, None);
        for _ in 0..BISECTIONS {
            if hi - lo < self.tol {
                break;
            }
            let mid = 0.5 * (lo + hi);
            match self.step(s, t, mid) {
                Some(n) => (lo, last) = (mid, Some(n)),
                None => hi = mid,
            }
        }
        last
    }

    /// Sections spaced evenly along the spine of `sections` (in order), re-solved there.
    fn evenly(&self, sections: &[Section]) -> Result<Vec<Section>> {
        let fail = || KernelError::Failed("the fillet's sections could not be spaced along it".into());
        let mut lengths = vec![0.0];
        let mut total = 0.0;
        for w in sections.windows(2) {
            total += w[0].c.dist(w[1].c);
            lengths.push(total);
        }
        if !(total > self.tol) {
            return Err(KernelError::Invalid("the fillet has no length: the faces meet only at a point".into()));
        }
        let n = ((total / self.r * PER_RADIUS).ceil() as usize).clamp(MIN_SECTIONS, MAX_SECTIONS);
        let (first, last) = (sections.first().ok_or_else(fail)?, sections.last().ok_or_else(fail)?);
        let mut out = vec![*first];
        for k in 1..n {
            let at = total * k as f64 / n as f64;
            let i = lengths.partition_point(|l| *l <= at).clamp(1, sections.len() - 1);
            let (s0, s1) = (sections.get(i - 1).ok_or_else(fail)?, sections.get(i).ok_or_else(fail)?);
            let (l0, l1) = (lengths.get(i - 1).copied().ok_or_else(fail)?, lengths.get(i).copied().ok_or_else(fail)?);
            let f = if l1 > l0 { (at - l0) / (l1 - l0) } else { 0.0 };
            let t = (s1.c - s0.c).normalized().ok_or_else(fail)?;
            out.push(self.ball(s0.c + (s1.c - s0.c) * f, t).ok_or_else(fail)?);
        }
        out.push(*last);
        Ok(out)
    }
}

/// The rolling-ball fillet of radius `radius` between the faces `a` and `b` pick: the ball sits
/// on the side of each face the other face's pick is on, and the fillet runs both ways from
/// where it starts, near the picks, until a contact reaches its faces' boundary. The faces
/// `trim` names are cut back to where the blend meets them and stitched to it: the result is
/// one body of the blend and those trimmed bodies (the blend alone with [`FilletTrim::None`]).
pub fn surface_fillet(a: FilletFace<'_>, b: FilletFace<'_>, radius: f64, trim: FilletTrim) -> Result<Body> {
    if !(radius.is_finite() && radius > 0.0) {
        return Err(KernelError::Invalid("the fillet radius must be a positive number".into()));
    }
    let size = a.body.size().max(b.body.size()).max(radius);
    let tol = (size * 1e-6).max(1e-9);
    guard("surface fillet", || {
        let chord = (size * 1e-4).min(radius * 0.05);
        let (ma, mb) = (a.body.tessellate(chord)?, b.body.tessellate(chord)?);
        let roll = Roll { a: Side::new(a, &ma, chord, b.pick)?, b: Side::new(b, &mb, chord, a.pick)?, r: radius, tol };
        let refuse = |m: &str| KernelError::Invalid(m.into());
        let (qa, na) = roll.a.near(a.pick).ok_or_else(|| refuse("the first pick is off its face's surface"))?;
        let (qb, nb) = roll.b.near(b.pick).ok_or_else(|| refuse("the second pick is off its face's surface"))?;
        let t = na.cross(nb).normalized().ok_or_else(|| refuse("the faces are parallel at the picks: a ball between them touches no edge"))?;
        // Start between the picks' offsets, in the plane square to the faces' meeting there.
        let c0 = ((qa + na * (roll.a.sign * radius)) + (qb + nb * (roll.b.sign * radius))) * 0.5;
        let start = roll
            .ball(c0, t)
            .filter(|s| roll.a.on_face(s.q1) && roll.b.on_face(s.q2))
            .ok_or_else(|| refuse("a ball of this radius can't touch both faces near the picks"))?;
        let h = radius * 0.5;
        let mut sections: Vec<Section> = roll.run(start, -t, h)?.into_iter().rev().collect();
        sections.push(start);
        sections.extend(roll.run(start, t, h)?);
        let sections = roll.evenly(&sections)?;
        let blend = blend_sheet(&sections)?;
        let mut kept = Vec::new();
        for (k, f) in [a, b].into_iter().enumerate() {
            if trim.trims(k) {
                kept.push(trimmed(f, &sections, k, radius)?);
            }
        }
        if kept.is_empty() {
            return Ok(blend);
        }
        let mut all: Vec<&Body> = vec![&blend];
        all.extend(kept.iter());
        crate::surfaces::stitch(&all, tol * 100.0)
    })
}

/// The body of `f` cut back to where the blend meets it (contact `k` of each section): cut by
/// the strip from the spine through the contacts, square to the face along them, keeping the
/// side away from the other face.
fn trimmed(f: FilletFace<'_>, sections: &[Section], k: usize, r: f64) -> Result<Body> {
    use crate::splitface::SplitTool;
    use crate::surface_trim::{TrimKeep, trim_keep};
    let fail = || KernelError::Failed("surface fillet: the strip that trims a face".into());
    if !f.body.is_surface() {
        return Err(KernelError::Invalid("not supported yet: trimming a solid's face to a fillet (trim only the surface)".into()));
    }
    let contact = |s: &Section| if k == 0 { (s.q1, s.q2) } else { (s.q2, s.q1) };
    let spine: Vec<Vec3> = sections.iter().map(|s| s.c).collect();
    let past: Vec<Vec3> = sections.iter().map(|s| s.c + (contact(s).0 - s.c) * 2.0).collect();
    let tool = strip(&spine, &past).ok_or_else(fail)?;
    // Keep the side the contact faces away from the other face's contact, along the face.
    let mid = sections.get(sections.len() / 2).ok_or_else(fail)?;
    let (q, other) = contact(mid);
    let n = (mid.c - q).normalized().ok_or_else(fail)?;
    let v = q - other;
    let keep = q + (v - n * v.dot(n)).normalized().ok_or_else(fail)? * r;
    trim_keep(f.body, &SplitTool::Face { body: &tool, face: 0 }, TrimKeep::Side(keep))
}

/// A one-face sheet ruled between the polylines `a` and `b` (interpolated, as many points each).
fn strip(a: &[Vec3], b: &[Vec3]) -> Option<Body> {
    use mt::ParametricCurve;
    let (base, top) = (crate::build::interpolate_cubic(a)?, crate::build::interpolate_cubic(b)?);
    let v = |c: &mt::BSplineCurve<mt::Point3>, t: f64| mt::builder::vertex(c.subs(t));
    let (b0, b1, t0, t1) = (v(&base, 0.0), v(&base, 1.0), v(&top, 0.0), v(&top, 1.0));
    let surface = mt::Surface::BSplineSurface(mt::BSplineSurface::homotopy(base.clone(), top.clone()));
    let edges = vec![
        mt::builder::line(&b0, &t0),
        mt::Edge::new(&t0, &t1, mt::Curve::BSplineCurve(top)),
        mt::builder::line(&b1, &t1).inverse(),
        mt::Edge::new(&b0, &b1, mt::Curve::BSplineCurve(base)).inverse(),
    ];
    let face = mt::Face::try_new(vec![mt::Wire::from(edges)], surface).ok()?;
    Some(surface_body(vec![face]))
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
    let v = |p: Vec3| mt::builder::vertex(p3(p));
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
