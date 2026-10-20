//! Split Body made while another component is active: every piece stays in the component the
//! body belongs to (an imported part split from the design's root, #2979), not only the first.

use super::*;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

/// A 10 mm block in a new component, then the design's root active again.
fn block_in_component(s: &mut Session) -> (u64, u64) {
    let created = run(s, "component.create", json!({"name": "Part"}));
    let component = created["component"].as_u64().unwrap();
    let occurrence = created["occurrence"].as_u64().unwrap();
    run(s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Block"}));
    run(s, "component.activate", json!({"component": 0}));
    (component, occurrence)
}

fn owners(s: &Session) -> Vec<(String, u64)> {
    let st = s.model.state();
    let mut v: Vec<(String, u64)> = st.bodies.iter().map(|b| (b.name.clone(), s.doc.model_body_component(b))).collect();
    v.sort();
    v
}

#[test]
fn both_pieces_stay_in_the_component_of_the_split_body() {
    let mut s = Session::default();
    let (component, _) = block_in_component(&mut s);
    run(&mut s, "solid.split_body", json!({"body": "Block", "plane": {"origin": [4, 0, 0], "normal": [1, 0, 0]}}));
    assert_eq!(owners(&s), [("Block".to_string(), component), ("Block (1)".to_string(), component)]);
    // The design's owner sets agree.
    let st = s.model.state();
    assert_eq!(s.doc.body_owners(&st), std::collections::HashSet::from([component]));
}

#[test]
fn a_piece_of_a_body_moved_into_a_component_stays_there() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Block"}));
    let created = run(&mut s, "component.from_bodies", json!({"bodies": ["Block"]}));
    let component = created["components"][0]["component"].as_u64().unwrap();
    run(&mut s, "component.activate", json!({"component": 0}));
    run(&mut s, "solid.split_body", json!({"body": "Block", "plane": {"origin": [4, 0, 0], "normal": [1, 0, 0]}}));
    assert_eq!(owners(&s), [("Block".to_string(), component), ("Block (1)".to_string(), component)]);
}

#[test]
fn the_pieces_are_shown_where_the_body_was() {
    let mut s = Session::default();
    let (_, occurrence) = block_in_component(&mut s);
    run(&mut s, "occurrence.move", json!({"occurrence": occurrence, "translate": [30, 0, 0], "capture": false}));
    let before = s.world_state().body("Block").unwrap().mesh().bounds();
    run(&mut s, "solid.split_body", json!({"body": "Block", "plane": {"origin": [34, 0, 0], "normal": [1, 0, 0]}}));
    let world = s.world_state();
    let (a, b) = (world.body("Block").unwrap().mesh().bounds(), world.body("Block (1)").unwrap().mesh().bounds());
    // Together the pieces fill the block where it was shown, each on one side of the cut.
    for p in [a, b] {
        assert!(p.min.x >= before.min.x - 1e-6 && p.max.x <= before.max.x + 1e-6, "{p:?} outside {before:?}");
    }
    assert!((a.min.x.min(b.min.x) - before.min.x).abs() < 1e-6 && (a.max.x.max(b.max.x) - before.max.x).abs() < 1e-6);
}
