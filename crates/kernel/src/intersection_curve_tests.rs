//! Regressions against the actual patched truck constructor, run by workspace CI.
use solidcraft_geom::Vec3;
use truck_meshalgo::prelude::PolylineCurve;
use truck_modeling::{BoundedCurve, ParametricCurve, Plane, Point3, Sphere};
use truck_shapeops::IntersectionCurveWithParameters;

fn planes() -> (Plane, Plane) {
    let o = Point3::new(0.0, 0.0, 0.0);
    (Plane::new(o, Point3::new(1.0, 0.0, 0.0), Point3::new(0.0, 1.0, 0.0)), Plane::new(o, Point3::new(1.0, 0.0, 0.0), Point3::new(0.0, 0.0, 1.0)))
}

#[test]
fn degenerate_intersection_polylines_return_none_without_panicking() {
    let origin = Point3::new(0.0, 0.0, 0.0);
    for points in [
        vec![],
        vec![origin],
        vec![origin, origin],
        vec![origin; 3],
        vec![origin, Point3::new(f64::NAN, 0.0, 0.0)],
        vec![origin, Point3::new(f64::INFINITY, 0.0, 0.0)],
    ] {
        let (a, b) = planes();
        let result = std::panic::catch_unwind(|| IntersectionCurveWithParameters::try_new(a, b, PolylineCurve(points.clone()), 0.01));
        assert!(result.is_ok(), "constructor panicked for {points:?}");
        assert!(result.unwrap().is_none(), "collapsed input must not create a curve: {points:?}");
    }
}

#[test]
fn closed_intersection_retains_the_closure_and_spherical_supports() {
    let a = Sphere::new(Point3::new(0.0, 0.0, 1.0), 2.0_f64.sqrt());
    let b = Sphere::new(Point3::new(0.0, 0.0, -1.0), 2.0_f64.sqrt());
    let poly = PolylineCurve(vec![
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(-1.0, 0.0, 0.0),
        Point3::new(0.0, -1.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
    ]);
    let curve = IntersectionCurveWithParameters::try_new(a, b, poly, 0.01).unwrap();
    let (lo, hi) = curve.range_tuple();
    assert_eq!(curve.subs(lo), curve.subs(hi));
    for i in 0..=40 {
        let p = curve.subs(lo + (hi - lo) * i as f64 / 40.0);
        assert!(p.z.abs() < 1e-8, "{p:?}");
        assert!(((p.x * p.x + p.y * p.y).sqrt() - 1.0).abs() <= 0.01, "{p:?}");
    }
}

#[test]
fn open_intersection_keeps_endpoints_on_both_planes() {
    let (a, b) = planes();
    let start = Point3::new(1.0, 0.0, 0.0);
    let end = Point3::new(3.0, 0.0, 0.0);
    let curve = IntersectionCurveWithParameters::try_new(a, b, PolylineCurve(vec![start, end]), 0.01).unwrap();
    let (lo, hi) = curve.range_tuple();
    assert_eq!(curve.subs(lo), start);
    assert_eq!(curve.subs(hi), end);
    assert_eq!(curve.subs((lo + hi) / 2.0), Point3::new(2.0, 0.0, 0.0));
}

/// A triangular face whose first edge is an intersection curve of two planes: the parallel planes
/// `z = 0` and `z = 1` when `support_gap` is `1.0` (the projection onto both never converges), the
/// same plane twice when it is `0.0` (the leader lies on both and always evaluates).
fn triangle_with_intersection_edge(support_gap: f64) -> truck_modeling::Shell {
    use truck_modeling::{Curve, Edge, Face, IntersectionCurve, Leader, Line, Shell, Surface, Vertex, Wire};
    let plane = |z: f64| Surface::Plane(Plane::new(Point3::new(0.0, 0.0, z), Point3::new(1.0, 0.0, z), Point3::new(0.0, 1.0, z)));
    let (v0, v1, v2) = (Vertex::new(Point3::new(0.0, 0.0, 0.5)), Vertex::new(Point3::new(1.0, 0.0, 0.5)), Vertex::new(Point3::new(0.5, 1.0, 0.5)));
    let leader = Leader::Polyline(PolylineCurve(vec![Point3::new(0.0, 0.0, 0.5), Point3::new(1.0, 0.0, 0.5)]));
    let bad = IntersectionCurve::new_unchecked(Box::new(plane(0.5 - support_gap / 2.0)), Box::new(plane(0.5 + support_gap / 2.0)), leader, 0.01);
    let line = |a: &Vertex, b: &Vertex| Edge::new(a, b, Curve::Line(Line(a.point(), b.point())));
    let wire = Wire::from(vec![Edge::new(&v0, &v1, Curve::IntersectionCurve(bad)), line(&v1, &v2), line(&v2, &v0)]);
    Shell::from(vec![Face::new(vec![wire], plane(0.5))])
}

/// Meshing an edge whose intersection curve does not evaluate (#1252): `IntersectionCurve::subs`
/// unwraps a failed projection, and the mesher sampled the edge through it. The two supports here
/// are parallel planes, so the projection never converges. Nothing may panic (wasm aborts on one,
/// and the native guard prints the message first) and the mesh stays finite.
#[test]
fn meshing_an_edge_that_does_not_evaluate_neither_panics_nor_goes_non_finite() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use truck_meshalgo::prelude::RobustMeshableShape;
    let shell = triangle_with_intersection_edge(1.0);

    let me = std::thread::current().id();
    let count = Arc::new(AtomicUsize::new(0));
    let c2 = count.clone();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |_| {
        if std::thread::current().id() == me {
            c2.fetch_add(1, Ordering::SeqCst);
        }
    }));
    let meshed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| shell.robust_triangulation(0.01)));
    std::panic::set_hook(prev);
    assert_eq!(count.load(Ordering::SeqCst), 0, "panics while meshing");
    let meshed = meshed.unwrap();
    for face in meshed.face_iter() {
        if let Some(mesh) = face.surface() {
            assert!(mesh.positions().iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()));
        }
    }
}

