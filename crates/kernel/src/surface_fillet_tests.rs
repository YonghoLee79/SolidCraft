//! Fillets between separate surface bodies (#769), checked against analytic blends: two plates
//! at right angles give a quarter cylinder, a plate and a cylinder a known arc, crossing plates
//! the quadrant the picks choose; faces a ball can't touch both of are refused.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::f64::consts::PI;

use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};

use crate::{Body, FilletFace, KernelError, extrude_sheet, measure, patch_region, surface_fillet};

/// A flat sheet on `plane` over `[u0, u1] × [v0, v1]` of its coordinates.
fn plate(plane: Plane, u0: f64, v0: f64, u1: f64, v1: f64) -> Body {
    let outer = Loop2::polygon(&[Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)]);
    patch_region(&plane, &Region2 { outer, holes: Vec::new() }).unwrap()
}

/// The ground, z = 0, over x in [x0, x1] and y in [0, 30].
fn ground(x0: f64, x1: f64) -> Body {
    plate(Plane::XY, x0, 0.0, x1, 30.0)
}

/// The wall x = 0 over y in [0, 30] and z in [z0, z1].
fn wall(z0: f64, z1: f64) -> Body {
    plate(Plane { origin: Vec3::ZERO, x: Vec3::Y, y: Vec3::Z }, 0.0, z0, 30.0, z1)
}

fn fillet(a: &Body, pa: Vec3, b: &Body, pb: Vec3, r: f64) -> crate::Result<Body> {
    surface_fillet(FilletFace { body: a, pick: pa }, FilletFace { body: b, pick: pb }, r)
}

/// Every point of the fillet's mesh lies `r` from the line through `o` along y.
fn on_cylinder(b: &Body, o: Vec3, r: f64) -> f64 {
    let mesh = b.tessellate(1e-3).unwrap();
    mesh.positions.iter().map(|p| (Vec3::new(p.x - o.x, 0.0, p.z - o.z).len() - r).abs()).fold(0.0, f64::max)
}

/// A ground and a wall meeting at a right angle: the fillet is the quarter cylinder of radius 5
/// about the line (5, y, 5), as long as both plates (30): area 5 · π/2 · 30.
#[test]
fn two_plates_at_a_right_angle_give_a_quarter_cylinder() {
    let (g, w) = (ground(0.0, 40.0), wall(0.0, 40.0));
    let b = fillet(&g, Vec3::new(20.0, 15.0, 0.0), &w, Vec3::new(0.0, 15.0, 20.0), 5.0).unwrap();
    let m = measure(&b).unwrap();
    let want = 5.0 * PI / 2.0 * 30.0;
    assert!((m.area - want).abs() < 1e-3 * want, "area {} vs {want}", m.area);
    let off = on_cylinder(&b, Vec3::new(5.0, 0.0, 5.0), 5.0);
    assert!(off < 1e-4, "{off} off the quarter cylinder");
    assert!(m.bbox.min.y.abs() < 1e-6 && (m.bbox.max.y - 30.0).abs() < 1e-6, "{:?}", m.bbox);
    assert!(m.bbox.min.x.abs() < 1e-6 && (m.bbox.max.x - 5.0).abs() < 1e-6 && (m.bbox.max.z - 5.0).abs() < 1e-6, "{:?}", m.bbox);
}

/// The ground and a cylinder of radius 10 along y whose axis is 4 above it: a ball of radius 3
/// on the ground and outside the cylinder has its centre 13 from the axis and 3 up, at
/// x = √(13² − 1²); the arc between its contacts turns by acos(−1/13).
#[test]
fn a_plate_and_a_cylinder_give_the_arc_between_them() {
    let (radius, h, r) = (10.0, 4.0, 3.0);
    let g = ground(-40.0, 40.0);
    let circle = vec![Seg2::Arc { center: Vec2::ZERO, radius, start: 0.0, sweep: 2.0 * PI }];
    // The cylinder's plane faces −y, so extruding from −30 to 0 runs y from 30 to 0.
    let cyl = extrude_sheet(&Plane { origin: Vec3::new(0.0, 0.0, h), x: Vec3::X, y: Vec3::Z }, &[circle], -30.0, 0.0).unwrap();
    let on_cyl = Vec3::new(radius * (PI / 6.0).cos(), 15.0, h + radius * (PI / 6.0).sin());
    let b = fillet(&g, Vec3::new(30.0, 15.0, 0.0), &cyl, on_cyl, r).unwrap();
    let m = measure(&b).unwrap();
    let xc = (169.0f64 - 1.0).sqrt();
    let want = r * (-1.0f64 / 13.0).acos() * 30.0;
    assert!((m.area - want).abs() < 1e-3 * want, "area {} vs {want}", m.area);
    let off = on_cylinder(&b, Vec3::new(xc, 0.0, r), r);
    assert!(off < 1e-4, "{off} off the ball's path");
}

/// Two plates crossing along the y axis: the picks choose the quadrant, and the fillet sits in
/// it, a quarter cylinder about (±5, y, ±5).
#[test]
fn crossing_plates_take_the_quadrant_the_picks_choose() {
    let (g, w) = (ground(-20.0, 20.0), wall(-20.0, 20.0));
    for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
        let b = fillet(&g, Vec3::new(10.0 * sx, 15.0, 0.0), &w, Vec3::new(0.0, 15.0, 10.0 * sz), 5.0).unwrap();
        let off = on_cylinder(&b, Vec3::new(5.0 * sx, 0.0, 5.0 * sz), 5.0);
        assert!(off < 1e-4, "quadrant ({sx}, {sz}): {off} off");
        let m = measure(&b).unwrap();
        assert!((m.centroid.x * sx) > 0.0 && (m.centroid.z * sz) > 0.0, "quadrant ({sx}, {sz}): centroid {:?}", m.centroid);
    }
}

/// Faces a ball can't touch both of are refused, saying why, without panicking: parallel plates,
/// plates narrower than the ball reaches, and a radius that is not a positive number.
#[test]
fn faces_a_ball_cannot_touch_both_of_are_refused() {
    let g = ground(0.0, 40.0);
    let above = plate(Plane { origin: Vec3::new(0.0, 0.0, 10.0), ..Plane::XY }, 0.0, 0.0, 40.0, 30.0);
    let refused = |r: crate::Result<Body>| match r {
        Err(KernelError::Invalid(m) | KernelError::Failed(m)) => m,
        other => panic!("{other:?}"),
    };
    let m = refused(fillet(&g, Vec3::new(20.0, 15.0, 0.0), &above, Vec3::new(20.0, 15.0, 10.0), 2.0));
    assert!(m.contains("parallel"), "{m}");
    let (narrow, short) = (ground(0.0, 3.0), wall(0.0, 3.0));
    let m = refused(fillet(&narrow, Vec3::new(1.5, 15.0, 0.0), &short, Vec3::new(0.0, 15.0, 1.5), 5.0));
    assert!(m.contains("can't touch both"), "{m}");
    let w = wall(0.0, 40.0);
    for r in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(fillet(&g, Vec3::new(20.0, 15.0, 0.0), &w, Vec3::new(0.0, 15.0, 20.0), r).is_err(), "{r}");
    }
    assert!(fillet(&g, Vec3::new(f64::NAN, 15.0, 0.0), &w, Vec3::new(0.0, 15.0, 20.0), 5.0).is_err());
}
