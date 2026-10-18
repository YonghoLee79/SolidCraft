//! Faceted booleans with helical tools (modelled threads).

use std::f64::consts::PI;

use solidcraft_geom::{Plane, Vec2, Vec3};

use crate::{BoolOp, cylinder, faceted_boolean, sweep_helix_mesh};

/// A trapezoid groove screwed into a shaft: the result is closed, and the volume removed is
/// the section's 2π∫r dA per turn over the turns inside the shaft.
#[test]
fn faceted_cut_of_a_helical_groove() {
    let shaft = cylinder(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 1.5, 4.0).unwrap();
    let (p, k) = (0.5, Vec3::new(0.0, 0.0, 1.0));
    // From a turn under the shaft to its middle: 4.5 turns, the first half-turn in the air.
    let base = Vec3::new(0.0, 0.0, -p);
    let plane = Plane { origin: base, x: Vec3::new(1.0, 0.0, 0.0), y: k };
    let section = [Vec2::new(1.2, -0.06), Vec2::new(1.6, -0.06), Vec2::new(1.6, 0.06), Vec2::new(1.2, 0.06)];
    let groove = sweep_helix_mesh(&plane, &section, base, k, p, 5.0, 48).unwrap();
    let cut = faceted_boolean(&shaft, &groove, BoolOp::Cut, 1e-3).unwrap().unwrap();
    let m = cut.tessellate(0.01).unwrap().measure();
    // In the shaft from z = 0: the section (r 1.2…1.5, 0.12 tall) for 4 turns, plus the part of
    // the first turn's section above z = 0 (its z range is ±0.06 about the turn's height).
    let per_turn = 2.0 * PI * 0.12 * (1.5f64.powi(2) - 1.2f64.powi(2)) / 2.0;
    let removed = PI * 2.25 * 4.0 - m.volume;
    assert!((removed - 4.0 * per_turn).abs() < 0.03 * per_turn * 4.0, "{removed} vs {}", 4.0 * per_turn);
}

/// A groove cut into the wall of a bore through a block (a nut): it runs out of both ends.
#[test]
fn faceted_cut_into_a_bore() {
    let nut = crate::box_solid(Vec3::new(-3.0, -3.0, 0.0), Vec3::new(3.0, 3.0, 2.0)).unwrap();
    let bore = cylinder(Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0), 1.23, 4.0).unwrap();
    let nut = crate::boolean(&nut, &bore, BoolOp::Cut).unwrap().unwrap();
    let v0 = nut.tessellate(1e-3).unwrap().measure().volume;
    let (p, k) = (0.5, Vec3::new(0.0, 0.0, 1.0));
    let base = Vec3::new(0.0, 0.0, -p);
    let plane = Plane { origin: base, x: Vec3::new(1.0, 0.0, 0.0), y: k };
    let section = [Vec2::new(1.1, -0.1), Vec2::new(1.5, -0.1), Vec2::new(1.5, 0.1), Vec2::new(1.1, 0.1)];
    let groove = sweep_helix_mesh(&plane, &section, base, k, p, 6.0, 48).unwrap();
    let cut = faceted_boolean(&nut, &groove, BoolOp::Cut, 2.5e-3).unwrap().unwrap();
    let removed = v0 - cut.tessellate(1e-3).unwrap().measure().volume;
    // Four turns of the section's part outside the bore (r 1.23…1.5, 0.2 tall).
    let want = 4.0 * PI * 0.2 * (1.5f64.powi(2) - 1.23f64.powi(2));
    assert!((removed - want).abs() < 0.03 * want, "{removed} vs {want}");
}