/// A boolean's result with an edge that does not evaluate is refused with an error (#1252). The
/// mesher's panic on such an edge used to reject the attempt; now that it no longer panics, the
/// result must be refused explicitly or a plausible-looking mesh would let it through.
#[test]
fn a_result_with_an_edge_that_does_not_evaluate_is_refused() {
    use truck_modeling::Solid;
    let solid = |gap: f64| Solid::new_unchecked(vec![triangle_with_intersection_edge(gap)]);
    assert!(!crate::boolean_record::edges_evaluate(&solid(1.0)));
    assert!(crate::boolean_record::edges_evaluate(&solid(0.0)));
}

/// A small biquadratic B-spline patch, 2 mm by 0.75 mm, that the plane `z = 0` cuts along the
/// arc `0.2 x² + (y - 0.1)² = 0.25`: the patch is `z = 0.2 x² + (y - 0.1)² - 0.25` over
/// `x ∈ [-1, 1]`, `y ∈ [0, 0.75]`, and the plane is a large one (a metre across).
fn small_patch_and_large_plane() -> (truck_modeling::Surface, truck_modeling::Surface) {
    use truck_modeling::{BSplineSurface, KnotVec, Surface};
    // Bezier ordinates of the quadratics g(x(u)) and h(y(v)): the values at the ends, and twice
    // the middle value less the mean of the ends in between.
    let ordinates = |f: &dyn Fn(f64) -> f64, a: f64, b: f64| [f(a), 2.0 * f((a + b) / 2.0) - (f(a) + f(b)) / 2.0, f(b)];
    let (xs, ys) = ([-1.0, 0.0, 1.0], [0.0, 0.375, 0.75]);
    let g = ordinates(&|x| 0.2 * x * x, -1.0, 1.0);
    let h = ordinates(&|y| (y - 0.1) * (y - 0.1) - 0.25, 0.0, 0.75);
    let control = (0..3).map(|i| (0..3).map(|j| Point3::new(xs[i], ys[j], g[i] + h[j])).collect()).collect();
    let patch = BSplineSurface::new((KnotVec::bezier_knot(2), KnotVec::bezier_knot(2)), control);
    let plane = Plane::new(Point3::new(-500.0, -500.0, 0.0), Point3::new(500.0, -500.0, 0.0), Point3::new(-500.0, 500.0, 0.0));
    (Surface::Plane(plane), Surface::BSplineSurface(patch))
}

/// The point of the arc above `x`.
fn on_arc(x: f64) -> Point3 {
    Point3::new(x, 0.1 + (0.25 - 0.2 * x * x).sqrt(), 0.0)
}

