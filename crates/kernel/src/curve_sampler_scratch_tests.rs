use super::*;
use mt::{BoundedCurve, Invertible, Transformed};

#[test]
fn sampler_and_support_destructors_pin_the_scratch_loan() {
    // Inherited resource drops do not retain a borrowed lifetime after last use.
    // Both explicit destructors are required by this API's memory contract.
    #[allow(drop_bounds)]
    fn require_lease_destructor<T: Drop>() {}
    require_lease_destructor::<CurveSampler<'static>>();
    require_lease_destructor::<PreparedSurface<'static>>();
}

fn point_bits(p: Option<mt::Point3>) -> Option<[u64; 3]> {
    p.map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
}

fn spline(vertical: bool, offset: f64) -> mt::Surface {
    let p = mt::Point3::new;
    let points = if vertical {
        vec![vec![p(offset, 0., -1.), p(offset, 0., 1.)], vec![p(offset + 1., 0., -1.), p(offset + 1., 0., 1.)]]
    } else {
        vec![vec![p(offset, -1., 0.), p(offset, 1., 0.)], vec![p(offset + 1., -1., 0.), p(offset + 1., 1., 0.)]]
    };
    mt::Surface::BSplineSurface(mt::BSplineSurface::new((mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)), points))
}

fn crossing() -> mt::Curve {
    let p = mt::Point3::new;
    let leader = mt::Leader::BSpline(mt::BSplineCurve::new(mt::KnotVec::bezier_knot(1), vec![p(0., 0., 0.), p(1., 0., 0.)]));
    mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(spline(false, 0.)), Box::new(spline(true, 0.)), leader, 1e-8))
}

fn check_samples(curve: &mt::Curve, scratch: &mut GridScratch) {
    let sampler = CurveSampler::with_scratch(curve, scratch);
    let (lo, hi) = curve.range_tuple();
    for k in 0..=32 {
        let t = lo + (hi - lo) * k as f64 / 32.;
        let expected = curve.try_subs(t);
        assert!(expected.is_some(), "sample {k}");
        assert_eq!(point_bits(sampler.at(t)), point_bits(expected));
    }
}

#[test]
fn consecutive_curves_reuse_both_supports_and_reversed_samples() {
    let curve = crossing();
    let mut scratch = GridScratch::new();
    let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
    check_samples(&curve, &mut scratch);
    let mut reversed = curve.clone();
    reversed.invert();
    check_samples(&reversed, &mut scratch);
    assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2 * 2601);
    assert_eq!(scratch.entries.lock().unwrap().len(), 2);
    fn require_send<T: Send>() {}
    require_send::<CurveSampler<'static>>();
}

#[test]
fn third_support_recycles_before_preparation_and_retains_seed_bits() {
    let scratch = GridScratch::new();
    let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
    for offset in [0., 2., 4., 0.] {
        let surface = spline(false, offset);
        let key = Arc::new(SurfaceKey::new(&surface).unwrap());
        let point = mt::Point3::new(offset + 0.3, 0.4, 0.2);
        let expected = mt::algo::surface::presearch(&surface, point, ((0., 1.), (0., 1.)), 50);
        let actual = scratch.seed(&key, &surface, point).unwrap();
        assert_eq!((actual.0.to_bits(), actual.1.to_bits()), (expected.0.to_bits(), expected.1.to_bits()));
        let entries = scratch.entries.lock().unwrap();
        assert!(entries.len() <= 2);
        assert!(entries.iter().all(|(_, grid)| grid.points.len() == 2601 && grid.u.len() == 51 && grid.v.len() == 51));
    }
    assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 4 * 2601);
}

#[test]
fn transformed_and_weighted_supports_keep_projection_bits() {
    let p = mt::Point3::new;
    let controls = vec![
        vec![mt::Vector4::new(0., 0., -1., 1.), mt::Vector4::new(0., 0., 2., 2.)],
        vec![mt::Vector4::new(0.5, 0., -0.5, 0.5), mt::Vector4::new(3., 0., 3., 3.)],
    ];
    let rational = mt::Surface::NurbsSurface(mt::NurbsSurface::new(mt::BSplineSurface::new(
        (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
        controls,
    )));
    let leader = mt::Leader::BSpline(mt::BSplineCurve::new(mt::KnotVec::bezier_knot(1), vec![p(0.1, 0., 0.), p(0.9, 0., 0.)]));
    let curve = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(spline(false, 0.)), Box::new(rational), leader, 1e-8));
    let mut scratch = GridScratch::new();
    for transform in [
        mt::Matrix4::from_scale(1.),
        mt::Matrix4::from_translation(mt::Vector3::new(3., -2., 1.)),
        mt::Matrix4::from_nonuniform_scale(-2., 3., 0.5),
        mt::Matrix4::from_angle_z(mt::Rad(0.37)),
    ] {
        check_samples(&curve.transformed(transform), &mut scratch);
        assert_eq!(scratch.entries.lock().unwrap().len(), 2);
    }
}

