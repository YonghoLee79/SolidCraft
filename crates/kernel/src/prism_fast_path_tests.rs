//! The prism fast path: `boolean_whole` asks the exact 2D prism route before it meshes both
//! operands, so a plate and the tool through it are decided without a mesh of either.
use super::*;
use crate::{BoolOp, boolean, box_solid, cylinder, measure};
use std::f64::consts::PI;

/// A cylindrical tool through a plate: the exact prism route answers the pair, and the material
/// it removes is the tool's own.
#[test]
fn a_hole_through_a_plate_removes_the_tool_volume() {
    let plate = box_solid(Vec3::ZERO, Vec3::new(40.0, 30.0, 10.0)).expect("plate");
    let tool = cylinder(Vec3::new(20.0, 15.0, -1.0), Vec3::Z, 5.0, 12.0).expect("tool");
    let cut = boolean(&plate, &tool, BoolOp::Cut).expect("cut").expect("a hole leaves material");
    let m = measure(&cut).expect("measure");
    let want = 40.0 * 30.0 * 10.0 - PI * 25.0 * 10.0;
    assert!((m.volume - want).abs() < want * 5e-3, "{} vs {want}", m.volume);
    assert!(cut.validity().is_empty(), "{:?}", cut.validity());
}

/// Boxes that stay apart: the union keeps both, whichever route decides it.
#[test]
fn bodies_that_stay_apart_join_into_one_body_with_both_lumps() {
    let a = box_solid(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0)).expect("a");
    let b = box_solid(Vec3::new(20.0, 0.0, 0.0), Vec3::new(30.0, 10.0, 10.0)).expect("b");
    let joined = boolean(&a, &b, BoolOp::Union).expect("union").expect("two boxes");
    assert!((measure(&joined).expect("measure").volume - 2000.0).abs() < 1e-6);
    assert_eq!(joined.lumps().expect("lumps").len(), 2);
}

/// A tool wholly inside its target leaves a closed void, not a through cut.
#[test]
fn a_tool_wholly_inside_leaves_a_closed_void() {
    let block = box_solid(Vec3::ZERO, Vec3::new(40.0, 40.0, 40.0)).expect("block");
    let inner = box_solid(Vec3::new(15.0, 15.0, 15.0), Vec3::new(25.0, 25.0, 25.0)).expect("inner");
    let cut = boolean(&block, &inner, BoolOp::Cut).expect("cut").expect("a void leaves material");
    assert!((measure(&cut).expect("measure").volume - 63000.0).abs() < 1e-6);
    assert!(cut.validity().is_empty(), "{:?}", cut.validity());
}

