//! Has a face room to move?
//!
//! Coincident faces are pushed apart before a boolean (see [`crate::coplanar`]). That leaves
//! the result as it was only where the space a face moves through holds nothing of the other
//! body (a cut), or nothing but its material (a join). A [`Clearance`] decides that on the
//! other body's faces: every triangle of its mesh is clipped against that space, so a wall, a
//! rib or a hole anywhere in it is found. Sample points miss what lies between them.
//!
//! A cut's or join's tool grown past the faces it lies on, to keep coincident faces out of its
//! boolean, is asked the same way (in the document's evaluation): it may gain only empty space
//! (a cut) or material (a join).

use std::collections::HashMap;

use solidcraft_geom::{Aabb3, Mesh, Vec3};
use truck_modeling as mt;

use crate::body::Body;
use crate::offset::{Surf, surf_of};

/// How a body's material sits in a region of space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occupancy {
    /// The region holds none of it.
    Empty,
    /// The region lies wholly in it.
    Full,
    /// A face reaches into the region, or may: no answer.
    Mixed,
}

/// A convex region: the points `p` with `n · p <= d` for every side `(n, d)`, `n` a unit vector
/// pointing out of it.
struct Convex(Vec<(Vec3, f64)>);

impl Convex {
    /// The prism a triangle sweeps when it moves along the unit vector `along` (not in its
    /// plane) from the distance `lo` to `hi`, its sides moved out by `margin` (in, when
    /// negative). `None` for a triangle without area or an empty interval.
    fn swept(tri: [Vec3; 3], along: Vec3, margin: f64, lo: f64, hi: f64) -> Option<Convex> {
        let [a, b, c] = tri;
        let centre = (a + b + c) / 3.0;
        if hi <= lo || !(margin + lo + hi).is_finite() || (b - a).cross(c - a).dot(along).abs() < 1e-300 {
            return None;
        }
        let mut sides = Vec::with_capacity(5);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let mut out = (q - p).cross(along).normalized()?;
            if out.dot(centre - p) > 0.0 {
                out = -out;
            }
            sides.push((out, out.dot(p) + margin));
        }
        let base = along.dot(a);
        sides.push((along, base + hi));
        sides.push((-along, -(base + lo)));
        Some(Convex(sides))
    }

    fn contains(&self, p: Vec3) -> bool {
        self.0.iter().all(|(n, d)| n.dot(p) <= *d)
    }

    /// Does the triangle reach into the region? Touching its boundary counts.
    fn meets(&self, tri: [Vec3; 3]) -> bool {
        // A side with the whole triangle beyond it settles most triangles at once.
        if self.0.iter().any(|(n, d)| tri.iter().all(|p| n.dot(*p) > *d)) {
            return false;
        }
        // What the sides leave of the triangle, one side after the other.
        let mut left: Vec<Vec3> = tri.to_vec();
        let mut next: Vec<Vec3> = Vec::with_capacity(tri.len() + self.0.len());
        for (n, d) in &self.0 {
            let Some(&last) = left.last() else { return false };
            next.clear();
            let (mut p, mut sp) = (last, n.dot(last) - d);
            for &q in &left {
                let sq = n.dot(q) - d;
                if (sp < 0.0 && sq > 0.0) || (sp > 0.0 && sq < 0.0) {
                    next.push(p + (q - p) * (sp / (sp - sq)));
                }
                if sq <= 0.0 {
                    next.push(q);
                }
                (p, sp) = (q, sq);
            }
            std::mem::swap(&mut left, &mut next);
        }
        !left.is_empty()
    }
}

/// How far a face can stand off its triangles, seen from a region along one direction.
#[derive(Clone, Copy)]
enum Reach {
    /// Not at all: a planar face, a facet of a mesh body.
    Exact,
    /// Sideways only: a wall of straight lines along the direction (a hole drilled along it).
    Sideways,
    /// By the chord error, any way.
    Around,
    /// By the chord error, to one side: a face of a cylinder, a cone or a sphere lies beyond its
    /// triangles seen from the material when it is `convex`, seen from outside when it is not.
    Beyond { convex: bool },
    /// It ends before the region begins or begins past its end (a fillet running into the
    /// plane the region starts from).
    Never,
}

