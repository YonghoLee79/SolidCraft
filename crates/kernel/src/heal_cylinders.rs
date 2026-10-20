//! Neighbouring faces side by side on one cylinder become one face (#2844): a prism boolean's
//! slabs split every wall at every vertex any other section has on it, so a ring joined onto a
//! perforated plate came out as full-height strips, one seam wherever its foot crossed a hole
//! edge. Only for prism booleans' results, whose walls are exact cylinders.

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::{PI, TAU};

use solidcraft_geom::Vec3;
use truck_meshalgo::tessellation::RobustMeshableShape;
use truck_modeling::{self as mt, builder};

use super::{chain_loops, edge_points, find, on_split_line};
use crate::body::{from_p3, p3, v3};

/// A merged face spans at most this much of a turn (and a half wall exactly that): sketch
/// circles reach the kernel as two half arcs, and their walls stay two faces.
const MAX_SWEEP: f64 = PI + 1e-7;

/// A cylinder split into more strips than this keeps them: finding the runs takes n² lengths and
/// n² log n time for n strips, and a dense hole pattern under a ring can make thousands.
const MAX_STRIPS: usize = 256;

/// The cylinder a face lies on: a point on its axis, the axis (the same way for every face on
/// the cylinder), the radius, and whether the face's outward normal points away from the axis.
#[derive(Clone, Copy)]
struct Cyl {
    o: Vec3,
    a: Vec3,
    r: f64,
    convex: bool,
}

impl Cyl {
    fn of(f: &mt::Face, tol: f64) -> Option<Cyl> {
        if matches!(f.surface(), mt::Surface::Plane(_)) {
            return None;
        }
        match crate::offset::surf_of(f, tol)? {
            crate::offset::Surf::Cylinder { o, a, r, convex } => {
                let a = if a.x + a.y * 1e-3 + a.z * 1e-6 < 0.0 { -a } else { a };
                (r.is_finite() && r > tol).then_some(Cyl { o, a, r, convex })
            }
            _ => None,
        }
    }

    fn same(&self, o: &Cyl, tol: f64) -> bool {
        let off = o.o - self.o;
        self.convex == o.convex && self.a.dot(o.a) > 1.0 - 1e-9 && (self.r - o.r).abs() < tol && (off - self.a * off.dot(self.a)).len() < tol
    }
}

/// Where a face lies about its cylinder: the turn its boundary spans (`lo..hi`, about a frame
/// fixed for its group) and the height along the axis.
#[derive(Clone, Copy)]
struct Span {
    lo: f64,
    hi: f64,
    z0: f64,
    z1: f64,
}

