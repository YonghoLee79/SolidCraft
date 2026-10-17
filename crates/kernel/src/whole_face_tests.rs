use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};

use super::*;
use crate::{BoolOp, boolean, box_solid, measure};

fn square(a: Vec2, b: Vec2) -> Region2 {
    let p = Vec2::new;
    let line = |a: Vec2, b: Vec2| Seg2::Line { a, b };
    Region2 { outer: Loop2 { segs: vec![line(a, p(b.x, a.y)), line(p(b.x, a.y), b), line(b, p(a.x, b.y)), line(p(a.x, b.y), a)] }, holes: vec![] }
}

#[test]
fn a_profile_matches_the_face_it_outlines_and_no_other() {
    let block = box_solid(Vec3::ZERO, Vec3::new(20.0, 10.0, 5.0)).unwrap();
    let top = Plane { origin: Vec3::new(0.0, 0.0, 5.0), ..Plane::XY };
    let whole = matching_face(&block, &top, &square(Vec2::new(0.0, 0.0), Vec2::new(20.0, 10.0)), &[0.0]).unwrap();
    assert!((whole.normal - Vec3::Z).len() < 1e-9 && whole.height.abs() < 1e-9, "{whole:?}");
    // A smaller rectangle on the same face is not the face.
    assert!(matching_face(&block, &top, &square(Vec2::new(1.0, 1.0), Vec2::new(19.0, 9.0)), &[0.0]).is_none());
    // Nor is the bottom face, though its outline is the same.
    assert!(matching_face(&block, &top, &square(Vec2::ZERO, Vec2::new(20.0, 10.0)), &[-3.0]).is_none());
}

#[test]
fn a_cut_over_the_whole_top_takes_only_the_face_and_a_step_beside_it_does_not() {
    let block = box_solid(Vec3::ZERO, Vec3::new(20.0, 10.0, 5.0)).unwrap();
    let top =
        matching_face(&block, &Plane { origin: Vec3::new(0.0, 0.0, 5.0), ..Plane::XY }, &square(Vec2::ZERO, Vec2::new(20.0, 10.0)), &[0.0]).unwrap();
    assert!(cut_takes_only_the_face(&block, top, 2.0));
    // A step standing beside the top face: a slab would take it too.
    let step = box_solid(Vec3::new(20.0, 0.0, 0.0), Vec3::new(30.0, 10.0, 8.0)).unwrap();
    let stepped = boolean(&block, &step, BoolOp::Union).unwrap().unwrap();
    let face = matching_face(&stepped, &Plane { origin: Vec3::new(0.0, 0.0, 5.0), ..Plane::XY }, &square(Vec2::ZERO, Vec2::new(20.0, 10.0)), &[0.0])
        .unwrap();
    assert!(!cut_takes_only_the_face(&stepped, face, 2.0));
}

#[test]
fn a_prism_from_the_face_joins_by_its_shared_edges() {
    let block = box_solid(Vec3::ZERO, Vec3::new(20.0, 10.0, 5.0)).unwrap();
    let plane = Plane { origin: Vec3::new(0.0, 0.0, 5.0), ..Plane::XY };
    let region = square(Vec2::ZERO, Vec2::new(20.0, 10.0));
    let face = matching_face(&block, &plane, &region, &[0.0]).unwrap();
    let built = crate::extrude(&plane, std::slice::from_ref(&region), 0.0, 3.0).unwrap().remove(0);
    let tool = face_prism(&block, face, 3.0, true, &built).unwrap();
    assert!(tool.history().is_some_and(|h| h.operands == 1));
    let joined = boolean(&block, &tool, BoolOp::Union).unwrap().unwrap();
    assert!((measure(&joined).unwrap().volume - 20.0 * 10.0 * 8.0).abs() < 1e-6);
}
