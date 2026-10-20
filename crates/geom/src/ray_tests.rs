use super::*;

#[test]
fn a_parameter_along_it_is_a_distance() {
    let Some(r) = Ray::towards(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -4.0)) else { panic!("a ray") };
    assert_eq!(r.at(2.5), Vec3::new(1.0, 2.0, 0.5));
}

#[test]
fn no_ray_along_no_direction() {
    assert!(Ray::towards(Vec3::ZERO, Vec3::ZERO).is_none());
    assert!(Ray::towards(Vec3::ZERO, Vec3::new(f64::NAN, 0.0, 1.0)).is_none());
    assert!(!Ray::new(Vec3::new(f64::INFINITY, 0.0, 0.0), UnitVec3::Z).is_finite());
}

#[test]
fn where_a_ray_crosses_a_box() {
    let b = crate::Aabb3 { min: Vec3::ZERO, max: Vec3::new(2.0, 2.0, 2.0) };
    let down = |x: f64, y: f64, z: f64| Ray::new(Vec3::new(x, y, z), -UnitVec3::Z);
    assert_eq!(b.ray_span(&down(1.0, 1.0, 5.0), 0.0), Some((3.0, 5.0)));
    // From inside: entered behind the origin.
    assert_eq!(b.ray_span(&down(1.0, 1.0, 1.0), 0.0), Some((-1.0, 1.0)));
    // Beside the box, unless grown to reach it.
    assert_eq!(b.ray_span(&down(3.0, 1.0, 5.0), 0.0), None);
    assert!(b.ray_span(&down(3.0, 1.0, 5.0), 1.5).is_some());
    // Along an axis, the other axes only bound where it may pass.
    assert_eq!(Ray::towards(Vec3::new(-1.0, 1.0, 1.0), Vec3::X).and_then(|r| b.ray_span(&r, 0.0)), Some((1.0, 3.0)));
}
