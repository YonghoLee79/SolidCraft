//! Immutable spline support grids reused while sampling borrowed curves.
//! Grid scan adapted from truck-geotrait 0.4.0 algo/surface.rs
//! (Apache-2.0, copyright RICOS Co. Ltd.); see ATTRIBUTION.md.
use crate::surface_key::SurfaceKey;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use truck_meshalgo::prelude::TrySubs;
#[cfg(test)]
use truck_modeling::SearchNearestParameter;
use truck_modeling::{self as mt, ParametricCurve, ParametricSurface, ParametricSurface3D};

// Matches truck-geometry's private PRESEARCH_DIVISION for these concrete surfaces.
const DIVISIONS: usize = 50;
#[cfg(test)]
std::thread_local! { static GRID_DISTANCES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
std::thread_local! { static GRID_POINTS_PREPARED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Clone)]
pub(super) struct Grid {
    u: Vec<f64>,
    v: Vec<f64>,
    points: Vec<mt::Point3>,
}
impl Grid {
    pub(super) fn prepare(surface: &mt::Surface) -> Option<Self> {
        use mt::BoundedSurface;
        let ((u0, u1), (v0, v1)) = match surface {
            mt::Surface::BSplineSurface(s) => s.range_tuple(),
            mt::Surface::NurbsSurface(s) => s.range_tuple(),
            _ => return None,
        };
        let coordinates = |lo: f64, hi: f64| {
            (0..=DIVISIONS)
                .map(|i| {
                    let p = i as f64 / DIVISIONS as f64;
                    lo * (1.0 - p) + hi * p
                })
                .collect::<Vec<_>>()
        };
        let u = coordinates(u0, u1);
        let v = coordinates(v0, v1);
        let points = if let Some(points) = crate::grid_basis::batched(surface, &u, &v) {
            #[cfg(test)]
            GRID_POINTS_PREPARED.with(|n| n.set(n.get().saturating_add(points.len())));
            points
        } else {
            let mut points = Vec::with_capacity((DIVISIONS + 1) * (DIVISIONS + 1));
            for &a in &u {
                for &b in &v {
                    #[cfg(test)]
                    GRID_POINTS_PREPARED.with(|n| n.set(n.get().saturating_add(1)));
                    points.push(surface.subs(a, b));
                }
            }
            points
        };
        Some(Self { u, v, points })
    }
    pub(super) fn seed(&self, point: mt::Point3) -> (f64, f64) {
        use mt::MetricSpace;
        let mut seed = (0.0, 0.0);
        let mut minimum = f64::INFINITY;
        // Retain U-major order, strict ties, the zero fallback and every distance,
        // including non-finite samples. Only the immutable surface evaluations moved.
        for (i, row) in self.points.chunks(DIVISIONS + 1).enumerate() {
            for (j, sample) in row.iter().enumerate() {
                #[cfg(test)]
                GRID_DISTANCES.with(|n| n.set(n.get().saturating_add(1)));
                let distance = sample.distance2(point);
                if distance < minimum
                    && let Some((&u, &v)) = self.u.get(i).zip(self.v.get(j))
                {
                    minimum = distance;
                    seed = (u, v);
                }
            }
        }
        seed
    }
}

/// Two grids shared by consecutive curves in one planar-surface pass. No state
/// survives the pass; an unkeyable curve clears these before using local grids.
/// Grid data stays within 126,480 bytes, matching two ordinary support grids.
/// At most four live support keys add 32 KiB of words, plus allocation headers.
pub(crate) struct GridScratch {
    entries: Mutex<VecDeque<(Arc<SurfaceKey>, Grid)>>,
}
impl GridScratch {
    pub(crate) fn new() -> Self {
        Self { entries: Mutex::new(VecDeque::with_capacity(2)) }
    }
    fn seed(&self, key: &Arc<SurfaceKey>, surface: &mt::Surface, point: mt::Point3) -> Option<(f64, f64)> {
        let mut entries = self.entries.try_lock().ok()?;
        if let Some(index) = entries.iter().position(|(cached, _)| cached == key) {
            let entry = entries.remove(index)?;
            let seed = entry.1.seed(point);
            entries.push_back(entry);
            return Some(seed);
        }
        if entries.len() >= 2 {
            // Release the old data before preparing the replacement.
            drop(entries.pop_front());
        }
        let grid = Grid::prepare(surface)?;
        let seed = grid.seed(point);
        entries.push_back((Arc::clone(key), grid));
        Some(seed)
    }
}

