//! Where two sketch curves visibly cross: the shared finder behind the crossing snaps and the
//! references that resolve a crossing. It holds the Trim/Break pair finders, so a crossing here
//! is exactly a point Trim would cut at.

use std::f64::consts::TAU;

use solidcraft_geom::{Seg2, Vec2};

use crate::{Shape, Sketch, intersections};

/// Trim's tolerance for a point on a cut.
pub const TRIM_TOL: f64 = 1e-7;

/// Parameter of `p` on the bounded shape, ends included.
pub fn on_bounded(sh: &Shape, p: Vec2) -> Option<f64> {
    if sh.dist(p) > TRIM_TOL * 10.0 {
        return None;
    }
    let t = sh.param(p);
    let (t0, t1) = sh.range();
    let slack = match *sh {
        Shape::Line { a, b } => TRIM_TOL * 10.0 / a.dist(b).max(1e-12),
        Shape::Round { r, .. } => TRIM_TOL * 10.0 / r.max(1e-12),
    };
    if sh.is_full() || (t >= t0 - slack && t <= t1 + slack) { Some(t) } else { None }
}

/// Points where curve `oi` (shape `other`) meets the shape `sh` of the curve being cut, with
/// the parameter on `sh`. `ends` adds `other`'s end points touching `sh`; `sh_infinite` lets an
/// infinite `sh` (#1359) meet `other` beyond its two points.
pub fn cut_pair(sk: &Sketch, sh: &Shape, sh_infinite: bool, oi: usize, other: &Shape, ends: bool) -> Vec<(f64, Vec2)> {
    let mut pts = intersections(sh, other);
    if ends {
        // Overlapping or touching ends that the analytic intersection misses (parallel lines).
        if let Shape::Line { a, b } = *other {
            pts.extend([a, b]);
        }
        if let Shape::Round { sweep, .. } = *other
            && sweep < TAU - 1e-12
        {
            pts.extend([other.point(0.0), other.point(sweep)]);
        }
    }
    // An infinite line (#1359) cuts wherever its line crosses, beyond its two points too.
    let other_infinite = sk.curves.get(oi).is_some_and(|c| c.infinite);
    let mut out: Vec<(f64, Vec2)> = Vec::new();
    for q in pts {
        if (other_infinite || on_bounded(other, q).is_some())
            && let Some(t) = if sh_infinite { Some(sh.param(q)) } else { on_bounded(sh, q) }
            && !out.iter().any(|(_, x)| x.dist(q) < TRIM_TOL * 10.0)
        {
            out.push((t, q));
        }
    }
    out
}

/// Exact pieces of a free-form curve (polyline lines for the few kinds without them).
pub fn pieces(sk: &Sketch, ci: usize) -> Vec<Seg2> {
    sk.exact_segs(ci).unwrap_or_else(|| sk.segs(ci))
}