/// A tool that misses its target, in section or along the extrusion, leaves it as it is: the
/// sampled shortcut answers that by reusing the operand itself, and the 2D route must not
/// rebuild it into a look-alike solid on the way past - an untouched body in a timeline has to
/// stay the very same body.
#[test]
fn a_cut_that_changes_nothing_keeps_the_original_body() {
    let plate = box_solid(Vec3::ZERO, Vec3::new(40.0, 30.0, 10.0)).expect("plate");
    let past_the_end = box_solid(Vec3::new(200.0, 200.0, -5.0), Vec3::new(210.0, 210.0, 20.0)).expect("tool");
    assert!(
        boolean(&plate, &past_the_end, BoolOp::Cut).expect("cut").expect("material stays").same(&plate),
        "a tool clear of the plate must leave the very same body"
    );
    let inside_the_height = box_solid(Vec3::new(200.0, 200.0, 5.0), Vec3::new(210.0, 210.0, 20.0)).expect("tool");
    assert!(
        boolean(&plate, &inside_the_height, BoolOp::Cut).expect("cut").expect("material stays").same(&plate),
        "a tool that only overlaps the plate's height must leave the very same body"
    );
    let covers_it = box_solid(Vec3::new(-10.0, -10.0, -10.0), Vec3::new(50.0, 40.0, 20.0)).expect("tool");
    assert!(boolean(&plate, &covers_it, BoolOp::Cut).expect("cut").is_none(), "a tool over the whole plate removes it");
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> solidcraft_geom::Region2 {
    solidcraft_geom::Region2 {
        outer: solidcraft_geom::Loop2::polygon(&[
            solidcraft_geom::Vec2::new(x0, y0),
            solidcraft_geom::Vec2::new(x1, y0),
            solidcraft_geom::Vec2::new(x1, y1),
            solidcraft_geom::Vec2::new(x0, y1),
        ]),
        holes: vec![],
    }
}

/// The 80 x 80 x 1.2 plate, with the 9 x 9 grid of 6.4 mm holes when `holes`.
fn plate(holes: bool) -> crate::Body {
    let plate = crate::extrude(&solidcraft_geom::Plane::XY, &[rect(0.0, 0.0, 80.0, 80.0)], 0.0, 1.2).unwrap().remove(0);
    if !holes {
        return plate;
    }
    let grid: Vec<solidcraft_geom::Region2> =
        (0..81).map(|k| (4.8 + 8.0 * (k % 9) as f64, 4.8 + 8.0 * (k / 9) as f64)).map(|(x, y)| rect(x, y, x + 6.4, y + 6.4)).collect();
    let tools = crate::extrude(&solidcraft_geom::Plane::XY, &grid, -0.1, 1.2).unwrap();
    tools.iter().fold(plate, |p, t| boolean(&p, t, BoolOp::Cut).unwrap().unwrap())
}

/// The ring (r 19.6 / 20.8 about (41.3, 40.7), off the hole grid) from `z0` to 21.2.
fn ring(z0: f64) -> crate::Body {
    let c = solidcraft_geom::Vec2::new(41.3, 40.7);
    let ring = solidcraft_geom::Region2 {
        outer: solidcraft_geom::Loop2::circle_from(c, 20.8, 0.0),
        holes: vec![solidcraft_geom::Loop2::circle_from(c, 19.6, 0.0).reversed()],
    };
    crate::extrude(&solidcraft_geom::Plane::XY, &[ring], z0, 21.2).unwrap().remove(0)
}

/// #2844: a ring joined onto a plate with a grid of square holes kept its walls split into
/// full-height strips, one seam wherever its foot crossed a hole edge. Sunk into the plate,
/// standing on it, and on a plain plate (the control): above the plate each wall is one cylinder
/// (its two halves) under a rim of two half circles, and the body holds the material the operands
/// make. Sunk, the wall's foot in each hole it crosses stays a face of its own: pieces stacked up
/// the axis don't merge.
#[test]
fn a_ring_on_a_perforated_plate_keeps_whole_walls() {
    let (plain, holed) = (plate(false), plate(true));
    for (base, z0, cylinders) in [(&plain, 0.6, 2), (&holed, 0.6, 12), (&holed, 1.2, 2)] {
        let ring = ring(z0);
        let joined = boolean(base, &ring, BoolOp::Union).unwrap().unwrap();
        let m = measure(&joined).unwrap();
        assert_eq!(m.merged.face_types.get("cylinder"), Some(&cylinders), "ring from z {z0}: {:?}", m.merged.face_types);
        let walls = joined
            .solid
            .face_iter()
            .filter(|f| !matches!(f.surface(), truck_modeling::Surface::Plane(_)))
            .filter(|f| f.edge_iter().flat_map(|e| crate::heal::edge_points(&e)).any(|p| (p.z - 21.2).abs() < 1e-9))
            .count();
        assert_eq!(walls, 4, "ring from z {z0}");
        // The top rims too: a half circle per edge, not a piece per strip.
        let rim: std::collections::HashSet<_> = joined
            .solid
            .edge_iter()
            .filter(|e| !matches!(e.curve(), truck_modeling::Curve::Line(_)))
            .filter(|e| crate::heal::edge_points(e).iter().all(|p| (p.z - 21.2).abs() < 1e-9))
            .map(|e| e.id())
            .collect();
        assert_eq!(rim.len(), 4, "ring from z {z0}");
        assert!(joined.validity().is_empty(), "{:?}", joined.validity());
        let common = boolean(base, &ring, BoolOp::Intersect).unwrap().map_or(0.0, |b| measure(&b).unwrap().volume);
        let want = measure(base).unwrap().volume + measure(&ring).unwrap().volume - common;
        assert!((m.volume - want).abs() < want * 1e-4, "ring from z {z0}: {} vs {want}", m.volume);
    }
}