struct PreparedSurface<'a> {
    original: &'a mt::Surface,
    grid: RefCell<Option<Grid>>,
    local_shared: Option<Arc<Mutex<Option<Grid>>>>,
    shared: Option<(&'a GridScratch, Arc<SurfaceKey>)>,
}
impl Clone for PreparedSurface<'_> {
    // Ordinary clones start fresh; shared clones retain the same two-grid bound.
    fn clone(&self) -> Self {
        let mut cloned = Self::new(self.original);
        cloned.local_shared = self.local_shared.as_ref().map(Arc::clone);
        cloned.shared = self.shared.as_ref().map(|(scratch, key)| (*scratch, Arc::clone(key)));
        cloned
    }
}
impl<'a> PreparedSurface<'a> {
    fn new(original: &'a mt::Surface) -> Self {
        Self { original, grid: RefCell::new(None), local_shared: None, shared: None }
    }
    fn seed(&self, point: mt::Point3) -> Option<(f64, f64)> {
        if !matches!(self.original, mt::Surface::BSplineSurface(_) | mt::Surface::NurbsSurface(_)) {
            return None;
        }
        if let Some((scratch, key)) = &self.shared {
            return scratch.seed(key, self.original, point);
        }
        if let Some(local) = &self.local_shared {
            let mut grid = local.try_lock().ok()?;
            if grid.is_none() {
                *grid = Grid::prepare(self.original);
            }
            return grid.as_ref().map(|grid| grid.seed(point));
        }
        let mut grid = self.grid.try_borrow_mut().ok()?;
        if grid.is_none() {
            *grid = Grid::prepare(self.original);
        }
        grid.as_ref().map(|g| g.seed(point))
    }
}
// Drop checking keeps the support's scratch lifetime live through destruction,
// including clones retained after their last sampling call.
impl Drop for PreparedSurface<'_> {
    fn drop(&mut self) {
        drop(self.grid.get_mut().take());
        drop(self.shared.take());
        drop(self.local_shared.take());
    }
}
impl ParametricSurface for PreparedSurface<'_> {
    type Point = mt::Point3;
    type Vector = mt::Vector3;
    fn subs(&self, u: f64, v: f64) -> mt::Point3 {
        self.original.subs(u, v)
    }
    fn uder(&self, u: f64, v: f64) -> mt::Vector3 {
        self.original.uder(u, v)
    }
    fn vder(&self, u: f64, v: f64) -> mt::Vector3 {
        self.original.vder(u, v)
    }
    fn uuder(&self, u: f64, v: f64) -> mt::Vector3 {
        self.original.uuder(u, v)
    }
    fn uvder(&self, u: f64, v: f64) -> mt::Vector3 {
        self.original.uvder(u, v)
    }
    fn vvder(&self, u: f64, v: f64) -> mt::Vector3 {
        self.original.vvder(u, v)
    }
    fn parameter_range(&self) -> (mt::ParameterRange, mt::ParameterRange) {
        self.original.parameter_range()
    }
    fn u_period(&self) -> Option<f64> {
        self.original.u_period()
    }
    fn v_period(&self) -> Option<f64> {
        self.original.v_period()
    }
}
impl ParametricSurface3D for PreparedSurface<'_> {
    fn normal(&self, u: f64, v: f64) -> mt::Vector3 {
        self.original.normal(u, v)
    }
}
impl mt::SearchNearestParameter<mt::D2> for PreparedSurface<'_> {
    type Point = mt::Point3;
    fn search_nearest_parameter<H: Into<mt::SPHint2D>>(&self, point: mt::Point3, hint: H, trials: usize) -> Option<(f64, f64)> {
        let hint = hint.into();
        let effective = match hint {
            mt::SPHint2D::None => self.seed(point).map_or(hint, |(u, v)| mt::SPHint2D::Parameter(u, v)),
            _ => hint,
        };
        self.original.search_nearest_parameter(point, effective, trials)
    }
}