#[test]
fn explicit_hints_conflicting_lock_and_clones_keep_original_search() {
    let surface = spline(false, 0.);
    let scratch = GridScratch::new();
    let mut prepared = PreparedSurface::new(&surface);
    prepared.shared = Some((&scratch, Arc::new(SurfaceKey::new(&surface).unwrap())));
    let cloned = prepared.clone();
    let point = mt::Point3::new(0.3, 0.4, 0.2);
    let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
    for hint in [mt::SPHint2D::Parameter(0.2, 0.7), mt::SPHint2D::Range((0.8, 0.1), (0.9, 0.2))] {
        assert_eq!(cloned.search_nearest_parameter(point, hint, 50), surface.search_nearest_parameter(point, hint, 50));
    }
    let guard = scratch.entries.lock().unwrap();
    assert_eq!(cloned.search_nearest_parameter(point, None, 50), surface.search_nearest_parameter(point, None, 50));
    assert!(cloned.grid.borrow().is_none());
    assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get), before);
    drop(guard);
    assert_eq!(cloned.search_nearest_parameter(point, None, 50), surface.search_nearest_parameter(point, None, 50));
    assert_eq!(scratch.entries.lock().unwrap().len(), 1);
}

#[test]
fn poisoned_scratch_preserves_projection_and_original_failure_results() {
    let mut scratch = GridScratch::new();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = scratch.entries.lock().unwrap();
        panic!("simulate interrupted grid access");
    }));
    let curve = crossing();
    let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
    check_samples(&curve, &mut scratch);
    assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get), before);

    let p = mt::Point3::new;
    let leader = mt::Leader::BSpline(mt::BSplineCurve::new(
        mt::KnotVec::bezier_knot(3),
        vec![p(0., 0., 0.), p(0.25, 4. / 3., 0.), p(0., 4. / 3., 0.), p(0.25, 0., 0.)],
    ));
    let invalid =
        mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(spline(false, 0.)), Box::new(spline(true, 0.)), leader, 0.01));
    let sampler = CurveSampler::with_scratch(&invalid, &mut scratch);
    assert!(invalid.try_subs(0.).is_some());
    assert!(invalid.try_subs(0.5).is_none());
    for k in 0..=32 {
        let t = k as f64 / 32.;
        assert_eq!(point_bits(sampler.at(t)), point_bits(invalid.try_subs(t)));
    }
}

fn oversized_curve(two_splines: bool, invalid: bool) -> mt::Curve {
    let p = mt::Point3::new;
    let mut knots = vec![0., 0.];
    knots.extend((1..199).map(|i| i as f64 / 199.));
    knots.extend([1., 1.]);
    let knots = mt::KnotVec::from(knots);
    let points: Vec<_> = (0..200)
        .map(|i| {
            let t = i as f64 / 199.;
            p(t, if invalid { 0. } else { t * t }, 0.)
        })
        .collect();
    let support = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
        (knots.clone(), mt::KnotVec::bezier_knot(1)),
        points.iter().map(|q| vec![p(q.x, q.y, -1.), p(q.x, q.y, 1.)]).collect(),
    ));
    assert!(SurfaceKey::new(&support).is_none());
    let plane = if two_splines { spline(false, 0.) } else { mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.))) };
    let leader = if invalid {
        mt::Leader::BSpline(mt::BSplineCurve::new(
            mt::KnotVec::bezier_knot(3),
            vec![p(0., 0., 0.), p(0.25, 4. / 3., 0.), p(0., 4. / 3., 0.), p(0.25, 0., 0.)],
        ))
    } else {
        mt::Leader::BSpline(mt::BSplineCurve::new(knots, points))
    };
    mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(plane), Box::new(support), leader, 1e-8))
}

#[test]
fn oversized_support_clears_shared_data_then_reuses_local_grid() {
    let curve = oversized_curve(false, false);
    let mut scratch = GridScratch::new();
    check_samples(&crossing(), &mut scratch);
    assert_eq!(scratch.entries.lock().unwrap().len(), 2);
    let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
    check_samples(&curve, &mut scratch);
    assert!(scratch.entries.lock().unwrap().is_empty());
    assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2601);
}

fn bilinear_face(height: f64, inverse: bool) -> mt::Face {
    let p = mt::Point3::new;
    let surface = mt::Surface::BSplineSurface(mt::BSplineSurface::new(
        (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
        vec![vec![p(0., 0., 0.), p(0., 1., 0.)], vec![p(1., 0., 0.), p(1., 1., height)]],
    ));
    let points = [p(0., 0., 0.), p(1., 0., 0.), p(1., 1., height), p(0., 1., 0.)];
    let vertices: Vec<_> = points.iter().copied().map(mt::Vertex::new).collect();
    let edges: Vec<_> = (0..4)
        .map(|i| {
            let j = (i + 1) % 4;
            let side = mt::Surface::Plane(mt::Plane::new(points[i], points[j], points[i] + mt::Vector3::unit_z()));
            let leader = mt::Leader::BSpline(mt::BSplineCurve::new(mt::KnotVec::bezier_knot(1), vec![points[i], points[j]]));
            let curve = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(surface.clone()), Box::new(side), leader, 1e-8));
            mt::Edge::new_unchecked(&vertices[i], &vertices[j], curve)
        })
        .collect();
    let mut face = mt::Face::try_new(vec![edges.into()], surface).unwrap();
    if inverse {
        face.invert();
    }
    face
}

