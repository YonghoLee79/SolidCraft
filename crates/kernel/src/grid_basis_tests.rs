//! Independent vendor-reference checks for bounded spline coefficient reuse.
use crate::grid_basis::batched;
use truck_modeling::{self as mt, BoundedSurface, ParametricSurface};

fn coordinates(surface: &mt::Surface) -> (Vec<f64>, Vec<f64>) {
    let mt::Surface::BSplineSurface(s) = surface else { panic!("B-spline fixture required") };
    let ((u0, u1), (v0, v1)) = s.range_tuple();
    let range = |lo: f64, hi: f64| {
        (0..=50)
            .map(|i| {
                let p = i as f64 / 50.;
                lo * (1. - p) + hi * p
            })
            .collect()
    };
    (range(u0, u1), range(v0, v1))
}
fn compare(surface: &mt::Surface) {
    let (u, v) = coordinates(surface);
    let points = batched(surface, &u, &v).expect("small fixture must use bounded basis cache");
    let bits = |p: mt::Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let ordinary: Vec<_> = u.iter().flat_map(|&a| v.iter().map(move |&b| bits(surface.subs(a, b)))).collect();
    assert_eq!(points.into_iter().map(bits).collect::<Vec<_>>(), ordinary);
}
fn bezier(degree: usize, shift: f64) -> mt::Surface {
    let points =
        (0..=degree).map(|i| (0..=degree).map(|j| mt::Point3::new(shift + i as f64, -shift + j as f64, (i * j) as f64 * 0.13)).collect()).collect();
    mt::Surface::BSplineSurface(mt::BSplineSurface::new((mt::KnotVec::bezier_knot(degree), mt::KnotVec::bezier_knot(degree)), points))
}
#[test]
fn small_bezier_grids_match_every_vendor_point_bit() {
    for degree in 1..=3 {
        for shift in [0., 1e12, -1e-12] {
            compare(&bezier(degree, shift));
        }
    }
}
#[test]
fn repeated_knots_and_shifted_parameter_ranges_keep_vendor_bits() {
    for knots in [vec![-2., -2., -2., 0., 3., 3., 3.], vec![0., 0., 0.5, 0.5, 1., 1.]] {
        let p = mt::Point3::new;
        let points = (0..4).map(|i| (0..3).map(|j| p(i as f64, j as f64, (i * j) as f64 * 0.17)).collect()).collect();
        compare(&mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (mt::KnotVec::from(knots), mt::KnotVec::from(vec![5., 5., 5., 9., 9., 9.])),
            points,
        )));
    }
}
#[test]
fn signed_zero_infinite_and_nan_control_bits_match_vendor() {
    let p = mt::Point3::new;
    for special in [-0., f64::INFINITY, f64::NEG_INFINITY, f64::from_bits(0x7ff8_0000_0000_0042)] {
        let knots = mt::KnotVec::bezier_knot(1);
        compare(&mt::Surface::BSplineSurface(mt::BSplineSurface::new(
            (knots.clone(), knots),
            vec![vec![p(special, -0., 0.), p(0., 1., 0.)], vec![p(1., 0., 0.), p(1., 1., 0.)]],
        )));
    }
}
#[test]
fn irregular_rows_keep_the_vendors_exact_zip_limits() {
    let p = mt::Point3::new;
    for second in [vec![], vec![p(1., 0., 0.)], vec![p(1., 0., 0.), p(1., 1., 0.), p(f64::NAN, f64::NAN, f64::NAN)]] {
        let knots = mt::KnotVec::bezier_knot(1);
        compare(&mt::Surface::BSplineSurface(mt::BSplineSurface::new_unchecked(
            (knots.clone(), knots),
            vec![vec![p(0., 0., 0.), p(0., 1., 0.)], second],
        )));
    }
}
#[test]
fn larger_basis_requests_and_coordinate_shapes_fall_back() {
    let large = bezier(4, 0.);
    let (u, v) = coordinates(&large);
    assert!(batched(&large, &u, &v).is_none());
    let small = bezier(1, 0.);
    let (u, v) = coordinates(&small);
    assert!(batched(&small, &u[..50], &v).is_none());
    assert!(batched(&small, &u, &[]).is_none());
}
#[test]
fn analytic_and_rational_surfaces_keep_the_ordinary_path() {
    let p = mt::Point3::new;
    let u = vec![0.; 51];
    let v = vec![0.; 51];
    let plane = mt::Surface::Plane(mt::Plane::new(p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.)));
    assert!(batched(&plane, &u, &v).is_none());
    let knots = mt::KnotVec::bezier_knot(1);
    let rational = mt::Surface::NurbsSurface(mt::NurbsSurface::new(mt::BSplineSurface::new(
        (knots.clone(), knots),
        vec![
            vec![mt::Vector4::new(0., 0., 0., 1.), mt::Vector4::new(0., 2., 0., 2.)],
            vec![mt::Vector4::new(0.5, 0., 0., 0.5), mt::Vector4::new(1., 1., 0., 1.)],
        ],
    )));
    assert!(batched(&rational, &u, &v).is_none());
}
#[test]
fn malformed_degrees_and_empty_controls_defer_without_panicking() {
    let p = mt::Point3::new;
    let u = vec![0.; 51];
    let v = vec![0.; 51];
    for points in [vec![], vec![vec![]], vec![vec![p(0., 0., 0.); 2]; 5]] {
        let knots = mt::KnotVec::bezier_knot(1);
        let surface = mt::Surface::BSplineSurface(mt::BSplineSurface::new_unchecked((knots.clone(), knots), points));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| batched(&surface, &u, &v)));
        assert!(result.expect("cache eligibility must not panic").is_none());
    }
}