pub(crate) struct CurveSampler<'a> {
    original: &'a mt::Curve,
    supports: Option<(PreparedSurface<'a>, PreparedSurface<'a>)>,
}
impl<'a> CurveSampler<'a> {
    pub(crate) fn new(original: &'a mt::Curve) -> Self {
        // Borrow the actual supports: transforms, weights and parameterization stay
        // unchanged and cannot be mutated for this sampler's lifetime. Each spline
        // needs at most 2,601 points plus 102 parameters (about 63.2 KB); all storage
        // drops with this sampler and is never shared between edges or operations.
        let supports = match original {
            mt::Curve::IntersectionCurve(c) => Some((PreparedSurface::new(c.surface0()), PreparedSurface::new(c.surface1()))),
            _ => None,
        };
        Self { original, supports }
    }
    /// Drop guards retain the exclusive scratch loan until this sampler and its
    /// prepared support clones have released all grids and key references.
    /// Large support keys retain the existing local cache, after dropping the
    /// shared grids, rather than repeating an uncached search for every sample.
    pub(crate) fn with_scratch(original: &'a mt::Curve, scratch: &'a mut GridScratch) -> Self {
        let mt::Curve::IntersectionCurve(curve) = original else {
            return Self::new(original);
        };
        let key_a = SurfaceKey::new(curve.surface0()).map(Arc::new);
        let key_b = SurfaceKey::new(curve.surface1()).map(Arc::new);
        let unkeyable = |surface: &mt::Surface, key: &Option<Arc<SurfaceKey>>| {
            matches!(surface, mt::Surface::BSplineSurface(_) | mt::Surface::NurbsSurface(_)) && key.is_none()
        };
        if unkeyable(curve.surface0(), &key_a) || unkeyable(curve.surface1(), &key_b) {
            match scratch.entries.get_mut() {
                Ok(entries) => entries.clear(),
                Err(poisoned) => poisoned.into_inner().clear(),
            }
            let mut local = Self::new(original);
            if let Some((a, b)) = local.supports.as_mut() {
                // Clones of this fallback share its two local grids. They cannot
                // multiply retained grid data while extending the scratch loan.
                a.local_shared = Some(Arc::new(Mutex::new(None)));
                b.local_shared = Some(Arc::new(Mutex::new(None)));
            }
            return local;
        }
        let shared: &'a GridScratch = scratch;
        let prepared = |surface: &'a mt::Surface, key: Option<Arc<SurfaceKey>>| PreparedSurface {
            original: surface,
            grid: RefCell::new(None),
            local_shared: None,
            shared: key.map(|key| (shared, key)),
        };
        Self { original, supports: Some((prepared(curve.surface0(), key_a), prepared(curve.surface1(), key_b))) }
    }
    pub(crate) fn at(&self, t: f64) -> Option<mt::Point3> {
        match (self.original, self.supports.as_ref()) {
            (mt::Curve::IntersectionCurve(c), Some((a, b))) => {
                mt::double_projection(a, None, b, None, c.leader().subs(t), c.leader().der(t), 100).map(|triple| triple.0)
            }
            _ => self.original.try_subs(t),
        }
    }
    /// Keep the caller's existing guarded `subs` failure contract.
    pub(crate) fn subs(&self, t: f64) -> mt::Point3 {
        self.at(t).unwrap_or_else(|| self.original.subs(t))
    }
}

