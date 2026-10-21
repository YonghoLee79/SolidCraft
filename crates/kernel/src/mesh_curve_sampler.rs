//! Reuse intersection support grids only while meshing one shell.
//! The grid and ordered seed scan are shared with curve_sampler; see its attribution.
use crate::curve_sampler::Grid;
use crate::surface_key::SurfaceKey;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use truck_meshalgo::tessellation::TrySubs;
use truck_modeling::{self as mt, ParametricCurve, ParametricSurface, ParametricSurface3D};

// At most 32,378,880 bytes of grid data and 4 MiB of key words per shell, plus headers.
// Each key construction temporarily needs at most 8 KiB. No state survives this shell.
// Only unhinted spline support searches consume a slot. Active entries retain their
// slots even after eviction; unavailable slots keep the original search unchanged.
pub(super) const MAX_GRIDS: usize = 512;
// Local references are weak so an idle entry can be recycled under pressure.
// A separate permit counts every live entry, including an evicted entry another
// thread upgraded just before removal. It is returned only after its grid drops.
struct Slots {
    live: AtomicUsize,
    #[cfg(test)]
    peak: AtomicUsize,
    maximum: usize,
}
struct Permit(Arc<Slots>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::AcqRel);
    }
}
struct GridEntry {
    grid: OnceLock<Option<Grid>>,
    last_used: AtomicUsize,
    _permit: Permit,
}
impl Drop for GridEntry {
    fn drop(&mut self) {
        // Destroy grid allocation before returning its slot through Permit::drop.
        drop(self.grid.take());
    }
}
type SharedGrid = Arc<GridEntry>;
#[derive(Default)]
struct LocalState {
    cached: Weak<GridEntry>,
    disabled: bool,
}
type LocalGrid = Mutex<LocalState>;
enum Admission {
    Shared(SharedGrid),
    Uncacheable,
    Unavailable,
}
pub(super) struct MeshBudget {
    maximum: usize,
    cells: Mutex<HashMap<SurfaceKey, SharedGrid>>,
    slots: Arc<Slots>,
    clock: AtomicUsize,
    #[cfg(test)]
    evictions: AtomicUsize,
    #[cfg(test)]
    misses: AtomicUsize,
}
impl MeshBudget {
    pub(super) fn new(grids: usize) -> Self {
        Self {
            maximum: grids,
            cells: Mutex::new(HashMap::new()),
            slots: Arc::new(Slots {
                live: AtomicUsize::new(0),
                #[cfg(test)]
                peak: AtomicUsize::new(0),
                maximum: grids,
            }),
            clock: AtomicUsize::new(0),
            #[cfg(test)]
            evictions: AtomicUsize::new(0),
            #[cfg(test)]
            misses: AtomicUsize::new(0),
        }
    }
    fn touch(&self, cell: &GridEntry) {
        cell.last_used.store(self.clock.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    }
    fn reserve(&self) -> Option<Permit> {
        let mut live = self.slots.live.load(Ordering::Acquire);
        loop {
            if live >= self.slots.maximum {
                return None;
            }
            let next = live.checked_add(1)?;
            match self.slots.live.compare_exchange_weak(live, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    #[cfg(test)]
                    self.slots.peak.fetch_max(next, Ordering::Relaxed);
                    return Some(Permit(self.slots.clone()));
                }
                Err(current) => live = current,
            }
        }
    }
    fn cell(&self, surface: &mt::Surface) -> Admission {
        if self.maximum == 0 {
            return Admission::Unavailable;
        }
        let Some(key) = SurfaceKey::new(surface) else {
            return Admission::Uncacheable;
        };
        let Ok(mut cells) = self.cells.lock() else {
            return Admission::Unavailable;
        };
        if let Some(cell) = cells.get(&key) {
            self.touch(cell);
            return Admission::Shared(cell.clone());
        }
        if self.slots.live.load(Ordering::Acquire) >= self.maximum {
            let victim =
                cells.values().filter(|cell| Arc::strong_count(cell) == 1).min_by_key(|cell| cell.last_used.load(Ordering::Relaxed)).map(Arc::as_ptr);
            if let Some(victim) = victim {
                cells.retain(|_, cell| Arc::as_ptr(cell) != victim);
                #[cfg(test)]
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        // A Weak upgrade may race eviction. Its live permit prevents over-admission.
        let Some(permit) = self.reserve() else {
            #[cfg(test)]
            self.misses.fetch_add(1, Ordering::Relaxed);
            return Admission::Unavailable;
        };
        let cell = Arc::new(GridEntry { grid: OnceLock::new(), last_used: AtomicUsize::new(0), _permit: permit });
        self.touch(&cell);
        cells.insert(key, cell.clone());
        Admission::Shared(cell)
    }
    #[cfg(test)]
    fn stats(&self) -> (usize, usize, usize, usize) {
        (
            self.slots.live.load(Ordering::Acquire),
            self.slots.peak.load(Ordering::Relaxed),
            self.evictions.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
        )
    }
}
struct PreparedSurface<'a> {
    original: &'a mt::Surface,
    grid: &'a LocalGrid,
    budget: &'a MeshBudget,
}
impl Clone for PreparedSurface<'_> {
    fn clone(&self) -> Self {
        Self { original: self.original, grid: self.grid, budget: self.budget }
    }
}
impl<'a> PreparedSurface<'a> {
    fn seed(&self, point: mt::Point3) -> Option<(f64, f64)> {
        if !matches!(self.original, mt::Surface::BSplineSurface(_) | mt::Surface::NurbsSurface(_)) {
            return None;
        }
        let cell = {
            let mut local = self.grid.lock().ok()?;
            if local.disabled {
                return None;
            }
            if let Some(cell) = local.cached.upgrade() {
                self.budget.touch(&cell);
                cell
            } else {
                match self.budget.cell(self.original) {
                    Admission::Shared(cell) => {
                        local.cached = Arc::downgrade(&cell);
                        cell
                    }
                    Admission::Uncacheable => {
                        local.disabled = true;
                        return None;
                    }
                    Admission::Unavailable => return None,
                }
            }
        };
        // Both locks are released before evaluating the bare immutable support.
        cell.grid.get_or_init(|| Grid::prepare(self.original)).as_ref().map(|grid| grid.seed(point))
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

#[derive(Clone)]
pub(super) struct MeshCurve {
    original: Arc<mt::Curve>,
    grids: Arc<(LocalGrid, LocalGrid)>,
    budget: Arc<MeshBudget>,
}
impl MeshCurve {
    pub(super) fn new(original: mt::Curve, budget: Arc<MeshBudget>) -> Self {
        Self { original: Arc::new(original), grids: Arc::new((Mutex::new(LocalState::default()), Mutex::new(LocalState::default()))), budget }
    }
}
impl ParametricCurve for MeshCurve {
    type Point = mt::Point3;
    type Vector = mt::Vector3;
    fn subs(&self, t: f64) -> mt::Point3 {
        self.original.subs(t)
    }
    fn der(&self, t: f64) -> mt::Vector3 {
        self.original.der(t)
    }
    fn der2(&self, t: f64) -> mt::Vector3 {
        self.original.der2(t)
    }
    fn parameter_range(&self) -> mt::ParameterRange {
        self.original.parameter_range()
    }
    fn period(&self) -> Option<f64> {
        self.original.period()
    }
}
impl mt::BoundedCurve for MeshCurve {}
impl mt::ParameterDivision1D for MeshCurve {
    type Point = mt::Point3;
    fn parameter_division(&self, range: (f64, f64), tol: f64) -> (Vec<f64>, Vec<mt::Point3>) {
        self.original.parameter_division(range, tol)
    }
}
impl TrySubs for MeshCurve {
    fn try_subs(&self, t: f64) -> Option<mt::Point3> {
        match self.original.as_ref() {
            mt::Curve::IntersectionCurve(c) => {
                let a = PreparedSurface { original: c.surface0(), grid: &self.grids.0, budget: &self.budget };
                let b = PreparedSurface { original: c.surface1(), grid: &self.grids.1, budget: &self.budget };
                // Same entry point, leader, initial hints, iteration limit and rejection behavior.
                mt::double_projection(&a, None, &b, None, c.leader().subs(t), c.leader().der(t), 100).map(|triple| triple.0)
            }
            _ => self.original.try_subs(t),
        }
    }
    fn approx_subs(&self, t: f64) -> mt::Point3 {
        self.original.approx_subs(t)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::unreachable)]
mod tests {
    use super::*;
    fn curve(offset: f64) -> mt::Curve {
        let p = mt::Point3::new;
        let plane = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.)));
        let spline = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(2), mt::KnotVec::bezier_knot(1)),
            vec![
                vec![p(offset, 0., -1.), p(offset, 0., 1.)],
                vec![p(offset + 0.5, 0., -1.), p(offset + 0.5, 0., 1.)],
                vec![p(offset + 1., 1., -1.), p(offset + 1., 1., 1.)],
            ],
        ));
        let leader = mt::Leader::BSpline(mt::BSplineCurve::new(
            mt::KnotVec::bezier_knot(2),
            vec![p(offset, 0., 0.), p(offset + 0.5, 0., 0.), p(offset + 1., 1., 0.)],
        ));
        mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(plane), Box::new(spline), leader, 1e-8))
    }
    fn bits(p: Option<mt::Point3>) -> Option<[u64; 3]> {
        p.map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
    }
    #[test]
    fn samples_and_exhausted_budget_match_original() {
        for cap in [0, 1, 512] {
            let raw = curve(0.);
            let adapted = MeshCurve::new(raw.clone(), Arc::new(MeshBudget::new(cap)));
            for i in 0..=64 {
                let t = i as f64 / 64.;
                assert_eq!(bits(raw.try_subs(t)), bits(adapted.try_subs(t)));
                assert_eq!(bits(Some(raw.approx_subs(t))), bits(Some(adapted.approx_subs(t))));
            }
        }
    }
    #[test]
    fn distinct_supports_keep_distinct_grids() {
        let budget = Arc::new(MeshBudget::new(2));
        for offset in [0., 10.] {
            let raw = curve(offset);
            let adapted = MeshCurve::new(raw.clone(), budget.clone());
            for i in 0..=32 {
                let t = i as f64 / 32.;
                assert_eq!(bits(raw.try_subs(t)), bits(adapted.try_subs(t)));
            }
        }
        assert_eq!(budget.cells.lock().unwrap().len(), 2);
    }
    #[test]
    fn concurrent_clones_initialize_once_and_drop() {
        let budget = Arc::new(MeshBudget::new(1));
        let adapted = MeshCurve::new(curve(0.), budget.clone());
        let weak = Arc::downgrade(&adapted.grids);
        let expected = bits(adapted.original.try_subs(0.4));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let c = adapted.clone();
                scope.spawn(move || assert_eq!(bits(c.try_subs(0.4)), expected));
            }
        });
        assert_eq!(budget.cells.lock().unwrap().len(), 1);
        drop(adapted);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn nonintersection_curve_keeps_original_contract() {
        let raw = mt::Curve::Line(mt::Line(mt::Point3::new(0., 0., 0.), mt::Point3::new(1., 2., 3.)));
        let adapted = MeshCurve::new(raw.clone(), Arc::new(MeshBudget::new(1)));
        for t in [-1., 0., 0.5, 1., 2.] {
            assert_eq!(bits(raw.try_subs(t)), bits(adapted.try_subs(t)));
            assert_eq!(raw.der(t), adapted.der(t));
            assert_eq!(raw.der2(t), adapted.der2(t));
        }
    }
    use mt::Transformed;
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
            let sampler = MeshCurve::new(transformed.clone(), Arc::new(MeshBudget::new(512)));
            for k in 0..=32 {
                let t = k as f64 / 32.;
                let expected = transformed.try_subs(t);
                assert!(expected.is_some(), "valid weighted fixture sample {k}");
                assert_eq!(bits(sampler.try_subs(t)), bits(expected));
            }
        }
    }
    #[test]
    fn equal_independent_supports_share_one_grid_and_operation_drop_releases_it() {
        let budget = Arc::new(MeshBudget::new(1));
        let a = MeshCurve::new(curve(0.), budget.clone());
        let b = MeshCurve::new(curve(0.), budget.clone());
        assert_eq!(bits(a.try_subs(0.3)), bits(b.try_subs(0.3)));
        assert_eq!(budget.cells.lock().unwrap().len(), 1);
        let ga = a.grids.1.lock().unwrap().cached.upgrade().unwrap();
        let gb = b.grids.1.lock().unwrap().cached.upgrade().unwrap();
        assert!(Arc::ptr_eq(&ga, &gb));
        let weak = Arc::downgrade(&ga);
        drop(ga);
        drop(gb);
        drop(a);
        drop(b);
        assert!(weak.upgrade().is_some());
        drop(budget);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn independent_concurrent_curves_admit_one_exact_support() {
        let budget = Arc::new(MeshBudget::new(1));
        let expected = bits(curve(0.).try_subs(0.4));
        std::thread::scope(|scope| {
            for _ in 0..12 {
                let b = budget.clone();
                scope.spawn(move || assert_eq!(bits(MeshCurve::new(curve(0.), b).try_subs(0.4)), expected));
            }
        });
        assert_eq!(budget.cells.lock().unwrap().len(), 1);
    }
    #[test]
    fn key_retains_signed_zero_nan_payload_and_surface_kind() {
        let make = |x: f64| {
            mt::Surface::BSplineSurface(mt::BSplineSurface::new(
                (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
                vec![vec![mt::Point3::new(x, 0., 0.), mt::Point3::new(0., 1., 0.)], vec![mt::Point3::new(1., 0., 0.), mt::Point3::new(1., 1., 0.)]],
            ))
        };
        assert_ne!(SurfaceKey::new(&make(0.)), SurfaceKey::new(&make(-0.)));
        assert_ne!(SurfaceKey::new(&make(f64::from_bits(0x7ff8000000000001))), SurfaceKey::new(&make(f64::from_bits(0x7ff8000000000002))));
        let rational = rational([[1., 1.], [1., 1.]]);
        assert_ne!(SurfaceKey::new(&make(0.)), SurfaceKey::new(&rational));
    }
    #[test]
    fn oversized_key_preserves_original_fallback() {
        let surface = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(199), mt::KnotVec::bezier_knot(1)),
            (0..200).map(|i| vec![mt::Point3::new(i as f64, 0., 0.), mt::Point3::new(i as f64, 1., 0.)]).collect(),
        ));
        assert!(SurfaceKey::new(&surface).is_none());
        let budget = MeshBudget::new(1);
        assert!(matches!(budget.cell(&surface), Admission::Uncacheable));
        assert!(budget.cells.lock().unwrap().is_empty());
    }

    #[test]
    fn default_budget_recycles_only_after_distinct_supports_exceed_capacity() {
        let budget = Arc::new(MeshBudget::new(MAX_GRIDS));
        for i in 0..=MAX_GRIDS {
            let raw = curve(i as f64 * 3.);
            let adapted = MeshCurve::new(raw.clone(), budget.clone());
            assert_eq!(bits(raw.try_subs(0.4)), bits(adapted.try_subs(0.4)));
            assert!(budget.stats().0 <= MAX_GRIDS);
            if i < MAX_GRIDS {
                assert_eq!(budget.stats().2, 0);
            }
        }
        assert_eq!(budget.stats().1, MAX_GRIDS);
        assert_eq!(budget.stats().2, 1);
        let slots = budget.slots.clone();
        drop(budget);
        assert_eq!(slots.live.load(Ordering::Acquire), 0);
    }
    #[test]
    fn recency_counter_wrap_keeps_samples_and_slot_limit() {
        let budget = Arc::new(MeshBudget::new(1));
        budget.clock.store(usize::MAX, Ordering::Relaxed);
        for offset in [0., 10., 0.] {
            let raw = curve(offset);
            let adapted = MeshCurve::new(raw.clone(), budget.clone());
            assert_eq!(bits(raw.try_subs(0.4)), bits(adapted.try_subs(0.4)));
        }
        assert_eq!(budget.stats().1, 1);
    }
    #[test]
    fn poisoned_local_state_keeps_original_projection() {
        let budget = Arc::new(MeshBudget::new(1));
        let raw = curve(0.);
        let adapted = MeshCurve::new(raw.clone(), budget.clone());
        let clone = adapted.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = clone.grids.1.lock().unwrap();
                panic!("simulated local state panic");
            })
            .join()
            .is_err()
        );
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            assert_eq!(bits(raw.try_subs(t)), bits(adapted.try_subs(t)));
        }
        assert_eq!(budget.stats().0, 0);
    }
    #[test]
    fn oversized_local_support_keeps_original_projection_without_a_grid() {
        let p = mt::Point3::new;
        let mut knots = vec![0., 0.];
        knots.extend((1..199).map(|i| i as f64 / 199.));
        knots.extend([1., 1.]);
        let knots = mt::KnotVec::from(knots);
        let points: Vec<_> = (0..200)
            .map(|i| {
                let t = i as f64 / 199.;
                p(t, t * t, 0.)
            })
            .collect();
        let support = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (knots.clone(), mt::KnotVec::bezier_knot(1)),
            points.iter().map(|q| vec![p(q.x, q.y, -1.), p(q.x, q.y, 1.)]).collect(),
        ));
        assert!(SurfaceKey::new(&support).is_none());
        let plane = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.)));
        let leader = mt::Leader::BSpline(mt::BSplineCurve::new(knots, points));
        let raw = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(plane), Box::new(support), leader, 1e-8));
        let budget = Arc::new(MeshBudget::new(1));
        let adapted = MeshCurve::new(raw.clone(), budget.clone());
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            let expected = raw.try_subs(t);
            assert!(expected.is_some());
            assert_eq!(bits(expected), bits(adapted.try_subs(t)));
        }
        assert!(adapted.grids.1.lock().unwrap().disabled);
        assert_eq!(budget.stats().0, 0);
    }
    fn support(offset: f64) -> mt::Surface {
        let mt::Curve::IntersectionCurve(c) = curve(offset) else { unreachable!() };
        c.surface1().clone()
    }
    fn admitted(budget: &MeshBudget, offset: f64) -> SharedGrid {
        match budget.cell(&support(offset)) {
            Admission::Shared(c) => c,
            _ => panic!("expected available slot"),
        }
    }
    #[test]
    fn active_entry_blocks_admission_until_released() {
        let budget = MeshBudget::new(1);
        let first = admitted(&budget, 0.);
        assert!(matches!(budget.cell(&support(10.)), Admission::Unavailable));
        assert_eq!(budget.stats().0, 1);
        let weak = Arc::downgrade(&first);
        drop(first);
        let second = admitted(&budget, 10.);
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.stats().1, 1);
        drop(second);
        let slots = budget.slots.clone();
        drop(budget);
        assert_eq!(slots.live.load(Ordering::Acquire), 0);
    }
    #[test]
    fn evicted_weak_local_reacquires_exact_grid() {
        let budget = Arc::new(MeshBudget::new(1));
        let a = MeshCurve::new(curve(0.), budget.clone());
        let b = MeshCurve::new(curve(10.), budget.clone());
        for _ in 0..8 {
            for c in [&a, &b] {
                assert_eq!(bits(c.try_subs(0.4)), bits(c.original.try_subs(0.4)));
            }
        }
        assert_eq!(budget.stats().1, 1);
        assert!(budget.stats().2 >= 15);
    }
    #[test]
    fn concurrent_distinct_curves_never_exceed_live_slot_bound() {
        let budget = Arc::new(MeshBudget::new(2));
        std::thread::scope(|scope| {
            for i in 0..12 {
                let b = budget.clone();
                scope.spawn(move || {
                    let raw = curve(i as f64 * 3.);
                    let adapted = MeshCurve::new(raw.clone(), b.clone());
                    for j in 0..8 {
                        let t = j as f64 / 8.;
                        assert_eq!(bits(raw.try_subs(t)), bits(adapted.try_subs(t)));
                        assert!(b.stats().0 <= 2);
                    }
                });
            }
        });
        assert!(budget.stats().1 <= 2);
        let slots = budget.slots.clone();
        drop(budget);
        assert_eq!(slots.live.load(Ordering::Acquire), 0);
    }
    #[test]
    fn orphaned_active_entry_retains_permit_after_map_removal() {
        let budget = MeshBudget::new(1);
        let first = admitted(&budget, 0.);
        budget.cells.lock().unwrap().clear();
        assert!(matches!(budget.cell(&support(10.)), Admission::Unavailable));
        assert_eq!(budget.stats().0, 1);
        drop(first);
        let second = admitted(&budget, 10.);
        assert_eq!(budget.stats().1, 1);
        drop(second);
    }

    #[test]
    fn panicking_initializer_can_retry_and_drop_releases_permit() {
        let budget = MeshBudget::new(1);
        let entry = admitted(&budget, 0.);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            entry.grid.get_or_init(|| panic!("simulated preparation failure"));
        }));
        assert!(result.is_err());
        assert!(entry.grid.get().is_none());
        assert!(entry.grid.get_or_init(|| Grid::prepare(&support(0.))).is_some());
        let slots = budget.slots.clone();
        drop(entry);
        drop(budget);
        assert_eq!(slots.live.load(Ordering::Acquire), 0);
    }
    #[test]
    fn poisoned_map_falls_back_and_releases_resident_permit_on_drop() {
        let budget = Arc::new(MeshBudget::new(1));
        drop(admitted(&budget, 0.));
        let clone = budget.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = clone.cells.lock().unwrap();
                panic!("simulated map panic");
            })
            .join()
            .is_err()
        );
        assert!(matches!(budget.cell(&support(10.)), Admission::Unavailable));
        let raw = curve(10.);
        let prepared = MeshCurve::new(raw.clone(), budget.clone());
        assert_eq!(bits(raw.try_subs(0.4)), bits(prepared.try_subs(0.4)));
        let slots = budget.slots.clone();
        drop(prepared);
        drop(budget);
        assert_eq!(slots.live.load(Ordering::Acquire), 0);
    }
}

#[cfg(test)]
#[path = "mesh_curve_sampler_mesh_tests.rs"]
mod mesh_tests;
