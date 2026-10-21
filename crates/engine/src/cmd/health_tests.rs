//! `model.health`: the design's failures, warnings and history in brief.
#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn health_lists_failing_features_and_the_history() {
    let mut s = Session::default();
    let h = run(&mut s, "model.health", json!({}));
    assert_eq!((h["features"].clone(), h["errors"].clone(), h["undo_depth"].clone()), (json!(0), json!([]), json!(0)), "{h}");
    run(&mut s, "parameters.add", json!({"name": "L", "expression": "20", "unit": "mm"}));
    run(&mut s, "solid.box", json!({"length": "L", "width": 10, "height": 10}));
    run(&mut s, "solid.fillet", json!({"edges": [[20, 5, 10]], "radius": 3}));
    let h = run(&mut s, "model.health", json!({}));
    assert_eq!(h["errors"], json!([]), "{h}");
    assert_eq!(h["undo"].as_array().unwrap().last(), Some(&json!("Fillet")), "{h}");
    // The box too short for its fillet's edge: the fillet fails, and says so.
    run(&mut s, "parameters.change", json!({"name": "L", "expression": "2"}));
    let h = run(&mut s, "model.health", json!({}));
    let errors = h["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{h}");
    assert!(errors[0]["name"].as_str().unwrap().starts_with("Fillet") && errors[0]["error"].is_string(), "{h}");
    assert_eq!(h["dirty"], json!(true), "{h}");
    // Reading it changes nothing.
    let again = run(&mut s, "model.health", json!({}));
    assert_eq!(again["undo"], h["undo"]);
    assert_eq!(again["undo_depth"], h["undo_depth"]);
    run(&mut s, "edit.undo", json!({}));
    let h = run(&mut s, "model.health", json!({}));
    assert_eq!((h["errors"].clone(), h["redo_depth"].clone()), (json!([]), json!(1)), "{h}");
}

#[test]
fn notices_report_what_each_feature_says_about_itself() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [20, 10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance": 5}));
    let n = run(&mut s, "model.notices", json!({}));
    let all = n["notices"].as_array().unwrap();
    assert_eq!(all.len(), 2, "{n}");
    // The sketch says how far it is constrained; the extrude has nothing to add.
    let sketch = &all[0];
    assert_eq!((sketch["name"].clone(), sketch["status"].clone()), (json!("Base"), json!("ok")), "{n}");
    assert_eq!(sketch["values"]["profiles"], 1, "{n}");
    assert!(sketch["values"]["dof"].as_u64().unwrap() > 0, "an undimensioned rectangle moves: {n}");
    assert!(sketch["info"].as_str().unwrap().contains("degrees of freedom left"), "{n}");
    // Its issues are objects an agent can act on: here, the freedom left (severity info).
    let free = &sketch["issues"][0];
    assert_eq!(
        (free["severity"].clone(), free["code"].clone(), free["limit"].clone()),
        (json!("info"), json!("sketch_underconstrained"), json!(0)),
        "{n}"
    );
    assert_eq!(free["value"], sketch["values"]["dof"], "{n}");
    assert!(all[1].get("info").is_none() && all[1].get("issues").is_none(), "{n}");
    // By name or id; an unknown name is refused.
    let one = run(&mut s, "model.notices", json!({"features": ["Base"]}));
    assert_eq!(one["notices"].as_array().unwrap().len(), 1, "{one}");
    assert!(s.execute("model.notices", &json!({"features": ["Nope"]})).is_err());
    // model.health carries a digest of each notice, which moves when the notice does.
    let before = run(&mut s, "model.health", json!({}))["notices"].clone();
    assert_eq!(before.as_array().unwrap().len(), 1, "{before}");
    run(&mut s, "sketch.edit", json!({"sketch": "Base"}));
    run(&mut s, "sketch.dimension", json!({"entities": ["l1"], "value": 20}));
    run(&mut s, "sketch.finish", json!({}));
    let after = run(&mut s, "model.health", json!({}))["notices"].clone();
    assert_ne!(before[0]["digest"], after[0]["digest"], "{before} {after}");
}

#[test]
fn notice_issues_say_where_a_sketch_is_open_and_why_a_feature_failed() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Open"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [10, 0], [10, 5]]}));
    run(&mut s, "sketch.finish", json!({}));
    let n = run(&mut s, "model.notices", json!({"features": ["Open"]}));
    let issues = n["notices"][0]["issues"].as_array().unwrap();
    let open = issues.iter().find(|i| i["code"] == "sketch_open_profile").unwrap_or_else(|| panic!("{n}"));
    assert_eq!(open["severity"], "warning", "{n}");
    assert_eq!(open["entities"].as_array().unwrap().len(), 2, "two open ends: {n}");
    assert_eq!(open["at"].as_array().unwrap().len(), 2, "{n}");
    assert_eq!(n["notices"][0]["values"]["open_ends"], 2, "{n}");
    // A failing feature's error is an issue too.
    run(&mut s, "parameters.add", json!({"name": "L", "expression": "20", "unit": "mm"}));
    run(&mut s, "solid.box", json!({"length": "L", "width": 10, "height": 10, "corner": [50, 0, 0]}));
    run(&mut s, "solid.fillet", json!({"edges": [[70, 5, 10]], "radius": 3}));
    run(&mut s, "parameters.change", json!({"name": "L", "expression": "2"}));
    let n = run(&mut s, "model.notices", json!({}));
    let failed = n["notices"].as_array().unwrap().iter().find(|x| x["status"] == "error").unwrap_or_else(|| panic!("{n}"));
    assert_eq!((failed["issues"][0]["severity"].clone(), failed["issues"][0]["code"].clone()), (json!("error"), json!("feature_error")), "{n}");
    assert!(failed["issues"][0]["message"].as_str().is_some_and(|m| !m.is_empty()), "{n}");
}
