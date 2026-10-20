use super::*;

fn close(a: Vec3, b: Vec3) -> bool {
    a.dist(b) < 1e-12
}

#[test]
fn a_product_applies_the_right_factor_first() {
    let (turn, shift) = (Transform::rotation(Vec3::ZERO, Vec3::Z, std::f64::consts::FRAC_PI_2), Transform::translation(Vec3::new(1.0, 0.0, 0.0)));
    assert!(close((turn * shift).point(Vec3::ZERO), Vec3::new(0.0, 1.0, 0.0)));
    assert!(close((shift * turn).point(Vec3::ZERO), Vec3::new(1.0, 0.0, 0.0)));
    assert!(close((shift * turn).vector(Vec3::X), Vec3::Y));
}

#[test]
fn a_rigid_motion_turns_about_its_origin_then_moves() {
    let m = Transform::rigid(Vec3::new(0.0, 0.0, 5.0), Vec3::new(1.0, 0.0, 0.0), Vec3::Z, std::f64::consts::PI);
    assert!(close(m.point(Vec3::new(1.0, 0.0, 0.0)), Vec3::new(1.0, 0.0, 5.0)));
    assert!(close(m.point(Vec3::ZERO), Vec3::new(2.0, 0.0, 5.0)));
    assert!(m.is_rigid(1e-9));
    assert!(close((m * m.inverse().unwrap()).point(Vec3::new(3.0, -2.0, 7.0)), Vec3::new(3.0, -2.0, 7.0)));
    assert!((m.inverse().unwrap() * m).is_identity());
}

#[test]
fn reflections_scales_and_broken_rows_are_not_rigid() {
    let mirror = Transform::reflection(&Plane::XY);
    assert!(close(mirror.point(Vec3::new(1.0, 2.0, 3.0)), Vec3::new(1.0, 2.0, -3.0)));
    assert!(!mirror.is_rigid(1e-9));
    let mut m = Transform::IDENTITY;
    m[1][1] = 2.0;
    assert!(!m.is_rigid(1e-6));
    let mut m = Transform::IDENTITY;
    m[0][3] = 1e-6;
    assert!(!m.is_rigid(1e-6));
    let mut m = Transform::IDENTITY;
    m[2][0] = f64::NAN;
    assert!(!m.is_rigid(1e-6) && !m.is_finite());
    assert!(Transform([[0.0; 4]; 4]).inverse().is_none());
}

#[test]
fn saved_as_the_bare_array() {
    let m = Transform::translation(Vec3::new(1.0, 2.0, 3.0));
    let json = serde_json::to_value(m).unwrap();
    assert_eq!(json, serde_json::json!([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [1.0, 2.0, 3.0, 1.0]]));
    assert_eq!(serde_json::from_value::<Transform>(json).unwrap(), m);
}