/// The groove facets only the faces round it: a round boss and a second hole away from the
/// threaded bore keep their exact cylinders (they were all cut into flat facets, so nothing
/// could pick them as cylinders again).
#[test]
fn faceted_cut_leaves_faces_away_from_the_groove_exact() {
    let block = crate::box_solid(Vec3::new(-6.0, -6.0, 0.0), Vec3::new(6.0, 6.0, 2.0)).unwrap();
    let boss = cylinder(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, 1.0), 3.0, 2.0).unwrap();
    let part = crate::boolean(&block, &boss, BoolOp::Union).unwrap().unwrap();
    let bore = cylinder(Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0), 1.23, 6.0).unwrap();
    let part = crate::boolean(&part, &bore, BoolOp::Cut).unwrap().unwrap();
    let other = cylinder(Vec3::new(4.5, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0), 0.8, 4.0).unwrap();
    let part = crate::boolean(&part, &other, BoolOp::Cut).unwrap().unwrap();
    let cylinders = |b: &crate::Body| crate::measure(b).unwrap().merged.face_types.get("cylinder").copied().unwrap_or(0);
    assert_eq!(cylinders(&part), 3);
    let v0 = part.tessellate(1e-3).unwrap().measure().volume;
    // Through the bore's whole length (z 0…4), running out of both ends.
    let (p, k) = (0.5, Vec3::new(0.0, 0.0, 1.0));
    let base = Vec3::new(0.0, 0.0, -p);
    let plane = Plane { origin: base, x: Vec3::new(1.0, 0.0, 0.0), y: k };
    let section = [Vec2::new(1.1, -0.1), Vec2::new(1.5, -0.1), Vec2::new(1.5, 0.1), Vec2::new(1.1, 0.1)];
    let groove = sweep_helix_mesh(&plane, &section, base, k, p, 10.0, 48).unwrap();
    let cut = faceted_boolean(&part, &groove, BoolOp::Cut, 2.5e-3).unwrap().unwrap();
    assert_eq!(cut.shell_count(), 1);
    assert!(cut.validity().is_empty(), "{:?}", cut.validity());
    // The boss's wall and the second hole are still cylinders; only the bore's wall is faceted.
    assert_eq!(cylinders(&cut), 2);
    assert!(crate::cylinder_face_at(&cut, Vec3::new(3.0, 0.0, 3.0)).is_some(), "the boss");
    assert!(crate::cylinder_face_at(&cut, Vec3::new(5.3, 0.0, 1.0)).is_some(), "the other hole");
    // Those two (each round wall is two faces), the block's four sides and its top round the
    // boss come through whole, in the history.
    let h = cut.history().expect("the faceted boolean reports its history");
    assert_eq!(h.faces.iter().filter(|f| f.change == crate::history::Change::Kept).count(), 9);
    let removed = v0 - cut.tessellate(1e-3).unwrap().measure().volume;
    // Eight turns of the section's part outside the bore (r 1.23…1.5, 0.2 tall).
    let want = 8.0 * PI * 0.2 * (1.5f64.powi(2) - 1.23f64.powi(2));
    assert!((removed - want).abs() < 0.03 * want, "{removed} vs {want}");
}

/// #1250: a long thread (48 turns of an M8 groove on a 60 mm shaft). The groove's facets come in
/// sweep order; taken as BSP splitters in that order they made the tree as deep as the facet
/// list ("planar boolean: tree too deep", after minutes and gigabytes). It cuts now, closed.
#[test]
fn a_long_thread_cuts_without_a_deep_tree() {
    let (r, len, p) = (4.0, 60.0, 1.25);
    let shaft = cylinder(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), r, len).unwrap();
    let k = Vec3::new(0.0, 0.0, 1.0);
    let base = Vec3::new(0.0, 0.0, -p);
    let plane = Plane { origin: base, x: Vec3::new(1.0, 0.0, 0.0), y: k };
    let section = [Vec2::new(3.3, -0.08), Vec2::new(4.1, -0.5), Vec2::new(4.1, 0.5), Vec2::new(3.3, 0.08)];
    let groove = sweep_helix_mesh(&plane, &section, base, k, p, len / p + 2.0, 48).unwrap();
    let cut = faceted_boolean(&shaft, &groove, BoolOp::Cut, p * 0.005).unwrap().unwrap();
    let v = cut.tessellate(0.01).unwrap().measure().volume;
    let v0 = PI * r * r * len;
    assert!(v < v0 * 0.95 && v > v0 * 0.6, "{v} of {v0}");
}