/// The polyline leading an intersection curve is where the faces' meshes meet, so it can run up
/// to the mesh tolerance off the surfaces, and on a face that is small beside that tolerance it
/// can step out across the curve and back. At such a step the plane across the polyline runs
/// along the curve, beside it: truck's projection found no point there, and the whole curve was
/// refused (the boolean with it). The point is now taken across the curve itself, and the curve
/// is built with every point on both surfaces, in order along it.
#[test]
fn a_plane_cut_through_a_small_b_spline_patch_survives_a_polyline_stepping_across_the_curve() {
    use truck_modeling::{MetricSpace, ParametricSurface, SearchNearestParameter};
    let (plane, patch) = small_patch_and_large_plane();
    // Along the arc, except one point 0.08 mm outside it, from which the polyline comes back
    // straight across the arc (along its normal there).
    let normal = {
        let d = -0.2 * 0.5 / (on_arc(0.5).y - 0.1);
        truck_modeling::Vector3::new(-d, 1.0, 0.0) / (1.0 + d * d).sqrt()
    };
    let mut leader: Vec<Point3> = [-0.8, -0.4, 0.0, 0.3].into_iter().map(on_arc).collect();
    leader.extend([on_arc(0.5) + normal * 0.08, on_arc(0.5), on_arc(0.7), on_arc(0.9)]);
    let curve = IntersectionCurveWithParameters::try_new(plane.clone(), patch.clone(), PolylineCurve(leader.clone()), 0.05)
        .expect("the leader point across the curve refused the whole curve");
    let points = &curve.leader().0;
    assert_eq!(points.len(), leader.len());
    for p in points {
        for surface in [&plane, &patch] {
            let (u, v) = surface.search_nearest_parameter(*p, None, 100).unwrap();
            assert!(surface.subs(u, v).distance(*p) < 1e-6, "{p:?} is off a surface");
        }
        assert!((0.2 * p.x * p.x + (p.y - 0.1) * (p.y - 0.1) - 0.25).abs() < 1e-6, "{p:?} is off the arc");
    }
    // The point stepped out to came back next to where it left the arc, not elsewhere along it.
    assert!(points[4].distance(leader[4]) < 0.1, "{:?} moved too far", points[4]);
    assert!(points.windows(2).all(|w| w[0].x <= w[1].x + 0.01), "out of order along the arc: {points:?}");
}

/// A 10 mm cube at `at` whose faces lie up to `gap` off its edges, alternately in and out (as the
/// faces of a STEP part meet its edges only to the file's tolerance).
fn cube_with_loose_faces(at: truck_modeling::Vector3, gap: f64) -> truck_modeling::Solid {
    use truck_modeling::{EuclideanSpace, Face, InnerSpace, Shell, Solid, Surface, Vector3, Vertex, builder};
    let v = |x: f64, y: f64| builder::vertex(Point3::new(x, y, 0.0) + at);
    let p: Vec<Vertex> = vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0), v(0.0, 10.0)];
    let wire: truck_modeling::Wire = (0..4).map(|i| builder::line(&p[i], &p[(i + 1) % 4])).collect();
    let bottom = builder::try_attach_plane(&[wire.inverse()]).unwrap();
    let cube = builder::tsweep(&bottom, Vector3::new(0.0, 0.0, 10.0));
    let faces: Vec<Face> = cube.boundaries()[0]
        .face_iter()
        .enumerate()
        .map(|(i, face)| {
            let Surface::Plane(plane) = face.surface() else { panic!("a face of the cube is not a plane") };
            let n = plane.normal() * if i % 2 == 0 { gap } else { -gap };
            let o = plane.origin().to_vec() + n;
            let moved = truck_modeling::Plane::new(Point3::from_vec(o), Point3::from_vec(o + plane.u_axis()), Point3::from_vec(o + plane.v_axis()));
            assert!(n.magnitude() <= gap * (1.0 + 1e-9));
            let mut f = Face::new(face.absolute_boundaries().clone(), Surface::Plane(moved));
            if !face.orientation() {
                f.invert();
            }
            f
        })
        .collect();
    Solid::new(vec![Shell::from(faces)])
}

/// A plate cut across a cube whose faces meet its edges only to within 2e-6 mm: each face is
/// meshed on its own surface, so two faces' meshes part along their common edge by that much,
/// and so do the ends of the intersection curves that cross it. Such an end was found on no edge
/// of the cube's faces and on no end of the next curve on the plate's (both wanted 1e-6 mm), the
/// faces kept no loop and the result's shell stayed open. A point a thousandth of the tolerance
/// off an edge or a curve's end is now on it.
#[test]
fn a_plate_across_a_part_whose_faces_meet_its_edges_loosely_closes() {
    use truck_modeling::Vector3;
    let at = Vector3::new(0.0, 0.0, 0.0);
    let gap = 2e-6;
    let part = crate::Body::new(cube_with_loose_faces(at, gap)).unwrap();
    let plate = crate::box_solid(Vec3::new(-20.0, -20.0, 4.0), Vec3::new(30.0, 30.0, 6.0)).unwrap();
    let tol = plate.size().max(part.size()) * 2e-3;
    let (solid, _) = crate::ops::truck_boolean(&plate, &part, crate::BoolOp::Intersect, None, tol).expect("the shell of the shared part is closed");
    let body = crate::Body::new(solid).unwrap();
    let v = crate::measure(&body).unwrap().volume;
    assert!((v - 200.0).abs() < 0.01, "{v}");
}