/// Position along the pieces (piece index + local parameter) closest to `p`, and the distance.
pub fn locate(segs: &[Seg2], p: Vec2) -> (f64, f64) {
    segs.iter()
        .enumerate()
        .map(|(i, s)| {
            let t = s.closest_param(p);
            (i as f64 + t, s.point_at(t).dist(p))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((0.0, f64::INFINITY))
}

pub fn point_at(segs: &[Seg2], u: f64) -> Vec2 {
    let n = segs.len();
    let i = (u.floor() as usize).min(n.saturating_sub(1));
    segs.get(i).map(|s| s.point_at((u - i as f64).clamp(0.0, 1.0))).unwrap_or_default()
}

/// Closest point on curve `ci` (exact pieces, or the analytic shape).
pub fn project(sk: &Sketch, ci: usize, p: Vec2) -> Vec2 {
    if sk.curves.get(ci).is_some_and(|c| c.kind.is_freeform()) {
        let segs = pieces(sk, ci);
        let (u, _) = locate(&segs, p);
        return point_at(&segs, u);
    }
    sk.shape(ci).map(|s| s.project(p)).unwrap_or(p)
}

fn seg_x(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    let (r, s) = (b - a, d - c);
    let den = r.cross(s);
    if den.abs() < 1e-15 {
        return None;
    }
    let (t, u) = ((c - a).cross(s) / den, (c - a).cross(r) / den);
    ((-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u)).then(|| a + r * t)
}

/// Where curve `oi` meets curve `ci` (polyline crossings refined onto both exact curves; `ends`
/// adds `oi`'s end points lying on `ci`): (position along `ci`, the point). `mine` is `ci`'s
/// polyline and `segs` its exact pieces. `unbounded` lets an infinite line `oi` (#1359) meet `ci`
/// beyond its two points.
pub fn cuts_pair(sk: &Sketch, ci: usize, mine: &[Vec2], segs: &[Seg2], oi: usize, ends: bool, unbounded: bool) -> Vec<(f64, Vec2)> {
    let tol = 1e-6;
    let mut other = sk.polyline(oi);
    if unbounded
        && sk.curves.get(oi).is_some_and(|c| c.infinite)
        && let Some(Shape::Line { a, b }) = sk.shape(oi)
        && let Some(d) = (b - a).normalized()
    {
        other = vec![a - d * 1e7, b + d * 1e7];
    }
    let mut found: Vec<Vec2> = Vec::new();
    for w in mine.windows(2) {
        for v in other.windows(2) {
            if let Some(x) = seg_x(w[0], w[1], v[0], v[1]) {
                found.push(x);
            }
        }
    }
    // End points of the other curve lying on this one.
    if ends && let Some((a, b)) = sk.curves.get(oi).and_then(|c| c.kind.ends()) {
        for q in [a, b].iter().filter_map(|q| sk.point(*q)) {
            if locate(segs, q).1 < 1e-6 {
                found.push(q);
            }
        }
    }
    let mut out: Vec<(f64, Vec2)> = Vec::new();
    for x in found {
        // Refine onto both exact curves by alternating projection.
        let mut q = x;
        for _ in 0..40 {
            let n = project(sk, ci, project(sk, oi, q));
            if n.dist(q) < 1e-13 {
                q = n;
                break;
            }
            q = n;
        }
        let (u, d) = locate(segs, q);
        if d < tol && project(sk, oi, q).dist(q) < tol && !out.iter().any(|(_, y)| y.dist(q) < tol * 10.0) {
            out.push((u, q));
        }
    }
    out
}

/// How near a crossing (sketch units) an existing point must be to be the point there: the
/// crossing snap reuses it rather than adding one (Trim's own tolerance for a point on a cut).
pub const AT_CROSSING: f64 = TRIM_TOL * 10.0;

/// Tangent directions closer than this (sin of 0.5°) touch rather than cross.
const PARALLEL: f64 = 0.008_726_6;

/// A visible crossing of two different curves of a sketch (indices into `sk.curves`, a < b).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    pub at: Vec2,
    pub a: usize,
    pub b: usize,
}

fn infinite(sk: &Sketch, ci: usize) -> bool {
    sk.curves.get(ci).is_some_and(|c| c.infinite)
}

/// Unit tangent of curve `ci` at `q`: exact for line/round, else the nearest polyline segment's.
fn tangent_at(sk: &Sketch, ci: usize, q: Vec2) -> Option<Vec2> {
    if let Some(sh) = sk.shape(ci) {
        return Some(sh.tangent(sh.param(q)));
    }
    let poly = sk.polyline(ci);
    let best = poly.windows(2).min_by(|x, y| seg_dist(x, q).total_cmp(&seg_dist(y, q)))?;
    (*best.get(1)? - *best.first()?).normalized()
}

fn seg_dist(w: &[Vec2], q: Vec2) -> f64 {
    match (w.first(), w.get(1)) {
        (Some(&a), Some(&b)) => {
            let s = Seg2::Line { a, b };
            s.point_at(s.closest_param(q)).dist(q)
        }
        _ => f64::INFINITY,
    }
}

/// Distance from `p` to curve `ci` as drawn (infinite lines run on forever). Exact for free-form
/// curves too: a point on one of their crossings is off the display polyline.
fn dist_to(sk: &Sketch, ci: usize, p: Vec2) -> f64 {
    match sk.shape(ci) {
        Some(sh) if infinite(sk, ci) => sh.project(p).dist(p),
        Some(sh) => sh.dist(p),
        None => pieces(sk, ci).iter().map(|s| s.point_at(s.closest_param(p)).dist(p)).fold(f64::INFINITY, f64::min),
    }
}

/// Visible, non-tangent crossings of curves `a` and `b`, including corners and T-junctions where a
/// curve end touches the other (empty when a == b or either is unknown).
pub fn crossings_of(sk: &Sketch, a: usize, b: usize) -> Vec<Vec2> {
    if a == b || a >= sk.curves.len() || b >= sk.curves.len() {
        return Vec::new();
    }
    let pts: Vec<Vec2> = match (sk.shape(a), sk.shape(b)) {
        (Some(sa), Some(sb)) => cut_pair(sk, &sa, infinite(sk, a), b, &sb, true).into_iter().map(|(_, q)| q).collect(),
        _ => {
            // Free-form pair: cut the free-form one (an infinite line runs on forever).
            let (ci, oi) = if sk.shape(a).is_none() { (a, b) } else { (b, a) };
            let segs = pieces(sk, ci);
            cuts_pair(sk, ci, &sk.polyline(ci), &segs, oi, true, true).into_iter().map(|(_, q)| q).collect()
        }
    };
    // An infinite curve accepts any end point of the other, so check analytic curves pass through it
    // (free-form hits are already refined onto both curves).
    let on = |ci: usize, q: Vec2| sk.shape(ci).is_none() || dist_to(sk, ci, q) < 1e-6;
    pts.into_iter()
        .filter(|&q| on(a, q) && on(b, q))
        .filter(|&q| match (tangent_at(sk, a, q), tangent_at(sk, b, q)) {
            (Some(ta), Some(tb)) => ta.cross(tb).abs() >= PARALLEL,
            _ => false,
        })
        .collect()
}

/// The crossing nearest `at`, within `tol` (sketch units), among the `allowed` curves passing
/// within `tol` of `at` (a drag leaves out the curves that move with it).
pub fn crossing_near(sk: &Sketch, at: Vec2, tol: f64, allowed: impl Fn(usize) -> bool) -> Option<Crossing> {
    let near: Vec<usize> = (0..sk.curves.len()).filter(|&ci| allowed(ci) && dist_to(sk, ci, at) <= tol).collect();
    let mut best: Option<(f64, Crossing)> = None;
    for (n, &a) in near.iter().enumerate() {
        for &b in near.iter().skip(n + 1) {
            for q in crossings_of(sk, a, b) {
                let d = q.dist(at);
                if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, Crossing { at: q, a, b }));
                }
            }
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::CurveKind;

    use super::*;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    fn line(sk: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> usize {
        let pa = sk.add_point(v(a.0, a.1), None).unwrap();
        let pb = sk.add_point(v(b.0, b.1), None).unwrap();
        sk.add_line_pts(pa, pb, None).unwrap()
    }

    fn spline(sk: &mut Sketch, pts: &[(f64, f64)]) -> usize {
        let pts = pts.iter().map(|p| sk.add_point(v(p.0, p.1), None).unwrap()).collect();
        sk.add_curve(CurveKind::Spline { pts, control: false, degree: 3, periodic: false }, None).unwrap()
    }

    #[test]
    fn two_lines_cross_where_they_cross() {
        let mut sk = Sketch::new();
        line(&mut sk, (0.0, 0.0), (10.0, 10.0));
        line(&mut sk, (0.0, 10.0), (10.0, 0.0));
        let c = crossing_near(&sk, v(5.1, 5.0), 0.5, |_| true).unwrap();
        assert!(c.at.dist(v(5.0, 5.0)) < 1e-9 && c.a < c.b);
    }

    #[test]
    fn extensions_do_not_cross() {
        let mut sk = Sketch::new();
        let a = line(&mut sk, (0.0, 0.0), (4.0, 4.0));
        let b = line(&mut sk, (0.0, 10.0), (10.0, 0.0));
        assert!(crossings_of(&sk, a, b).is_empty());
        assert!(crossing_near(&sk, v(5.0, 5.0), 1.0, |_| true).is_none());
    }

    #[test]
    fn infinite_line_crosses_beyond_its_points() {
        let mut sk = Sketch::new();
        let a = line(&mut sk, (0.0, 0.0), (1.0, 0.0));
        sk.curves[a].infinite = true;
        let b = line(&mut sk, (5.0, -1.0), (5.0, 1.0));
        let x = crossings_of(&sk, a, b);
        assert_eq!(x.len(), 1);
        assert!(x[0].dist(v(5.0, 0.0)) < 1e-9);
        assert!(crossing_near(&sk, v(5.0, 0.1), 0.5, |_| true).is_some());
    }

    #[test]
    fn nearest_of_two_close_crossings_wins() {
        let mut sk = Sketch::new();
        sk.add_circle(v(0.0, 0.0), 1.0, None, None).unwrap();
        line(&mut sk, (-3.0, 0.0), (3.0, 0.0));
        assert!(crossing_near(&sk, v(0.6, 0.1), 2.0, |_| true).unwrap().at.dist(v(1.0, 0.0)) < 1e-9);
        assert!(crossing_near(&sk, v(-0.6, 0.1), 2.0, |_| true).unwrap().at.dist(v(-1.0, 0.0)) < 1e-9);
    }

    #[test]
    fn arc_crossing_outside_its_sweep_is_ignored() {
        let mut sk = Sketch::new();
        sk.add_arc(v(0.0, 0.0), v(5.0, 0.0), v(0.0, 5.0), [None; 3], None).unwrap();
        // The line meets the circle at x = -4.97, outside the 0..90 degree sweep.
        line(&mut sk, (-10.0, 0.5), (0.0, 0.5));
        assert!(crossing_near(&sk, v(-4.97, 0.5), 1.0, |_| true).is_none());
        // Inside the sweep it does cross.
        line(&mut sk, (0.0, 2.0), (10.0, 2.0));
        let c = crossing_near(&sk, v(4.5, 2.0), 1.0, |_| true).unwrap();
        assert!(c.at.dist(v(21.0f64.sqrt(), 2.0)) < 1e-9);
    }

    #[test]
    fn tangent_touch_is_not_a_crossing() {
        let mut sk = Sketch::new();
        let a = sk.add_circle(v(0.0, 0.0), 5.0, None, None).unwrap();
        let b = line(&mut sk, (-10.0, 5.0), (10.0, 5.0));
        assert!(crossings_of(&sk, a, b).is_empty());
    }

    #[test]
    fn spline_crosses_a_line() {
        let mut sk = Sketch::new();
        let s = spline(&mut sk, &[(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]);
        let l = line(&mut sk, (0.0, 2.0), (10.0, 2.0));
        let x = crossings_of(&sk, s, l);
        assert_eq!(x.len(), 2);
        let segs = sk.exact_segs(s).unwrap();
        for q in x {
            assert!((q.y - 2.0).abs() < 1e-6);
            let d = segs.iter().map(|g| g.point_at(g.closest_param(q)).dist(q)).fold(f64::INFINITY, f64::min);
            assert!(d < 1e-6, "off the spline by {d}");
        }
    }

    #[test]
    fn a_point_on_a_spline_crossing_finds_it_within_the_reuse_tolerance() {
        let mut sk = Sketch::new();
        let s = spline(&mut sk, &[(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]);
        let l = line(&mut sk, (0.0, 2.0), (10.0, 2.0));
        for q in crossings_of(&sk, s, l) {
            let c = crossing_near(&sk, q, AT_CROSSING, |_| true).expect("the crossing");
            assert!(c.at.dist(q) <= AT_CROSSING);
        }
    }

    #[test]
    fn infinite_line_crosses_a_spline_beyond_its_points() {
        let mut sk = Sketch::new();
        let s = spline(&mut sk, &[(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]);
        let l = line(&mut sk, (-3.0, 2.0), (-2.0, 2.0));
        sk.curves[l].infinite = true;
        assert_eq!(crossings_of(&sk, s, l).len(), 2);
        assert!(crossing_near(&sk, v(1.5, 2.0), 1.0, |_| true).is_some());
    }

    #[test]
    fn overlap_offers_no_crossing_inside_it() {
        let mut sk = Sketch::new();
        let a = line(&mut sk, (0.0, 0.0), (10.0, 0.0));
        let b = line(&mut sk, (2.0, 0.0), (8.0, 0.0));
        assert!(crossings_of(&sk, a, b).is_empty());
        assert!(crossing_near(&sk, v(5.0, 0.0), 0.5, |_| true).is_none());
    }

    #[test]
    fn corners_and_t_junctions_count() {
        let mut sk = Sketch::new();
        let a = line(&mut sk, (0.0, 0.0), (10.0, 0.0));
        let b = line(&mut sk, (10.0, 0.0), (10.0, 10.0));
        let c = line(&mut sk, (5.0, 0.0), (5.0, 8.0));
        let corner = crossings_of(&sk, a, b);
        assert_eq!(corner.len(), 1);
        assert!(corner[0].dist(v(10.0, 0.0)) < 1e-9);
        let t = crossings_of(&sk, a, c);
        assert_eq!(t.len(), 1);
        assert!(t[0].dist(v(5.0, 0.0)) < 1e-9);
        // End to end along one line is tangent, not a crossing.
        let d = line(&mut sk, (10.0, 0.0), (20.0, 0.0));
        assert!(crossings_of(&sk, a, d).is_empty());
    }

    #[test]
    fn a_curve_does_not_cross_itself() {
        let mut sk = Sketch::new();
        let s = spline(&mut sk, &[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
        assert!(crossings_of(&sk, s, s).is_empty());
        for i in 0..=10 {
            for j in 0..=10 {
                assert!(crossing_near(&sk, v(i as f64, j as f64), 3.0, |_| true).is_none());
            }
        }
    }
}
