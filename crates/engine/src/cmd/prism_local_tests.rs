//! A cut whose walls lie on the target's, in a target that is a prism along the cut only round
//! it (a chamfer or a round elsewhere): the 2D route in a column cut round the tool
//! (`solidcraft_kernel`'s `prism_local`). The B-rep boolean finds no result on these.

use serde_json::{Value, json};
use std::f64::consts::PI;

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn volume(s: &mut Session) -> f64 {
    run(s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap_or(f64::NAN)
}

/// A cut of `params`: the volume it takes, after checking it built.
fn cut(s: &mut Session, params: Value) -> f64 {
    let before = volume(s);
    let id = run(s, "solid.extrude", params)["feature"].as_u64().unwrap();
    let r = s.model.result(id).unwrap();
    assert!(r.error.is_none(), "{r:?}");
    before - volume(s)
}

/// An 80 x 50 x 10 plate with a 10 mm round pocket 2 mm deep in its top; `edge` dressed first.
fn plate(edge: impl FnOnce(&mut Session)) -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane":"XY"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[80,50]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance":10}));
    edge(&mut s);
    pocket(&mut s, 2.0);
    s
}

/// The round pocket cut `depth` down from the plate's top.
fn pocket(s: &mut Session, depth: f64) -> f64 {
    run(s, "sketch.create", json!({"plane":"XY","offset":10}));
    run(s, "sketch.circle.center", json!({"center":[40,25],"radius":5}));
    run(s, "sketch.finish", json!({}));
    cut(s, json!({"distance":-depth,"operation":"cut"}))
}

/// The pocket made deeper (2 to 5 mm): its wall lies on the pocket's. The plate is a prism
/// along the cut but for its chamfered rim, or its rounded bottom edges.
#[test]
fn a_round_pocket_made_deeper_beside_a_chamfer_or_a_round() {
    let chamfered = |s: &mut Session| {
        run(s, "solid.chamfer", json!({"edges":[[40,0,10],[40,50,10],[0,25,10],[80,25,10]],"distance":1}));
    };
    let rounded = |s: &mut Session| {
        run(s, "solid.fillet", json!({"edges":[[40,0,0],[40,50,0],[0,25,0],[80,25,0]],"radius":2}));
    };
    let want = PI * 25.0 * 3.0;
    for (what, mut s) in [("chamfered", plate(chamfered)), ("rounded", plate(rounded))] {
        let taken = pocket(&mut s, 5.0);
        assert!((taken - want).abs() < want * 1e-3, "{what}: {taken} taken, want {want}");
    }
}

/// #1360's ring cut round a hole, the hole counterbored: the ring's inner wall lies on the
/// hole's, in a disc whose top rim is chamfered. Without the chamfer the disc is a prism and
/// the 2D route takes it whole.
#[test]
fn a_ring_cut_round_a_counterbored_hole_in_a_chamfered_disc() {
    let ring = |chamfer: bool| {
        let mut s = Session::default();
        run(&mut s, "sketch.create", json!({"plane":"XY"}));
        run(&mut s, "sketch.circle.center", json!({"center":[0,0],"radius":40}));
        run(&mut s, "sketch.finish", json!({}));
        run(&mut s, "solid.extrude", json!({"distance":10}));
        run(&mut s, "solid.hole", json!({"position":[20,0,10],"diameter":6,"type":"counterbore","cb_diameter":9,"cb_depth":1.5}));
        if chamfer {
            run(&mut s, "solid.chamfer", json!({"edges":[[40,0,10]],"distance":1}));
        }
        run(&mut s, "sketch.create", json!({"plane":{"face":[0,0,10]}}));
        run(&mut s, "sketch.circle.center", json!({"center":[20,0],"radius":3}));
        run(&mut s, "sketch.circle.center", json!({"center":[20,0],"radius":6}));
        run(&mut s, "sketch.finish", json!({}));
        // The ring between the counterbore's rim (projected from the face) and the 6 mm circle.
        cut(&mut s, json!({"direction":"negative","distance":3,"operation":"cut","profiles":[{"point":[25.25,0]}]}))
    };
    let want = PI * (36.0 - 20.25) * 3.0;
    for chamfer in [false, true] {
        let taken = ring(chamfer);
        assert!((taken - want).abs() < want * 1e-3, "chamfer {chamfer}: {taken} taken, want {want}");
    }
}

/// #3454: a keyhole cut from the back of a plate with rounded corners and a rounded top rim. The
/// head slot's round end lies on the entry hole's wall. The column round it is cut above the
/// keyhole, across the plate's rounded corners: that cut's edges there must be read as the arcs
/// they are, or the column is not taken as a prism.
#[test]
fn a_keyhole_slot_on_its_entry_hole_in_a_plate_with_a_rounded_rim() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane":"XY"}));
    run(&mut s, "sketch.rectangle.rounded", json!({"p0":[0,0],"p1":[60,30],"radius":4}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance":5}));
    let k = 4.0 * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
    run(
        &mut s,
        "solid.fillet",
        json!({"radius":1,"edges":[[30,0,5],[30,30,5],[0,15,5],[60,15,5],[k,k,5],[60.0-k,k,5],[k,30.0-k,5],[60.0-k,30.0-k,5]]}),
    );
    // The entry hole and the neck, from the back.
    run(&mut s, "sketch.create", json!({"plane":"XY"}));
    run(&mut s, "sketch.circle.center", json!({"center":[30,10],"diameter":8}));
    run(&mut s, "sketch.slot.center_to_center", json!({"p0":[30,10],"p1":[30,20],"width":3.2}));
    run(&mut s, "sketch.finish", json!({}));
    cut(&mut s, json!({"distance":3.3,"operation":"cut","profiles":"all"}));
    // The head's slot, as wide as the entry, above the neck.
    run(&mut s, "sketch.create", json!({"plane":"XY","offset":1.5}));
    run(&mut s, "sketch.slot.center_to_center", json!({"p0":[30,10],"p1":[30,20],"width":8}));
    run(&mut s, "sketch.finish", json!({}));
    let taken = cut(&mut s, json!({"distance":1.8,"operation":"cut","profiles":"all"}));
    // The slot less what the entry and neck took at its level: the neck's part outside the
    // entry (its strip less the circle's segment across it) and its round end.
    let neck = 3.2 * 10.0 - (1.6 * (16.0f64 - 1.6 * 1.6).sqrt() + 16.0 * (0.4f64).asin()) + PI * 1.6 * 1.6 / 2.0;
    let want = 1.8 * (8.0 * 10.0 - neck);
    assert!((taken - want).abs() < want * 1e-3, "{taken} taken, want {want}");
}