// The explicit destructor also retains the scratch loan when this owner is kept
// after its last use. Its supports release their resources before the loan ends.
impl Drop for CurveSampler<'_> {
    fn drop(&mut self) {
        drop(self.supports.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits(p: mt::Point3) -> [u64; 3] {
        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
    }
    fn surface() -> mt::Surface {
        let p = mt::Point3::new;
        mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(2), mt::KnotVec::bezier_knot(1)),
            vec![vec![p(0., 0., -1.), p(0., 0., 1.)], vec![p(0.5, 0., -1.), p(0.5, 0., 1.)], vec![p(1., 1., -1.), p(1., 1., 1.)]],
        ))
    }
    fn intersection() -> mt::Curve {
        let p = mt::Point3::new;
        let plane = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.)));
        let leader = mt::Leader::BSpline(mt::BSplineCurve::new(mt::KnotVec::bezier_knot(2), vec![p(0., 0., 0.), p(0.5, 0., 0.), p(1., 1., 0.)]));
        mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(plane), Box::new(surface()), leader, 1e-8))
    }
    #[test]
    fn grid_samples_and_seeds_match_original_bitwise() {
        let s = surface();
        let grid = Grid::prepare(&s).unwrap();
        for (i, &u) in grid.u.iter().enumerate() {
            for (j, &v) in grid.v.iter().enumerate() {
                assert_eq!(bits(grid.points[i * 51 + j]), bits(s.subs(u, v)));
            }
        }
        use mt::BoundedSurface;
        let range = match &s {
            mt::Surface::BSplineSurface(b) => b.range_tuple(),
            _ => panic!("expected a B-spline fixture"),
        };
        for point in [mt::Point3::new(0.25, 0.1, 0.), mt::Point3::new(-1., 2., 1.), mt::Point3::new(f64::NAN, 0., 0.)] {
            let expected = mt::algo::surface::presearch(&s, point, range, 50);
            let actual = grid.seed(point);
            assert_eq!((actual.0.to_bits(), actual.1.to_bits()), (expected.0.to_bits(), expected.1.to_bits()));
        }
    }
    #[test]
    fn all_33_intersection_samples_match_original_bitwise() {
        let curve = intersection();
        let sampler = CurveSampler::new(&curve);
        let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        let distances_before = GRID_DISTANCES.with(std::cell::Cell::get);
        for i in 0..=32 {
            let t = i as f64 / 32.;
            assert_eq!(sampler.at(t).map(bits), curve.try_subs(t).map(bits), "sample {i}");
        }
        assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2601);
        assert_eq!(GRID_DISTANCES.with(std::cell::Cell::get) - distances_before, 33 * 2601);
        let (first, second) = sampler.supports.as_ref().unwrap();
        assert!(first.grid.borrow().is_none(), "plane must not prepare a grid");
        assert_eq!(second.grid.borrow().as_ref().unwrap().points.len(), 2601);
    }
    #[test]
    fn two_spline_supports_prepare_once_and_preserve_every_sample() {
        let p = mt::Point3::new;
        let plane = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
            vec![vec![p(0., 0., 0.), p(0., 1., 0.)], vec![p(1., 0., 0.), p(1., 1., 0.)]],
        ));
        let mt::Curve::IntersectionCurve(original) = intersection() else { panic!("expected an intersection fixture") };
        let curve = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(
            Box::new(plane),
            Box::new(original.surface1().clone()),
            original.leader().clone(),
            1e-8,
        ));
        let sampler = CurveSampler::new(&curve);
        let prepared_before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        let distances_before = GRID_DISTANCES.with(std::cell::Cell::get);
        for k in 0..=32 {
            let t = k as f64 / 32.;
            let expected = curve.try_subs(t);
            assert!(expected.is_some());
            assert_eq!(sampler.at(t).map(bits), expected.map(bits));
        }
        assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - prepared_before, 5202);
        assert_eq!(GRID_DISTANCES.with(std::cell::Cell::get) - distances_before, 33 * 5202);
    }
    #[test]
    fn reversed_curve_and_separate_sampler_keep_original_results() {
        use mt::{BoundedCurve, Invertible};
        let mut curve = intersection();
        curve.invert();
        let (lo, hi) = curve.range_tuple();
        let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        for _ in 0..2 {
            let sampler = CurveSampler::new(&curve);
            for k in 0..=32 {
                let t = lo + (hi - lo) * k as f64 / 32.0;
                assert_eq!(bits(sampler.subs(t)), bits(curve.subs(t)));
            }
        }
        assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2 * 2601);
    }
    #[test]
    fn explicit_hints_and_borrow_conflicts_fall_back_unchanged() {
        let s = surface();
        let prepared = PreparedSurface::new(&s);
        let point = mt::Point3::new(0.4, 0.3, 0.1);
        for hint in [
            mt::SPHint2D::Parameter(0.2, 0.7),
            mt::SPHint2D::Range((0.1, 0.8), (0.2, 0.9)),
            mt::SPHint2D::Range((0.8, 0.1), (0.9, 0.2)),
            mt::SPHint2D::Range((0.4, 0.4), (0.7, 0.7)),
        ] {
            assert_eq!(prepared.search_nearest_parameter(point, hint, 10), s.search_nearest_parameter(point, hint, 10));
            assert!(prepared.grid.borrow().is_none());
        }
        let _borrow = prepared.grid.borrow_mut();
        assert_eq!(prepared.search_nearest_parameter(point, None, 10), s.search_nearest_parameter(point, None, 10));
    }
}

