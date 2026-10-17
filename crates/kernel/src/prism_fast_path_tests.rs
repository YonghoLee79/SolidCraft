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

/// A prism join leaves one cylindrical wall as one face (#1986).
///
/// The prism boolean slices both operands at every level either of them ends at and sews a wall
/// per slab. A straight wall split that way is put back together by the coplanar healing after
/// the boolean; a cylindrical one is not, so a join left one cylinder as a stack of
/// co-cylindrical faces for good. A blend along the top of such a wall then read the face beside
/// it as one slab tall and refused, although the wall is whole and many times that.
mod joined_cylindrical_walls {
    use std::f64::consts::PI;
    use std::sync::Arc;

    use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};

    use crate::{Body, BoolOp, FaceSurface, boolean, chamfer, extrude, face_surface, fillet, measure};

    /// The sectors' radii: the outer one stands between [`R_MID`] and [`R_OUT`], the inner one
    /// between [`R_IN`] and [`R_MID`].
    const R_IN: f64 = 3.0;
    const R_MID: f64 = 6.0;
    const R_OUT: f64 = 10.0;
    /// The level the inner sector ends at, and the top both the outer one and the join reach.
    const JOIN: f64 = 18.0;
    const TOP: f64 = 20.0;
    /// Half of either sector's angle: both run from -60 to +60 degrees.
    const HALF: f64 = PI / 3.0;
    /// The chamfer asked for: larger than the `TOP - JOIN` slab, well inside the faces beside it
    /// (4 mm of top face, a wall `TOP` tall).
    const SIZE: f64 = 3.0;

    /// A counter-clockwise annular sector between `r0` and `r1`, from -[`HALF`] to +[`HALF`].
    fn sector(r0: f64, r1: f64) -> Region2 {
        let at = |r: f64, a: f64| Vec2::new(r * a.cos(), r * a.sin());
        Region2 {
            outer: Loop2 {
                segs: vec![
                    Seg2::Line { a: at(r0, -HALF), b: at(r1, -HALF) },
                    Seg2::Arc { center: Vec2::ZERO, radius: r1, start: -HALF, sweep: 2.0 * HALF },
                    Seg2::Line { a: at(r1, HALF), b: at(r0, HALF) },
                    Seg2::Arc { center: Vec2::ZERO, radius: r0, start: HALF, sweep: -2.0 * HALF },
                ],
            },
            holes: Vec::new(),
        }
    }

    fn prism(r0: f64, r1: f64, top: f64) -> Body {
        extrude(&Plane::XY, &[sector(r0, r1)], 0.0, top).expect("the sector extrudes").pop().expect("one body")
    }

    fn disc(r: f64, z0: f64, z1: f64) -> Body {
        let region = Region2 { outer: Loop2::circle(Vec2::ZERO, r), holes: Vec::new() };
        extrude(&Plane::XY, &[region], z0, z1).expect("the disc extrudes").pop().expect("one body")
    }

    /// The outer sector with the inner one joined to it: the join's levels are 0, [`JOIN`] and
    /// [`TOP`], so the outer wall is the same arc in two successive slabs.
    fn joined() -> Body {
        boolean(&prism(R_MID, R_OUT, TOP), &prism(R_IN, R_MID, JOIN), BoolOp::Union).expect("the join").expect("a result")
    }

    /// How many faces of `body` lie on the cylinder of radius `r`.
    fn walls_of_radius(body: &Body, r: f64) -> usize {
        (0..body.face_count())
            .filter(|i| matches!(face_surface(body, *i), Some(FaceSurface::Cylinder { radius, .. }) if (radius - r).abs() < 1e-6))
            .count()
    }

    /// The volume of an annular sector of [`HALF`] either side, between `r0` and `r1`, `h` tall.
    fn wall_volume(r0: f64, r1: f64, h: f64) -> f64 {
        HALF * (r1 * r1 - r0 * r0) * h
    }

    #[test]
    fn a_joined_cylindrical_wall_is_one_face() {
        let body = joined();
        assert_eq!(walls_of_radius(&body, R_OUT), 1, "one cylinder is one face, not one per slab");
        assert_eq!(walls_of_radius(&body, R_MID), 1, "the inner sector's wall above the join is one face");
        assert!(body.validity().is_empty(), "{:?}", body.validity());
        // The join itself is unchanged by how its walls are sewn.
        let want = wall_volume(R_MID, R_OUT, TOP) + wall_volume(R_IN, R_MID, JOIN);
        let got = measure(&body).expect("the join measures").volume;
        assert!((got - want).abs() < 2e-3 * want, "the join holds {got} mm3, want {want} mm3");
    }

    #[test]
    fn the_top_arc_edge_takes_a_chamfer_larger_than_one_slab() {
        let body = joined();
        let before = measure(&body).expect("the join measures").volume;
        let chamfered = chamfer(&body, &[Vec3::new(R_OUT, 0.0, TOP)], SIZE).expect("the chamfer");
        assert!(chamfered.validity().is_empty(), "{:?}", chamfered.validity());
        // A chamfer along one arc is one cone, and the wall it ran down is still one face.
        let cones = (0..chamfered.face_count()).filter(|i| matches!(face_surface(&chamfered, *i), Some(FaceSurface::Cone { .. }))).count();
        assert_eq!(cones, 1, "a chamfer along one arc edge is one conical face");
        assert_eq!(walls_of_radius(&chamfered, R_OUT), 1, "the chamfered wall stays one face");
        // The wedge taken: a triangle of legs `SIZE` swept round, so the integral of
        // (r - (R_OUT - SIZE))*r dr over the radii it covers, times the angle.
        let wedge = 2.0 * HALF * (SIZE * SIZE * SIZE / 3.0 + (R_OUT - SIZE) * SIZE * SIZE / 2.0);
        let took = before - measure(&chamfered).expect("the chamfer measures").volume;
        assert!((took - wedge).abs() < 2e-3 * wedge, "the chamfer took {took} mm3, want {wedge} mm3");
    }

    /// The wall a later feature picks is whole, however many slabs the join sliced it into: a
    /// fillet on the same arc edge lands, and afterwards exactly one face of radius [`R_OUT`]
    /// about the extrusion's axis is there to resolve a reference to.
    #[test]
    fn a_later_blend_reads_the_joined_wall_as_one_face() {
        let body = joined();
        let filleted = fillet(&body, &[Vec3::new(R_OUT, 0.0, TOP)], SIZE).expect("the fillet");
        assert!(filleted.validity().is_empty(), "{:?}", filleted.validity());
        let found: Vec<_> = (0..filleted.face_count())
            .filter_map(|i| match face_surface(&filleted, i) {
                Some(FaceSurface::Cylinder { radius, axis, .. }) if (radius - R_OUT).abs() < 1e-6 => Some(axis),
                _ => None,
            })
            .collect();
        assert_eq!(found.len(), 1, "one wall to resolve, not a stack of them");
        assert!(found[0].dot(Vec3::Z).abs() > 1.0 - 1e-9, "the wall's axis is the extrusion's");
    }

    /// What the join leaves is intersected the same as before. An intersection is never sewn
    /// slab by slab (the prism route declines it), so it meets the joined body through the
    /// general boolean, with the wall one face where it used to be two: the plane it is cut on
    /// is at the level the wall was split at, so the cut lands in the middle of a face instead
    /// of on its edge. A thin slice of the same body, as a clearance measurement takes, must
    /// come out in full too.
    #[test]
    fn a_joined_wall_is_intersected_the_same() {
        // A shaft with a flange over the top of it: the levels are 0, 8, 10 and 12, so the
        // flange's wall is the same arc in the last two slabs.
        let pin = boolean(&disc(4.0, 0.0, 10.0), &disc(8.0, 8.0, 12.0), BoolOp::Union).expect("the pin").expect("a pin");
        // A whole circle is two faces across its seam: two, not the four of a wall per slab.
        assert_eq!(walls_of_radius(&pin, 8.0), 2, "the flange's wall is one cylinder across the shaft's top");
        for (lo, hi) in [(10.0, 12.0), (9.99, 10.0)] {
            let slab = disc(20.0, lo, hi);
            let shared = boolean(&pin, &slab, BoolOp::Intersect).expect("the intersection").expect("they share material");
            assert!(shared.validity().is_empty(), "{lo}..{hi}: {:?}", shared.validity());
            let want = PI * 64.0 * (hi - lo);
            let got = measure(&shared).expect("the intersection measures").volume;
            assert!((got - want).abs() < 2e-3 * want, "{lo}..{hi}: the slice holds {got} mm3, want {want} mm3");
        }
    }

    /// A pin sunk a hair into a plate with a clearance hole round it. What the two share is a
    /// ring 0.01 mm deep, far shallower than either body's faceting, and the general boolean
    /// reads such a pair from samples and from its facets. The only facet that witnessed the
    /// ring was one the join left behind because it sewed the pin's wall as two faces, split a
    /// hair above the plate; sewing that wall as the one face it is must not turn the ring into
    /// no shared material at all.
    #[test]
    fn a_hair_deep_overlap_of_a_joined_pin_is_still_found() {
        let (bore, deep) = (5.0, 0.01);
        let outer = Loop2::polygon(&[Vec2::new(-30.0, -15.0), Vec2::new(30.0, -15.0), Vec2::new(30.0, 15.0), Vec2::new(-30.0, 15.0)]);
        let slab = extrude(&Plane::XY, &[Region2 { outer, holes: Vec::new() }], 0.0, 5.0).expect("the plate extrudes").pop().expect("one body");
        let plate = boolean(&slab, &disc(bore, -2.0, 7.0), BoolOp::Cut).expect("the bore").expect("material stays");
        // The shaft passes through the bore with room to spare; the head overhangs it and sits
        // `deep` into the plate's top. The shaft's top is inside the head, so the head's wall is
        // the same arc in both slabs above it.
        let pin = boolean(&disc(4.5, -2.0, 5.2), &disc(8.0, 5.0 - deep, 7.0), BoolOp::Union).expect("the pin").expect("a pin");
        assert_eq!(walls_of_radius(&pin, 8.0), 2, "the head's wall is one cylinder, not one per slab");
        let shared = boolean(&pin, &plate, BoolOp::Intersect).expect("the intersection").expect("the head's foot is in the plate");
        let want = PI * (64.0 - bore * bore) * deep;
        let got = measure(&shared).expect("the shared ring measures").volume;
        assert!((got - want).abs() < 5e-3 * want, "the ring holds {got} mm3, want {want} mm3");
    }

    /// Walls are joined only where nothing authored says they are apart: an operand carrying
    /// face paint, or lines its faces were split along on purpose, keeps its wall as it was
    /// sewn, slab by slab.
    #[test]
    fn a_painted_or_split_operand_keeps_its_wall_split() {
        let (outer, inner) = (prism(R_MID, R_OUT, TOP), prism(R_IN, R_MID, JOIN));
        let paint = crate::Paint { opacity: 1.0, faces: vec![crate::FacePaint { face: 0, color: [1.0, 0.0, 0.0], opacity: 1.0 }] };
        let painted = boolean(&outer.clone().with_paint(Some(paint)), &inner, BoolOp::Union).expect("the join").expect("a result");
        assert_eq!(walls_of_radius(&painted, R_OUT), 2, "a painted operand's wall is left as it was sewn");
        let lines = Arc::new(vec![vec![Vec3::new(R_OUT, 0.0, 0.0), Vec3::new(R_OUT, 0.0, TOP)]]);
        let split = boolean(&outer.with_split_lines(Some(lines)), &inner, BoolOp::Union).expect("the join").expect("a result");
        assert_eq!(walls_of_radius(&split, R_OUT), 2, "an operand with authored split lines keeps its wall split");
    }
}
