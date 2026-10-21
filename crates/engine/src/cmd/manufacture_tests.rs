use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn plate(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Outline"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0.0, 0.0], "p1": [40.0, 30.0]}));
    run(s, "sketch.finish", json!({}));
    run(s, "sketch.create", json!({"plane": "XY", "name": "Holes"}));
    run(s, "sketch.circle.center", json!({"center": [10.0, 10.0], "diameter": 6.0}));
    run(s, "sketch.circle.center", json!({"center": [30.0, 20.0], "diameter": 6.0}));
    run(s, "sketch.finish", json!({}));
}

fn setup_job(s: &mut Session) {
    run(s, "manufacture.tool.set", json!({"id": 1, "diameter": 6.0}));
    run(s, "manufacture.tool.set", json!({"id": 2, "kind": "drill", "diameter": 6.0}));
    run(s, "manufacture.setup.set", json!({"id": 1, "plane": "XY", "stock": {"min": [-5.0, -5.0, -3.0], "max": [45.0, 35.0, 0.0]}}));
}

#[test]
fn job_round_trip_with_undo() {
    let mut s = Session::default();
    plate(&mut s);
    setup_job(&mut s);
    run(
        &mut s,
        "manufacture.operation.set",
        json!({"setup": 1, "tool": 1, "kind": {"type": "contour", "source": {"sketch": "Outline"}, "side": "outside"}, "top": 0.0, "bottom": -3.0, "feed": 600.0, "spindle_rpm": 8000.0}),
    );
    let job = run(&mut s, "manufacture.job", json!({}));
    assert_eq!(job["operations"].as_array().map(Vec::len), Some(1));
    assert_eq!(job["tools"].as_array().map(Vec::len), Some(2));
    run(&mut s, "edit.undo", json!({}));
    let job = run(&mut s, "manufacture.job", json!({}));
    assert_eq!(job["operations"].as_array().map(Vec::len), Some(0));
}

#[test]
fn toolpath_and_post_all_dialects() {
    let mut s = Session::default();
    plate(&mut s);
    setup_job(&mut s);
    run(
        &mut s,
        "manufacture.operation.set",
        json!({"setup": 1, "tool": 1, "kind": {"type": "contour", "source": {"sketch": "Outline"}, "side": "outside"}, "top": 0.0, "bottom": -3.0, "feed": 600.0, "spindle_rpm": 8000.0}),
    );
    run(
        &mut s,
        "manufacture.operation.set",
        json!({"setup": 1, "tool": 2, "kind": {"type": "drill", "points": {"from": "sketch", "sketch": "Holes"}}, "top": 0.0, "bottom": -3.0, "feed": 200.0, "spindle_rpm": 3000.0}),
    );
    let t = run(&mut s, "manufacture.toolpath", json!({"operation": 2, "moves": true}));
    assert!(t["moves"].as_u64().unwrap() >= 2);
    for d in ["grbl", "fanuc", "haas"] {
        let r = run(&mut s, "manufacture.post", json!({"dialect": d}));
        assert!(r["gcode"].as_str().unwrap().contains("M30") || r["gcode"].as_str().unwrap().contains("M5"), "{d}");
    }
}

#[test]
fn hostile_params_are_errors() {
    let mut s = Session::default();
    plate(&mut s);
    setup_job(&mut s);
    for (id, p) in [
        ("manufacture.tool.set", json!({"diameter": -1.0})),
        ("manufacture.tool.set", json!({"diameter": 1e300})),
        ("manufacture.tool.set", json!({"id": 0, "diameter": 3.0})),
        ("manufacture.tool.set", json!({"diameter": 3.0, "bogus": 1})),
        ("manufacture.tool.delete", json!({"id": 99})),
        ("manufacture.setup.set", json!({"plane": "QQ"})),
        ("manufacture.setup.set", json!({"clearance": 1e308})),
        (
            "manufacture.operation.set",
            json!({"setup": 1, "tool": 1, "kind": {"type": "face"}, "top": 0.0, "bottom": 1.0, "feed": 1.0, "spindle_rpm": 1.0}),
        ),
        ("manufacture.operation.set", json!({"setup": 9, "tool": 1})),
        ("manufacture.toolpath", json!({"operation": 99})),
        ("manufacture.post", json!({"dialect": "mach3"})),
        ("manufacture.post", json!({"dialect": "grbl"})),
        ("manufacture.post", json!([])),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
}

#[test]
fn delete_refused_while_used() {
    let mut s = Session::default();
    plate(&mut s);
    setup_job(&mut s);
    run(
        &mut s,
        "manufacture.operation.set",
        json!({"setup": 1, "tool": 1, "kind": {"type": "face"}, "top": 0.0, "bottom": -1.0, "feed": 600.0, "spindle_rpm": 8000.0}),
    );
    assert!(s.execute("manufacture.tool.delete", &json!({"id": 1})).is_err());
    assert!(s.execute("manufacture.setup.delete", &json!({"id": 1})).is_err());
    run(&mut s, "manufacture.operation.delete", json!({"id": 1}));
    run(&mut s, "manufacture.tool.delete", json!({"id": 1}));
}