#[cfg(test)]
mod expanded_tests {
    use super::*;
    use mt::{BoundedSurface, Transformed};
    fn pbits(p: mt::Point3) -> [u64; 3] {
        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
    }
    fn vbits(v: mt::Vector3) -> [u64; 3] {
        [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
    }
    fn uvbits(uv: Option<(f64, f64)>) -> Option<(u64, u64)> {
        uv.map(|(u, v)| (u.to_bits(), v.to_bits()))
    }
    fn rational(weights: [[f64; 2]; 2]) -> mt::Surface {
        let controls = (0..2)
            .map(|i| {
                (0..2)
                    .map(|j| {
                        let w = weights[i][j];
                        mt::Vector4::new(i as f64 * w, j as f64 * w, (i * j) as f64 * w, w)
                    })
                    .collect()
            })
            .collect();
        mt::Surface::NurbsSurface(mt::NurbsSurface::new(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
            controls,
        )))
    }
    fn constant_surface() -> mt::Surface {
        mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
            vec![vec![mt::Point3::new(0., 0., 0.); 2]; 2],
        ))
    }
    fn check_grid(surface: &mt::Surface, queries: &[mt::Point3]) {
        let grid = Grid::prepare(surface).unwrap();
        let range = match surface {
            mt::Surface::BSplineSurface(s) => s.range_tuple(),
            mt::Surface::NurbsSurface(s) => s.range_tuple(),
            _ => panic!("expected a spline fixture"),
        };
        for (i, &u) in grid.u.iter().enumerate() {
            for (j, &v) in grid.v.iter().enumerate() {
                assert_eq!(pbits(grid.points[i * 51 + j]), pbits(surface.subs(u, v)));
            }
        }
        for &point in queries {
            let expected = mt::algo::surface::presearch(surface, point, range, 50);
            assert_eq!(uvbits(Some(grid.seed(point))), uvbits(Some(expected)));
        }
    }
    #[test]
    fn rational_grids_preserve_weights_ties_and_nonfinite_seed_behavior() {
        let queries = [
            mt::Point3::new(0.5, 0.5, 0.),
            mt::Point3::new(0.5 + f64::EPSILON, 0.5, 0.),
            mt::Point3::new(-0.0, 0., 0.),
            mt::Point3::new(f64::NAN, 0., 0.),
            mt::Point3::new(f64::INFINITY, 0., 0.),
        ];
        for weights in [
            [[1., 2.], [0.5, 3.]],
            [[1., 1.], [1., 1.]],
            [[0., 0.], [0., 0.]],
            [[1., -1.], [-1., 1.]],
            [[1e-200, 2e-200], [3e-200, 4e-200]],
            [[f64::NAN, 1.], [1., 1.]],
        ] {
            check_grid(&rational(weights), &queries);
        }
        // Every sample is tied: strict '<' must keep the first grid coordinate.
        let constant = constant_surface();
        check_grid(&constant, &queries);
        let g = Grid::prepare(&constant).unwrap();
        assert_eq!(uvbits(Some(g.seed(mt::Point3::new(1., 2., 3.)))), uvbits(Some((g.u[0], g.v[0]))));
    }
    #[test]
    fn weighted_transformed_intersections_match_all_33_projection_results() {
        let p = mt::Point3::new;
        let plane = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.)));
        let leader = mt::Leader::BSpline(mt::BSplineCurve::new(mt::KnotVec::bezier_knot(1), vec![p(0.1, 0.03, 0.02), p(0.9, 0.03, 0.02)]));
        let curve = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(
            Box::new(plane),
            Box::new(rational([[1., 2.], [0.5, 3.]])),
            leader,
            1e-8,
        ));
        for transform in [
            mt::Matrix4::from_scale(1.),
            mt::Matrix4::from_translation(mt::Vector3::new(3., -2., 1.)),
            mt::Matrix4::from_nonuniform_scale(-2., 3., 0.5),
            mt::Matrix4::from_angle_z(mt::Rad(0.37)),
        ] {
            let transformed = curve.transformed(transform);
            let sampler = CurveSampler::new(&transformed);
            for k in 0..=32 {
                let t = k as f64 / 32.;
                let expected = transformed.try_subs(t);
                assert!(expected.is_some(), "valid weighted fixture sample {k}");
                assert_eq!(sampler.at(t).map(pbits), expected.map(pbits));
            }
        }
    }
    #[test]
    fn nonunit_surface_bounds_keep_original_grid_and_nearest_parameters() {
        let p = mt::Point3::new;
        let s = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::from(vec![-3., -3., 2., 2.]), mt::KnotVec::from(vec![4., 4., 9., 9.])),
            vec![vec![p(0., 0., 0.), p(0., 1., 0.)], vec![p(1., 0., 0.), p(1., 1., 1.)]],
        ));
        let queries = [p(0.2, 0.3, 0.1), p(-1., 2., 0.4), s.subs(-0.8, 6.1)];
        check_grid(&s, &queries);
        let wrapped = PreparedSurface::new(&s);
        for point in queries {
            assert_eq!(uvbits(wrapped.search_nearest_parameter(point, None, 10)), uvbits(s.search_nearest_parameter(point, None, 10)));
        }
    }
    #[test]
    fn transformed_rational_supports_forward_normal_periods_and_newton_exactly() {
        let original = rational([[1., 2.], [0.5, 3.]]);
        for transform in [
            mt::Matrix4::from_translation(mt::Vector3::new(3., -2., 1.)),
            mt::Matrix4::from_nonuniform_scale(-2., 3., 0.5),
            mt::Matrix4::from_angle_z(mt::Rad(0.37)),
        ] {
            let s = original.transformed(transform);
            let wrapped = PreparedSurface::new(&s);
            check_grid(&s, &[mt::Point3::new(0.5, 0.5, 0.)]);
            assert_eq!(wrapped.parameter_range(), s.parameter_range());
            assert_eq!(wrapped.u_period(), s.u_period());
            assert_eq!(wrapped.v_period(), s.v_period());
            for (u, v) in [(0., 0.), (0.37, 0.61), (1., 1.)] {
                assert_eq!(pbits(wrapped.subs(u, v)), pbits(s.subs(u, v)));
                assert_eq!(vbits(wrapped.normal(u, v)), vbits(s.normal(u, v)));
                assert_eq!(vbits(wrapped.uder(u, v)), vbits(s.uder(u, v)));
                assert_eq!(vbits(wrapped.vder(u, v)), vbits(s.vder(u, v)));
                assert_eq!(vbits(wrapped.uuder(u, v)), vbits(s.uuder(u, v)));
                assert_eq!(vbits(wrapped.uvder(u, v)), vbits(s.uvder(u, v)));
                assert_eq!(vbits(wrapped.vvder(u, v)), vbits(s.vvder(u, v)));
                let point = s.subs(u, v);
                assert_eq!(uvbits(wrapped.search_nearest_parameter(point, None, 10)), uvbits(s.search_nearest_parameter(point, None, 10)));
            }
        }
        let curve = mt::Curve::Line(mt::Line(mt::Point3::new(1., 0., 0.), mt::Point3::new(1., 0., 1.)));
        let revolved = mt::Surface::RevolutedCurve(mt::Processor::new(mt::RevolutedCurve::by_revolution(
            curve,
            mt::Point3::new(0., 0., 0.),
            mt::Vector3::unit_z(),
        )));
        let wrapped = PreparedSurface::new(&revolved);
        assert_eq!(wrapped.u_period(), revolved.u_period());
        assert_eq!(wrapped.v_period(), revolved.v_period());
        assert_eq!(vbits(wrapped.normal(0.3, 0.7)), vbits(revolved.normal(0.3, 0.7)));
        assert!(wrapped.grid.borrow().is_none());
    }
    #[test]
    fn invalid_interior_sample_retains_rejection() {
        let p = mt::Point3::new;
        let plane0 = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.)));
        let plane1 = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 0., 1.)));
        let leader = mt::Leader::BSpline(mt::BSplineCurve::new(
            mt::KnotVec::bezier_knot(3),
            vec![p(0., 0., 0.), p(0.25, 4. / 3., 0.), p(0., 4. / 3., 0.), p(0.25, 0., 0.)],
        ));
        let curve = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(plane0), Box::new(plane1), leader, 0.01));
        let sampler = CurveSampler::new(&curve);
        assert!(curve.try_subs(0.).is_some() && curve.try_subs(1.).is_some());
        assert!(curve.try_subs(0.5).is_none());
        // Healing already calls subs under the kernel guard. Preserve that failure,
        // while validation continues to consume the fallible sampling result.
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| curve.subs(0.5))).is_err());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sampler.subs(0.5))).is_err());
        for k in 0..=32 {
            let t = k as f64 / 32.;
            assert_eq!(sampler.at(t).map(pbits), curve.try_subs(t).map(pbits));
        }
    }
    #[test]
    fn failure_of_first_support_does_not_prepare_second_support() {
        let p = mt::Point3::new;
        let leader = mt::Leader::Polyline(mt::PolylineCurve(vec![p(1., 1., 1.), p(2., 1., 1.)]));
        let curve = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(
            // NaN homogeneous coefficients make the original nearest search exhaust its
            // unchanged iteration budget; a constant surface instead returns Some(seed).
            Box::new(rational([[f64::NAN, 1.], [1., 1.]])),
            Box::new(rational([[1., 2.], [0.5, 3.]])),
            leader,
            0.01,
        ));
        let sampler = CurveSampler::new(&curve);
        if let mt::Curve::IntersectionCurve(c) = &curve {
            assert!(
                c.surface0().search_nearest_parameter(c.leader().subs(0.5), None, 10).is_none(),
                "fixture must fail in the first original nearest search, before surface1"
            );
        }
        assert!(curve.try_subs(0.5).is_none(), "nonfinite first support must reject the fixture");
        let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        assert!(sampler.at(0.5).is_none());
        assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2601);
        let (first, second) = sampler.supports.as_ref().unwrap();
        assert!(first.grid.borrow().is_some());
        assert!(second.grid.borrow().is_none());
    }
}

#[cfg(test)]
#[path = "curve_sampler_scratch_tests.rs"]
mod scratch_tests;