fn span_of(f: &mt::Face, c: &Cyl, u: Vec3, w: Vec3) -> Option<Span> {
    let pts: Vec<Vec3> = f.edge_iter().flat_map(|e| edge_points(&e)).collect();
    let angle = |p: Vec3| (p - c.o).dot(w).atan2((p - c.o).dot(u));
    // About the mean way out from the axis, which lies inside any span short of a whole turn.
    let mean = pts.iter().fold(Vec3::ZERO, |s, p| {
        let d = *p - c.o;
        s + (d - c.a * d.dot(c.a)).normalized().unwrap_or(Vec3::ZERO)
    });
    if mean.len() < 1e-9 {
        return None;
    }
    let mid = angle(c.o + mean);
    let (mut lo, mut hi, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &pts {
        let t = mid + (angle(*p) - mid + PI).rem_euclid(TAU) - PI;
        let z = (*p - c.o).dot(c.a);
        (lo, hi, z0, z1) = (lo.min(t), hi.max(t), z0.min(z), z1.max(z));
    }
    (lo.is_finite() && hi - lo <= MAX_SWEEP && z1 > z0).then_some(Span { lo, hi, z0, z1 })
}

/// Total area of faces, meshed on their own (None when one does not mesh).
fn meshed_area(faces: Vec<mt::Face>, tol: f64) -> Option<f64> {
    let shell: mt::Shell = faces.into();
    let meshed = shell.robust_triangulation(tol);
    let mut area = 0.0;
    for f in meshed.face_iter() {
        let m = f.surface()?;
        let mut any = false;
        for t in m.faces().triangle_iter() {
            let q = |k: usize| t.get(k).and_then(|v| m.positions().get(v.pos)).map(|p| from_p3(*p));
            let (Some(a), Some(b), Some(c)) = (q(0), q(1), q(2)) else { return None };
            area += (b - a).cross(c - a).len() * 0.5;
            any = true;
        }
        if !any {
            return None;
        }
    }
    Some(area)
}

/// One face on `c` over the faces `members` (edge-connected, spanning `t0..t1` and `z0..z1` about
/// the frame `u`, `w`): their outer boundary on one exact cylinder. None when it can't be built,
/// or it doesn't mesh to the area they cover.
fn merged_face(members: &[&mt::Face], c: &Cyl, (u, w): (Vec3, Vec3), (t0, t1, z0, z1): (f64, f64, f64, f64), tol: f64) -> Option<mt::Face> {
    use mt::{Invertible, ParametricSurface3D, SearchParameter};
    let mut count: HashMap<mt::EdgeID, usize> = HashMap::new();
    for m in members {
        let mut own: HashMap<mt::EdgeID, usize> = HashMap::new();
        for e in m.boundary_iters().into_iter().flatten() {
            *own.entry(e.id()).or_insert(0) += 1;
            *count.entry(e.id()).or_insert(0) += 1;
        }
        // A seam (an edge a face runs along both ways) only a whole turn has.
        if own.values().any(|n| *n > 1) {
            return None;
        }
    }
    let rest: Vec<mt::Edge> =
        members.iter().flat_map(|m| m.boundary_iters().into_iter().flatten()).filter(|e| count.get(&e.id()).copied() == Some(1)).collect();
    let wires: Vec<mt::Wire> = chain_loops(rest)?.into_iter().map(mt::Wire::from).collect();
    let at = |t: f64, z: f64| c.o + (u * t.cos() + w * t.sin()) * c.r + c.a * z;
    let (va, vb) = (builder::vertex(p3(at(t0, z0))), builder::vertex(p3(at(t1, z0))));
    let wall: mt::Face = builder::tsweep(&builder::circle_arc(&va, &vb, p3(at((t0 + t1) * 0.5, z0))), v3(c.a * (z1 - z0)));
    let mut surface = wall.oriented_surface();
    // Outward as the faces were.
    let probe = at((t0 + t1) * 0.5, (z0 + z1) * 0.5);
    let (s, t) = surface.search_parameter(p3(probe), None, 100)?;
    let n = surface.normal(s, t);
    let radial = (probe - c.o - c.a * (probe - c.o).dot(c.a)).normalized()?;
    if (Vec3::new(n.x, n.y, n.z).dot(radial) < 0.0) == c.convex {
        surface = surface.inverse();
    }
    let face = mt::Face::try_new(wires, surface).ok()?;
    let mesh_tol = (c.r * 1e-3).max(tol);
    let (whole, parts) = (meshed_area(vec![face.clone()], mesh_tol)?, meshed_area(members.iter().map(|f| (*f).clone()).collect(), mesh_tol)?);
    ((whole - parts).abs() <= parts * 1e-3).then_some(face)
}

/// Merge each group of edge-connected faces side by side on one cylinder, over the same height
/// (keeping faces apart along the lines in `keep`), into as few faces as there are runs of at most a half turn. Returns the new
/// shell and, for each of its faces, the faces of `shell` it was made from (by position); None
/// when nothing merged. A group whose merged face can't be built, or doesn't mesh to the area
/// its pieces cover, keeps its pieces.
pub(super) fn merge_cocylindrical(shell: &mt::Shell, tol: f64, keep: &[Vec<Vec3>]) -> Option<(mt::Shell, Vec<Vec<usize>>)> {
    let faces: Vec<mt::Face> = shell.face_iter().cloned().collect();
    let cyls: Vec<Option<Cyl>> = faces.iter().map(|f| Cyl::of(f, tol)).collect();
    if cyls.iter().flatten().count() < 2 {
        return None;
    }
    let mut by_edge: HashMap<mt::EdgeID, Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        if cyls.get(i).is_some_and(Option::is_some) {
            for e in f.edge_iter() {
                by_edge.entry(e.id()).or_default().push(i);
            }
        }
    }
    let edges: HashMap<mt::EdgeID, mt::Edge> = shell.edge_iter().map(|e| (e.id(), e)).collect();
    // How far each face reaches along its axis (from its own axis point).
    let heights: Vec<Option<(f64, f64)>> = faces
        .iter()
        .zip(&cyls)
        .map(|(f, c)| {
            let c = c.as_ref()?;
            let zs = f.edge_iter().flat_map(|e| edge_points(&e)).map(|p| (p - c.o).dot(c.a));
            Some(zs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| (lo.min(z), hi.max(z))))
        })
        .collect();
    // The pairs of faces that may merge across an edge: side by side over the same height, so
    // the merged face is the band they make, as the prism boolean would have built it. Pieces
    // stacked up the axis stay apart: a face notched where they meet trips later booleans that
    // cut at that height (#1510).
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (id, fs) in &by_edge {
        if let [a, b] = fs[..]
            && a != b
            && (keep.is_empty() || !edges.get(id).is_some_and(|e| on_split_line(&edge_points(e), keep)))
            && let (Some(Some(ca)), Some(Some(cb))) = (cyls.get(a), cyls.get(b))
            && ca.same(cb, tol)
            && let (Some(Some(ha)), Some(Some(hb))) = (heights.get(a), heights.get(b))
            && (ha.0 - (hb.0 + (cb.o - ca.o).dot(ca.a))).abs() < tol
            && (ha.1 - (hb.1 + (cb.o - ca.o).dot(ca.a))).abs() < tol
        {
            pairs.push((a.min(b), a.max(b)));
        }
    }
    pairs.sort_unstable();
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    for (a, b) in &pairs {
        let (ra, rb) = (find(&mut parent, *a), find(&mut parent, *b));
        if let Some(x) = parent.get_mut(ra.max(rb)) {
            *x = ra.min(rb);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..faces.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    // Each face of the result by its first piece: the piece itself, or a merged face.
    let mut out: BTreeMap<usize, (mt::Face, Vec<usize>)> = BTreeMap::new();
    let mut merged_any = false;
    for (root, members) in &groups {
        let (Some(Some(c)), true) = (cyls.get(*root), members.len() > 1) else {
            for m in members {
                out.insert(*m, (faces.get(*m)?.clone(), vec![*m]));
            }
            continue;
        };
        let u = c.a.any_perp();
        let w = c.a.cross(u);
        let spans: Option<Vec<Span>> = members.iter().map(|m| span_of(faces.get(*m)?, c, u, w)).collect();
        let runs = spans.map(|s| runs(members, &s, &pairs)).unwrap_or_default();
        let mut done: Vec<usize> = Vec::new();
        for (run, (t0, t1, z0, z1)) in runs {
            let pieces: Option<Vec<&mt::Face>> = run.iter().map(|m| faces.get(*m)).collect();
            if let Some(face) = pieces.and_then(|p| merged_face(&p, c, (u, w), (t0, t1, z0, z1), tol))
                && let Some(first) = run.first()
            {
                out.insert(*first, (face, run.clone()));
                done.extend(&run);
                merged_any = true;
            }
        }
        for m in members.iter().filter(|m| !done.contains(m)) {
            out.insert(*m, (faces.get(*m)?.clone(), vec![*m]));
        }
    }
    if !merged_any {
        return None;
    }
    let (faces, from): (Vec<mt::Face>, Vec<Vec<usize>>) = out.into_values().unzip();
    Some((faces.into(), from))
}

/// The group's faces (`members`, spanning `spans`) cut into runs round the axis of at most
/// [`MAX_SWEEP`]: as few runs as can be, the longest of them as short as can be (no sliver left
/// beside a full half turn, where a blend along the edge next to it would run off it), each
/// split into its edge-connected parts (by `pairs`): those of more than one face, sorted, with
/// the turn and height they span.
fn runs(members: &[usize], spans: &[Span], pairs: &[(usize, usize)]) -> Vec<(Vec<usize>, (f64, f64, f64, f64))> {
    let n = members.len();
    if n > MAX_STRIPS {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    let start = |k: usize| spans.get(k).map_or(0.0, |s| s.lo.rem_euclid(TAU));
    order.sort_by(|a, b| start(*a).total_cmp(&start(*b)));
    // The runs from each starting face (each face's span moved a turn on past the last), none
    // longer than `limit`.
    let cut = |s: usize, limit: f64| -> Vec<Vec<(usize, f64, f64)>> {
        let mut out: Vec<Vec<(usize, f64, f64)>> = Vec::new();
        let mut from = f64::NAN;
        for j in s..s + n {
            let Some((k, sp)) = order.get(j % n).and_then(|k| Some((*k, spans.get(*k)?))) else { continue };
            let lo = start(k) + if j >= n { TAU } else { 0.0 };
            let hi = lo + (sp.hi - sp.lo);
            match out.last_mut() {
                Some(run) if hi - from <= limit => run.push((k, lo, hi)),
                _ => {
                    from = lo;
                    out.push(vec![(k, lo, hi)]);
                }
            }
        }
        out
    };
    let Some(fewest) = (0..n).map(|s| cut(s, MAX_SWEEP).len()).min() else { return Vec::new() };
    // The shortest limit that still takes that few runs from some start: a run's length is
    // always that of some faces in order, so the limit is found among those.
    let mut limits: Vec<f64> = (0..n)
        .flat_map(|i| (0..n).map(move |j| (i, j)))
        .filter_map(|(i, j)| {
            let (a, b) = (spans.get(*order.get(i)?)?, spans.get(*order.get(j)?)?);
            let (lo, hi) = (a.lo.rem_euclid(TAU), b.lo.rem_euclid(TAU) + (b.hi - b.lo));
            let len = if hi >= lo { hi - lo } else { hi + TAU - lo };
            (len <= MAX_SWEEP).then_some(len)
        })
        .chain([MAX_SWEEP])
        .collect();
    limits.sort_by(f64::total_cmp);
    let fits = |limit: f64| (0..n).find(|s| cut(*s, limit).len() == fewest);
    let (mut lo, mut hi) = (0, limits.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi) / 2;
        if limits.get(mid).is_some_and(|l| fits(*l + 1e-9).is_some()) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let limit = limits.get(lo).map_or(MAX_SWEEP, |l| (l + 1e-9).min(MAX_SWEEP));
    let Some(best) = fits(limit).map(|s| cut(s, limit)) else { return Vec::new() };
    let mut result = Vec::new();
    for run in best {
        // Edge-connected parts of the run.
        let ks: Vec<usize> = run.iter().map(|x| x.0).collect();
        let mut parent: Vec<usize> = (0..n).collect();
        for (a, b) in pairs {
            let (Some(ia), Some(ib)) = (members.iter().position(|m| m == a), members.iter().position(|m| m == b)) else { continue };
            if ks.contains(&ia) && ks.contains(&ib) {
                let (ra, rb) = (find(&mut parent, ia), find(&mut parent, ib));
                if let Some(x) = parent.get_mut(ra.max(rb)) {
                    *x = ra.min(rb);
                }
            }
        }
        let mut parts: BTreeMap<usize, Vec<(usize, f64, f64)>> = BTreeMap::new();
        for x in &run {
            parts.entry(find(&mut parent, x.0)).or_default().push(*x);
        }
        for part in parts.into_values().filter(|p| p.len() > 1) {
            let t0 = part.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
            let t1 = part.iter().map(|x| x.2).fold(f64::NEG_INFINITY, f64::max);
            let z0 = part.iter().filter_map(|x| spans.get(x.0)).map(|s| s.z0).fold(f64::INFINITY, f64::min);
            let z1 = part.iter().filter_map(|x| spans.get(x.0)).map(|s| s.z1).fold(f64::NEG_INFINITY, f64::max);
            let mut faces: Vec<usize> = part.iter().filter_map(|x| members.get(x.0).copied()).collect();
            faces.sort_unstable();
            result.push((faces, (t0, t1, z0, z1)));
        }
    }
    result
}
