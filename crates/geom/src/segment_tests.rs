use super::*;

#[test]
fn the_nearest_point_stays_on_the_segment() {
    let s = Segment::new(Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0));
    assert_eq!(s.closest_point(Vec3::new(1.0, 3.0, 0.0)), Vec3::new(1.0, 0.0, 0.0));
    // Past an end: the end.
    assert_eq!(s.closest_point(Vec3::new(-2.0, 1.0, 0.0)), Vec3::ZERO);
    assert_eq!(s.param(Vec3::new(9.0, 0.0, 0.0)), 1.0);
    assert_eq!(s.dist(Vec3::new(2.0, 0.0, 5.0)), 5.0);
    assert_eq!(s.len(), 4.0);
    // In the plane too.
    assert_eq!(Segment::new(Vec2::ZERO, Vec2::new(0.0, 2.0)).dist(Vec2::new(3.0, 1.0)), 3.0);
}

#[test]
fn a_segment_with_no_length_is_its_point() {
    let p = Vec3::new(1.0, 2.0, 3.0);
    assert_eq!(Segment::new(p, p).closest_point(Vec3::ZERO), p);
    let far = Segment::new(Vec3::ZERO, Vec3::new(f64::INFINITY, 0.0, 0.0));
    assert_eq!(far.param(Vec3::X), 0.0);
}

#[test]
fn nearest_points_of_two_segments() {
    let s = Segment::new(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
    let t = Segment::new(Vec3::new(0.0, -1.0, 2.0), Vec3::new(0.0, 1.0, 2.0));
    assert_eq!(s.closest_points(&t), (Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0)));
    // Parallel: the first's start and the point across from it.
    let u = Segment::new(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0));
    assert_eq!(s.closest_points(&u), (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)));
}

#[test]
fn the_segments_of_a_polyline() {
    let pts = [Vec2::ZERO, Vec2::X, Vec2::new(1.0, 1.0)];
    let segs: Vec<Segment<Vec2>> = Segment::along(&pts).collect();
    assert_eq!(segs, vec![Segment::new(Vec2::ZERO, Vec2::X), Segment::new(Vec2::X, Vec2::new(1.0, 1.0))]);
    assert_eq!(Segment::along(&pts[..1]).count(), 0);
}

#[test]
fn a_chain_measured_along_its_segments() {
    let corners = [Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(2.0, 1.0)];
    let (path, ring) = (crate::Path::from(corners), crate::Ring::from(corners));
    assert_eq!(path.length(), 3.0);
    // Around the ring, back to the start too.
    assert!((ring.length() - (3.0 + 5f64.sqrt())).abs() < 1e-12);
    assert_eq!(path.distance_to(Vec2::new(1.0, -2.0)), 2.0);
    assert_eq!(crate::Path::<Vec2>::from([Vec2::ZERO]).distance_to(Vec2::ZERO), f64::INFINITY);
}