/// A body's faces, for telling how its material sits in the space a face moves through.
pub struct Clearance<'a> {
    body: &'a Body,
    mesh: Mesh,
    /// How far a curved face may stand off its triangles.
    sag: f64,
    tol: f64,
    /// Whether curved faces are judged by their exact bounds (see [`Clearance::with_exact_bounds`]).
    exact: bool,
}

/// Pairs of triangles one question may compare; beyond it there is no answer.
const MAX_PAIRS: usize = 2_000_000;

impl<'a> Clearance<'a> {
    /// `chord`: the chord error of the mesh the body is asked on.
    pub fn new(body: &'a Body, chord: f64) -> Option<Clearance<'a>> {
        let mesh = body.tessellate_with(chord, true).ok()?;
        (!mesh.triangles.is_empty()).then(|| Clearance { body, mesh, sag: 2.0 * chord, tol: (body.size() * 1e-7).max(1e-9), exact: false })
    }

    /// Judge curved faces by their exact bounds where `clear_of_slab` is asked, not by the chord error each way. A B-spline or NURBS
    /// face lies in the hull of its control net, so a face whose net ends before a region begins (a loft's side running out in the
    /// plane the region starts from) can't reach into it, however its triangles stand off it. A face of a sphere or a cylinder
    /// stands off its triangles away from the centre or the axis only, so a triangle that lies clear of the region and leans away
    /// from it is clear (a dome standing on the plane the region starts from). Without this such a face counts as reaching in by
    /// the chord error.
    pub(crate) fn with_exact_bounds(mut self, on: bool) -> Self {
        self.exact = on;
        self
    }

    /// How the body's material sits in the space that `faces` move through when they go along
    /// the unit vector `along` from the distance `lo` to `hi`. `faces`: the triangles of
    /// planar faces square to `along`, each with how far its face reaches past it sideways
    /// (negative: the triangle's sides are drawn in, so that faces lying against them don't
    /// count). `Mixed` as soon as a face of the body reaches into that space.
    pub fn swept(&self, faces: &[([Vec3; 3], f64)], along: Vec3, lo: f64, hi: f64) -> Occupancy {
        let Some((first, _)) = faces.first() else { return Occupancy::Mixed };
        let base = along.dot(first[0]);
        // The body's triangles near that space.
        let mut space = Aabb3::EMPTY;
        let mut pad = self.sag;
        for (tri, margin) in faces {
            pad = pad.max(self.sag + margin);
            tri.iter().for_each(|p| [lo - self.sag, hi + self.sag].iter().for_each(|s| space.add(*p + along * *s)));
        }
        let (min, max) = (space.min - Vec3::new(pad, pad, pad), space.max + Vec3::new(pad, pad, pad));
        let outside = |tri: &[Vec3; 3]| {
            let all = |f: &dyn Fn(Vec3) -> bool| tri.iter().all(|p| f(*p));
            all(&|p| p.x < min.x)
                || all(&|p| p.x > max.x)
                || all(&|p| p.y < min.y)
                || all(&|p| p.y > max.y)
                || all(&|p| p.z < min.z)
                || all(&|p| p.z > max.z)
        };
        let mut near: Vec<(u32, [Vec3; 3])> = Vec::new();
        for (t, face) in self.mesh.triangles.iter().zip(&self.mesh.tri_face) {
            let Some(tri) = self.mesh.tri(t) else { return Occupancy::Mixed };
            if !outside(&tri) {
                near.push((*face, tri));
            }
        }
        if faces.len().saturating_mul(near.len()) > MAX_PAIRS {
            return Occupancy::Mixed;
        }
        let inside = self.mesh.inside_index();
        let mut reach: HashMap<u32, Reach> = HashMap::new();
        let mut all = None;
        for (tri, margin) in faces {
            let prism = |side: f64, end: f64| Convex::swept(*tri, along, margin + side, lo - end, hi + end);
            let probe = (tri[0] + tri[1] + tri[2]) / 3.0 + along * ((lo + hi) * 0.5);
            // A sliver thinner than its margins covers nothing that its neighbours don't.
            let Some(tight) = prism(0.0, 0.0).filter(|p| p.contains(probe)) else { continue };
            let (Some(sideways), Some(around)) = (prism(self.sag, 0.0), prism(self.sag, self.sag)) else { return Occupancy::Mixed };
            // With no face in it, the prism is all material or all empty.
            let here = if inside.contains(probe) { Occupancy::Full } else { Occupancy::Empty };
            let reaches = |(face, t): &(u32, [Vec3; 3])| {
                around.meets(*t)
                    && match *reach.entry(*face).or_insert_with(|| self.reach(*face, along, base + lo, base + hi)) {
                        Reach::Exact => tight.meets(*t),
                        Reach::Sideways => sideways.meets(*t),
                        // Its triangles lie between the prism and the face.
                        Reach::Beyond { convex } if convex == (here == Occupancy::Full) => tight.meets(*t),
                        Reach::Around | Reach::Beyond { .. } => true,
                        Reach::Never => false,
                    }
            };
            if near.iter().any(reaches) {
                return Occupancy::Mixed;
            }
            if all.is_some_and(|seen| seen != here) {
                return Occupancy::Mixed;
            }
            all = Some(here);
        }
        all.unwrap_or(Occupancy::Mixed)
    }

    /// Has the body nothing between the heights `lo` and `hi` above the plane `n · p = d` (`n`
    /// a unit vector), anywhere?
    pub(crate) fn clear_of_slab(&self, n: Vec3, d: f64, lo: f64, hi: f64) -> bool {
        let mut reach: HashMap<u32, Reach> = HashMap::new();
        let mut round: HashMap<u32, Option<Surf>> = HashMap::new();
        self.mesh.triangles.iter().zip(&self.mesh.tri_face).all(|(t, face)| {
            let Some(tri) = self.mesh.tri(t) else { return false };
            // How far the face may stand off its triangle towards larger and towards smaller heights.
            let (up, down) = match *reach.entry(*face).or_insert_with(|| self.reach(*face, n, d + lo, d + hi)) {
                Reach::Never => return true,
                Reach::Around => (self.sag, self.sag),
                Reach::Beyond { .. } => match self.exact.then(|| round.entry(*face).or_insert_with(|| self.round_of(*face))).and_then(|s| s.as_ref())
                {
                    Some(s) => self.bulge(s, (tri[0] + tri[1] + tri[2]) / 3.0, n),
                    None => (self.sag, self.sag),
                },
                Reach::Exact | Reach::Sideways => (0.0, 0.0),
            };
            let heights = tri.map(|p| n.dot(p) - d);
            heights.iter().all(|s| s + up < lo) || heights.iter().all(|s| s - down > hi)
        })
    }

    /// The round surface (sphere, cylinder, cone, torus) a face lies on, if it is one.
    fn round_of(&self, face: u32) -> Option<Surf> {
        let f = self.body.solid.face_iter().nth(face as usize)?;
        surf_of(f, self.tol * 100.0)
    }

    /// How far the surface may stand off a triangle with its centroid at `p`, towards larger and towards smaller heights along `n`: a
    /// sphere or a cylinder lies radially outward from its triangles, within the chord error.
    fn bulge(&self, s: &Surf, p: Vec3, n: Vec3) -> (f64, f64) {
        let radial = match *s {
            Surf::Sphere { c, .. } => p - c,
            Surf::Cylinder { o, a, .. } => (p - o) - a * (p - o).dot(a),
            _ => return (self.sag, self.sag),
        };
        let Some(out) = radial.normalized() else { return (self.sag, self.sag) };
        let lean = out.dot(n);
        (self.sag * lean.max(0.0), self.sag * (-lean).max(0.0))
    }

    /// How the face's triangles stand for it in a region between the heights `lo` and `hi`
    /// along `along` (measured as `along · p`).
    fn reach(&self, face: u32, along: Vec3, lo: f64, hi: f64) -> Reach {
        if self.body.is_mesh() {
            return Reach::Exact;
        }
        crate::guard("face reach", || {
            let Some(f) = self.body.solid.face_iter().nth(face as usize) else { return Ok(Reach::Around) };
            let surface = f.oriented_surface();
            if matches!(surface, mt::Surface::Plane(_)) {
                return Ok(Reach::Exact);
            }
            if self.exact
                && let Some((least, greatest)) = crate::planar_faces::control_net_span(&surface, along)
                && (greatest <= lo || least >= hi)
            {
                return Ok(Reach::Never);
            }
            let round = surf_of(f, self.tol * 100.0);
            // A flat face stored otherwise (a revolve's end: a line swept round its axis) is
            // a plane all the same: its triangles lie in it.
            if matches!(round, Some(Surf::Plane { .. })) {
                return Ok(Reach::Exact);
            }
            let straight = matches!(round, Some(Surf::Cylinder { a, .. }) if a.dot(along).abs() > 1.0 - 1e-12)
                || crate::prism::wall_along(&surface, along, self.tol * 10.0, &mut 262_144);
            Ok(if straight {
                Reach::Sideways
            } else if round.and_then(|s| span(&s, along)).is_some_and(|(s0, s1)| s1 <= lo || s0 >= hi)
                || round.and_then(|s| face_span(f, &s, along)).is_some_and(|(s0, s1)| s1 <= lo || s0 >= hi)
            {
                Reach::Never
            } else if let Some(Surf::Cylinder { convex, .. } | Surf::Cone { convex, .. } | Surf::Sphere { convex, .. }) = round {
                Reach::Beyond { convex }
            } else {
                Reach::Around
            })
        })
        .unwrap_or(Reach::Around)
    }
}

/// The whole extent of a round surface along `along`, where it has one.
fn span(s: &Surf, along: Vec3) -> Option<(f64, f64)> {
    let about = |c: Vec3, r: f64| Some((along.dot(c) - r.abs(), along.dot(c) + r.abs()));
    match *s {
        Surf::Cylinder { o, a, r, .. } if a.dot(along).abs() < 1e-9 => about(o, r),
        Surf::Sphere { c, r, .. } => about(c, r),
        Surf::Torus { o, a, h, r, .. } if a.dot(along).abs() > 1.0 - 1e-12 => about(o + a * h, r),
        _ => None,
    }
}

/// The extent along `along` of a face of a surface of revolution (a cylinder, a cone or a torus), from the angles round its
/// axis that it spans: the face of a revolve that ends in a plane through its axis stands on one side of that plane, though
/// its surface goes round to the other, and its triangles may meet the plane where the face does (a revolve ended flush with
/// a face it joins, #3156). Along a circle of the surface the height is a sine of the angle, so over an arc that leaves
/// out the circle's highest point it is greatest at an end of the arc; the arcs the face spans are found from its
/// boundary, where its least and greatest angles lie (the angle round the axis has no extremes inside a face). `None` for
/// other faces, or a face that goes nearly all the way round.
fn face_span(f: &mt::Face, s: &Surf, along: Vec3) -> Option<(f64, f64)> {
    use mt::{BoundedCurve, ParametricCurve};
    const STEPS: u32 = 64;
    let along = along.normalized()?;
    // A point of the axis (a cone's apex) and its direction.
    let (o, a) = match *s {
        Surf::Cylinder { o, a, .. } | Surf::Torus { o, a, .. } => (o, a),
        Surf::Cone { v, a, .. } => (v, a),
        _ => return None,
    };
    let mut points: Vec<Vec3> = Vec::new();
    for e in f.edge_iter() {
        let c = e.curve();
        let (t0, t1) = c.range_tuple();
        points.extend((0..=STEPS).map(|i| crate::body::from_p3(c.subs(t0 + (t1 - t0) * f64::from(i) / f64::from(STEPS)))));
    }
    if points.is_empty() || points.iter().any(|p| !p.is_finite()) {
        return None;
    }
    // The heights along the axis the face lies within, and its least and greatest distance from the axis there.
    let heights = points.iter().map(|p| a.dot(*p - o));
    let (t0, t1) = heights.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| (lo.min(t), hi.max(t)));
    let (rho0, rho1, t0, t1) = match *s {
        Surf::Cylinder { r, .. } => (r, r, t0, t1),
        // The apex is at t = 0, the cone opening along a; the height along the axis has no extremes inside a face.
        Surf::Cone { half, .. } => {
            let k = half.tan();
            if !(k.is_finite() && k > 0.0 && t0 >= 0.0) {
                return None;
            }
            (t0 * k, t1 * k, t0, t1)
        }
        // A tube may reach higher or lower along the axis inside the face than on its boundary.
        Surf::Torus { big, h, r, .. } => {
            if big - r <= 0.0 {
                return None;
            }
            (big - r, big + r, h - r, h + r)
        }
        _ => return None,
    };
    // Heights along `along`: n·o + (n·a) t + rho |n⊥| cos θ, θ measured from n⊥ round the axis.
    let up = along.dot(a);
    let across = along - a * up;
    let m = across.len();
    let base = along.dot(o);
    let (low, high) = (base + (up * t0).min(up * t1), base + (up * t0).max(up * t1));
    if m < 1e-12 {
        return Some((low, high));
    }
    let e1 = across * (1.0 / m);
    let e2 = a.cross(e1);
    let mut angles: Vec<f64> = points.iter().map(|p| (*p - o).dot(e2).atan2((*p - o).dot(e1))).collect();
    angles.sort_by(f64::total_cmp);
    // The face spans the circle less the widest gap between the angles of its boundary.
    let tau = std::f64::consts::TAU;
    let (mut gap, mut from) = (angles.first()? + tau - angles.last()?, *angles.last()?);
    for w in angles.windows(2) {
        if let [x, y] = w
            && y - x > gap
        {
            (gap, from) = (y - x, *x);
        }
    }
    if gap < 0.2 {
        return None;
    }
    // The span starts at the gap's end and runs `tau - gap` round from there.
    let start = from + gap;
    let width = tau - gap;
    let holds = |angle: f64| (angle - start).rem_euclid(tau) <= width;
    let ends = [start.cos(), (start + width).cos()];
    let cos_max = if holds(0.0) { 1.0 } else { ends[0].max(ends[1]) };
    let cos_min = if holds(std::f64::consts::PI) { -1.0 } else { ends[0].min(ends[1]) };
    let most = if cos_max >= 0.0 { rho1 * m * cos_max } else { rho0 * m * cos_max };
    let least = if cos_min <= 0.0 { rho1 * m * cos_min } else { rho0 * m * cos_min };
    Some((low + least, high + most))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoolOp, boolean, box_solid, cylinder, fillet};

    fn clearance(body: &Body) -> Clearance<'_> {
        Clearance::new(body, body.size() * 2e-4).unwrap()
    }

    /// The top face of a 200 x 100 x 20 block between x = `x0` and `x1`, y = 10 and 90, as two
    /// triangles with their sides drawn in by 1e-4.
    fn top(x0: f64, x1: f64) -> Vec<([Vec3; 3], f64)> {
        let p = |x: f64, y: f64| Vec3::new(x, y, 20.0);
        vec![([p(x0, 10.0), p(x1, 10.0), p(x1, 90.0)], -1e-4), ([p(x0, 10.0), p(x1, 90.0), p(x0, 90.0)], -1e-4)]
    }

    /// The space over a block's top face is empty and the space under it is material, up to
    /// the smallest thing that stands in it, wherever that is.
    #[test]
    fn the_space_a_face_moves_through_is_empty_or_material_until_something_stands_in_it() {
        let block = box_solid(Vec3::ZERO, Vec3::new(200.0, 100.0, 20.0)).unwrap();
        let c = clearance(&block);
        assert_eq!(c.swept(&top(10.0, 190.0), Vec3::Z, 1e-4, 3.0), Occupancy::Empty);
        assert_eq!(c.swept(&top(10.0, 190.0), -Vec3::Z, 1e-4, 3.0), Occupancy::Full);
        // 25 down is through the bottom face (the block is 20 thick); 210 is past its side x = 200.
        assert_eq!(c.swept(&top(10.0, 190.0), -Vec3::Z, 1e-4, 25.0), Occupancy::Mixed);
        assert_eq!(c.swept(&top(10.0, 210.0), -Vec3::Z, 1e-4, 3.0), Occupancy::Mixed);
        assert_eq!(c.swept(&[], Vec3::Z, 1e-4, 3.0), Occupancy::Mixed);
        assert!(c.clear_of_slab(Vec3::Z, 20.0, 1e-4, 3.0) && !c.clear_of_slab(Vec3::Z, 20.0, -3.0, -1e-4));
        // A pin 0.4 x 0.4 on the top face, at x = 137.3 to 137.7: in the way of what covers it only.
        let pin = box_solid(Vec3::new(137.3, 61.1, 20.0), Vec3::new(137.7, 61.5, 26.0)).unwrap();
        let pinned = boolean(&block, &pin, BoolOp::Union).unwrap().unwrap();
        let c = clearance(&pinned);
        assert_eq!(c.swept(&top(10.0, 190.0), Vec3::Z, 1e-4, 3.0), Occupancy::Mixed);
        assert_eq!(c.swept(&top(10.0, 137.0), Vec3::Z, 1e-4, 3.0), Occupancy::Empty);
        assert!(!c.clear_of_slab(Vec3::Z, 20.0, 1e-4, 3.0) && c.clear_of_slab(Vec3::Z, 20.0, 6.0 + 1e-4, 9.0));
        // A 4 mm hole through the block at x = 60: no material all the way under what covers it.
        let hole = cylinder(Vec3::new(60.0, 50.0, -5.0), Vec3::Z, 2.0, 30.0).unwrap();
        let holed = boolean(&block, &hole, BoolOp::Cut).unwrap().unwrap();
        let c = clearance(&holed);
        assert_eq!(c.swept(&top(10.0, 190.0), -Vec3::Z, 1e-4, 3.0), Occupancy::Mixed);
        assert_eq!(c.swept(&top(70.0, 190.0), -Vec3::Z, 1e-4, 3.0), Occupancy::Full);
    }

    /// Round faces that end in the plane a face lies in leave the space beyond it free: a
    /// hole's wall, a fillet running into the top face. A round face that rises into that space
    /// by less than its chords show does not.
    #[test]
    fn round_faces_ending_at_the_plane_leave_the_space_beyond_it_free() {
        let block = box_solid(Vec3::ZERO, Vec3::new(200.0, 100.0, 20.0)).unwrap();
        let hole = cylinder(Vec3::new(60.0, 50.0, -5.0), Vec3::Z, 8.0, 30.0).unwrap();
        let holed = boolean(&block, &hole, BoolOp::Cut).unwrap().unwrap();
        // The fillet (r = 4) along the edge y = 0 of the top face ends on the line y = 4, z = 20.
        let rounded = fillet(&holed, &[Vec3::new(100.0, 0.0, 20.0)], 4.0).unwrap();
        let c = clearance(&rounded);
        // The top face from x = `x0` to 190 and y = `y0` to 90.
        let rect = |x0: f64, y0: f64| {
            let p = |x: f64, y: f64| Vec3::new(x, y, 20.0);
            vec![([p(x0, y0), p(190.0, y0), p(190.0, 90.0)], -1e-4), ([p(x0, y0), p(190.0, 90.0), p(x0, 90.0)], -1e-4)]
        };
        assert_eq!(c.swept(&rect(10.0, -10.0), Vec3::Z, 1e-4, 3.0), Occupancy::Empty);
        assert!(c.clear_of_slab(Vec3::Z, 20.0, 1e-4, 3.0));
        // Under the top face the fillet is in the way from its tangent line y = 4 down (and no
        // further: its chords lie inside it), the hole from x = 68 down.
        assert_eq!(c.swept(&rect(80.0, 4.0), -Vec3::Z, 1e-4, 3.0), Occupancy::Full);
        assert_eq!(c.swept(&rect(80.0, 3.9), -Vec3::Z, 1e-4, 3.0), Occupancy::Mixed);
        assert_eq!(c.swept(&rect(60.0, 5.0), -Vec3::Z, 1e-4, 3.0), Occupancy::Mixed);
        // A bar lying along x (axis at z = 12): its top ends 0.05 um short of where the space
        // begins (z = 20.0001), then reaches 0.9 um into it.
        for (radius, want) in [(8.00005, Occupancy::Empty), (8.001, Occupancy::Mixed)] {
            let bar = cylinder(Vec3::new(20.0, 50.0, 12.0), Vec3::X, radius, 160.0).unwrap();
            assert_eq!(clearance(&bar).swept(&top(30.0, 150.0), Vec3::Z, 1e-4, 3.0), want, "radius {radius}");
        }
    }
}