#[test]
fn planar_conversion_reuses_one_grid_and_keeps_boundary_identities_and_orientation() {
    for inverse in [false, true] {
        let face = bilinear_face(0., inverse);
        let original_ids: Vec<_> = face.edge_iter().map(|e| e.id()).collect();
        let shell: mt::Shell = vec![face.clone()].into();
        let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        let converted = crate::heal::planar_surfaces(&shell, 1e-8).unwrap();
        let result = converted.face_iter().next().unwrap();
        assert!(matches!(result.surface(), mt::Surface::Plane(_)));
        assert_eq!(result.orientation(), face.orientation());
        assert_eq!(result.edge_iter().map(|e| e.id()).collect::<Vec<_>>(), original_ids);
        assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2601);
    }
}

#[test]
fn nonplanar_surface_is_preserved_with_all_boundary_samples() {
    let shell: mt::Shell = vec![bilinear_face(0.3, false)].into();
    let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
    assert!(crate::heal::planar_surfaces(&shell, 1e-8).is_none());
    assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2601);
}

#[test]
fn oversized_support_clones_retain_only_two_grids_and_release_them_before_reuse() {
    for invalid in [false, true] {
        let curve = oversized_curve(true, invalid);
        let mut scratch = GridScratch::new();
        check_samples(&crossing(), &mut scratch);
        let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        let (weak_a, weak_b) = {
            let sampler = CurveSampler::with_scratch(&curve, &mut scratch);
            let (a, b) = sampler.supports.as_ref().unwrap();
            let ca = a.clone();
            let cb = b.clone();
            assert!(Arc::ptr_eq(a.local_shared.as_ref().unwrap(), ca.local_shared.as_ref().unwrap()));
            assert!(Arc::ptr_eq(b.local_shared.as_ref().unwrap(), cb.local_shared.as_ref().unwrap()));
            let mt::Curve::IntersectionCurve(intersection) = &curve else { panic!("fixture must be an intersection curve") };
            let mut failures = 0;
            for k in 0..=32 {
                let t = k as f64 / 32.;
                let expected = curve.try_subs(t);
                failures += usize::from(expected.is_none());
                assert_eq!(point_bits(sampler.at(t)), point_bits(expected));
                let cloned = mt::double_projection(&ca, None, &cb, None, intersection.leader().subs(t), intersection.leader().der(t), 100)
                    .map(|triple| triple.0);
                assert_eq!(point_bits(cloned), point_bits(expected));
            }
            assert_eq!(failures > 0, invalid);
            assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get) - before, 2 * 2601);
            (Arc::downgrade(a.local_shared.as_ref().unwrap()), Arc::downgrade(b.local_shared.as_ref().unwrap()))
        };
        // Keeping these observations does not retain either grid allocation.
        assert!(weak_a.upgrade().is_none() && weak_b.upgrade().is_none());
        assert!(scratch.entries.lock().unwrap().is_empty());
        check_samples(&crossing(), &mut scratch);
        assert_eq!(scratch.entries.lock().unwrap().len(), 2);
    }
}

#[test]
fn poisoned_oversized_fallback_clones_keep_original_projection_results() {
    let curve = oversized_curve(true, false);
    let mut scratch = GridScratch::new();
    {
        let sampler = CurveSampler::with_scratch(&curve, &mut scratch);
        let (a, b) = sampler.supports.as_ref().unwrap();
        let ca = a.clone();
        let cb = b.clone();
        assert!(sampler.at(0.5).is_some());
        let before = GRID_POINTS_PREPARED.with(std::cell::Cell::get);
        let local = Arc::clone(a.local_shared.as_ref().unwrap());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = local.lock().unwrap();
            panic!("simulate interrupted local fallback access");
        }));
        let mt::Curve::IntersectionCurve(intersection) = &curve else { panic!("fixture must be an intersection curve") };
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            let expected = curve.try_subs(t);
            assert_eq!(point_bits(sampler.at(t)), point_bits(expected));
            let cloned =
                mt::double_projection(&ca, None, &cb, None, intersection.leader().subs(t), intersection.leader().der(t), 100).map(|triple| triple.0);
            assert_eq!(point_bits(cloned), point_bits(expected));
        }
        assert_eq!(GRID_POINTS_PREPARED.with(std::cell::Cell::get), before);
    }
    check_samples(&crossing(), &mut scratch);
    assert_eq!(scratch.entries.lock().unwrap().len(), 2);
}
