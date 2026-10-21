//! `macro.*`: recording captures design commands, pause skips, replay rolls back a failure,
//! and a parameterized macro can flatten a sheet, build a plate, or chamfer small holes.

use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn error(s: &mut Session, id: &str, params: Value) -> String {
    s.execute(id, &params).map(|_| ()).err().map(|e| e.to_string()).expect("an error")
}

fn volume(s: &mut Session) -> f64 {
    run(s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap_or(f64::NAN)
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("solidcraft-macro-{}-{}-{tag}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn recording_captures_design_commands_and_pause_skips() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "macro.record", json!({"name": "Plate"}));
    run(&mut s, "solid.sphere", json!({"radius": 5}));
    run(&mut s, "macro.pause", json!({}));
    run(&mut s, "solid.cylinder", json!({"radius": 2, "height": 4}));
    run(&mut s, "macro.list", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "macro.resume", json!({}));
    run(&mut s, "parameters.add", json!({"name": "height", "expression": "3"}));
    let stop = run(&mut s, "macro.stop", json!({}));
    assert_eq!(stop["steps"], 2, "{stop}");
    assert_eq!(stop["commands"][0]["command"], "solid.sphere");
    assert_eq!(stop["commands"][1]["command"], "parameters.add");
    assert!(error(&mut s, "macro.stop", json!({})).contains("nothing is being recorded"));
    assert!(error(&mut s, "macro.pause", json!({})).contains("nothing is being recorded"));
}

#[test]
fn recording_starts_afresh_and_refuses_two_at_once() {
    let mut s = Session::default();
    run(&mut s, "macro.record", json!({}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "macro.stop", json!({}));
    run(&mut s, "macro.record", json!({}));
    assert!(error(&mut s, "macro.record", json!({})).contains("already recording"));
    assert!(error(&mut s, "macro.play", json!({})).contains("stop the recording"));
    assert_eq!(run(&mut s, "macro.stop", json!({}))["steps"], 0);
}

#[test]
fn a_playback_is_one_undo_and_a_failure_rolls_back() {
    let mut s = Session::default();
    let r = run(
        &mut s,
        "macro.play",
        json!({"commands": [{"command": "solid.box", "params": {"length": 10, "width": 10, "height": 10}}, {"command": "solid.sphere", "params": {"radius": 3}}]}),
    );
    assert_eq!(r["ran"], 2);
    assert_eq!(s.doc.features.len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.features.len(), 0);

    run(&mut s, "solid.sphere", json!({"radius": 1}));
    let before = s.doc.features.len();
    let err = error(
        &mut s,
        "macro.play",
        json!({"commands": [{"command": "solid.box", "params": {"length": 10, "width": 10, "height": 10}}, {"command": "sketch.create", "params": {"plane": "NOPE"}}]}),
    );
    assert!(err.contains("macro step 2"), "{err}");
    assert_eq!(s.doc.features.len(), before, "the box the first step made is gone");
}

#[test]
fn playback_refuses_bad_steps_before_running_any() {
    let mut s = Session::default();
    let e = error(&mut s, "macro.play", json!({"commands": [{"command": "solid.box"}, {"command": "no.such.command"}]}));
    assert!(e.contains("no.such.command"), "{e}");
    assert_eq!(s.doc.features.len(), 0);
    assert!(error(&mut s, "macro.play", json!({"commands": [{"command": "macro.record"}]})).contains("macro.record"));
    assert!(error(&mut s, "macro.play", json!({})).contains("nothing has been recorded"));
}

#[test]
fn a_macro_asks_for_its_parameters_and_reads_an_earlier_step() {
    let mut s = Session::default();
    let macro_ = json!({
        "macro": {"name": "Plate generator"},
        "params": [
            {"name": "length", "label": "Length", "default": "40 mm"},
            {"name": "width", "label": "Width"},
            {"name": "thick", "default": 5}
        ],
        "commands": [
            {"command": "solid.box", "params": {"length": "{{length}}", "width": "{{width}}", "height": "{{thick}}"}}
        ]
    });
    let dir = temp_dir("params");
    let path = dir.join("plate.json");
    std::fs::write(&path, macro_.to_string()).unwrap();
    run(&mut s, "macro.open", json!({"path": path.display().to_string()}));
    let err = error(&mut s, "macro.play", json!({}));
    assert!(err.contains("width"), "{err}");
    let r = run(&mut s, "macro.play", json!({"params": {"width": "20 mm"}}));
    assert_eq!(r["ran"], 1);
    assert!((volume(&mut s) - 40.0 * 20.0 * 5.0).abs() < 1e-3, "{}", volume(&mut s));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn find_holes_filters_by_size_feature_name_and_type() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 10}));
    run(&mut s, "solid.hole", json!({"position": [10, 10, 10], "diameter": "6 mm", "name": "Bore"}));
    run(&mut s, "solid.hole", json!({"position": [30, 20, 10], "diameter": "12 mm", "name": "Big"}));
    run(&mut s, "solid.cylinder", json!({"radius": 3, "height": 8, "base": [20, 15, 10], "operation": "new", "name": "Boss"}));

    let small = run(&mut s, "inspect.find_holes", json!({"max_diameter": "8 mm"}));
    assert_eq!(small["count"], 1, "only the 6 mm hole: {small}");
    assert_eq!(small["holes"][0]["feature"]["name"], "Bore");
    let edges = small["holes"][0]["edges"].as_array().unwrap();
    assert!(edges.len() >= 2, "a closing edge on each side: {small}");
    assert_eq!(small["edges"].as_array().unwrap().len(), edges.len());

    let named = run(&mut s, "inspect.find_holes", json!({"feature": "Bore"}));
    assert_eq!(named["count"], 1, "{named}");
    assert_eq!(named["holes"][0]["feature"]["type"], "Hole");

    let typed = run(&mut s, "inspect.find_holes", json!({"type": "Hole", "max_diameter": "20 mm"}));
    assert_eq!(typed["count"], 2, "both hole features, not the boss: {typed}");
    let large = run(&mut s, "inspect.find_holes", json!({"min_diameter": "5 mm"}));
    assert_eq!(large["count"], 2, "both holes are wider than 5 mm: {large}");
    let only_big = run(&mut s, "inspect.find_holes", json!({"min_diameter": "8 mm"}));
    assert_eq!(only_big["count"], 1, "only the 12 mm hole: {only_big}");
    assert_eq!(only_big["holes"][0]["feature"]["name"], "Big");
    assert!(error(&mut s, "inspect.find_holes", json!({"feature": "NoSuch"})).contains("NoSuch"));
    assert!(error(&mut s, "inspect.find_holes", json!({})).contains("max_diameter"));

    s.macros.loaded = Some(
        super::macros::parse_macro(
            "macro.open",
            &json!({"commands": [
                {"command": "inspect.find_holes", "params": {"max_diameter": "8 mm", "feature": "Bore"}, "let": "holes"},
                {"command": "solid.chamfer", "params": {"edges": "{{holes.edges}}", "distance": "1 mm"}}
            ]}),
        )
        .unwrap(),
    );
    let before = volume(&mut s);
    let r = run(&mut s, "macro.play", json!({}));
    assert_eq!(r["ran"], 2, "{r}");
    assert!(volume(&mut s) < before, "the chamfer cut material on both ends");
}

#[test]
fn saved_macros_write_and_hostile_names_are_refused() {
    let dir = temp_dir("save");
    let path = dir.join("box.json");
    let mut s = Session::default();
    run(&mut s, "macro.record", json!({}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "macro.stop", json!({}));
    let saved = run(&mut s, "macro.save", json!({"path": path.display().to_string()}));
    assert_eq!(saved["steps"], 1);
    let script: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(script["commands"].as_array().unwrap().len(), 1);
    for name in ["../evil.json", "a/b.json", "", "no extension"] {
        assert!(error(&mut s, "macro.save", json!({"name": name})).contains("name"), "{name}");
    }
    std::fs::write(&path, "not json").unwrap();
    assert!(error(&mut s, "macro.play", json!({"path": path.display().to_string()})).contains("command script"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_example_macros_run() {
    let mut s = Session::default();
    s.macros.loaded = Some(example("plate-generator.macro.json"));
    let r = run(&mut s, "macro.play", json!({}));
    assert_eq!(r["ran"], 5, "{r}");
    let found = run(&mut s, "inspect.find_holes", json!({"max_diameter": "7 mm"}));
    assert_eq!(found["count"], 4, "a hole in each corner: {found}");
    let v = volume(&mut s);
    let expected = 120.0 * 80.0 * 8.0 - 4.0 * std::f64::consts::PI * 3.25_f64.powi(2) * 8.0;
    assert!((v - expected).abs() < 1.0, "{v} vs {expected}");

    run(&mut s, "solid.hole", json!({"position": [60, 40, 8], "diameter": "12 mm", "name": "Big"}));
    s.macros.loaded = Some(example("chamfer-small-holes.macro.json"));
    let before = volume(&mut s);
    let r = run(&mut s, "macro.play", json!({}));
    assert_eq!(r["ran"], 2, "{r}");
    assert!(volume(&mut s) < before, "only the small holes were chamfered");
    let big = run(&mut s, "inspect.find_holes", json!({"feature": "Big"}));
    assert_eq!(big["count"], 1, "the 12 mm hole is still a hole: {big}");

    let mut t = Session::default();
    run(&mut t, "solid.box", json!({"length": 40, "width": 30, "height": 10}));
    run(&mut t, "solid.hole", json!({"position": [10, 10, 10], "diameter": "4 mm", "name": "Small"}));
    run(&mut t, "solid.hole", json!({"position": [30, 20, 10], "diameter": "8 mm", "name": "Wide"}));
    t.macros.loaded = Some(example("chamfer-large-holes.macro.json"));
    let before = volume(&mut t);
    let r = run(&mut t, "macro.play", json!({}));
    assert_eq!(r["ran"], 2, "{r}");
    assert!(volume(&mut t) < before, "the hole larger than 5 mm was chamfered");
    let small = run(&mut t, "inspect.find_holes", json!({"feature": "Small"}));
    assert_eq!(small["count"], 1, "the 4 mm hole is still a hole: {small}");

    let mut plate = Session::default();
    plate.run_script(&crate::sample::script()).unwrap();
    let found = run(&mut plate, "inspect.find_holes", json!({"min_diameter": "5 mm"}));
    assert!(found["count"].as_u64().unwrap() >= 3, "the sample plate's two Ø8 cuts and Ø12 bore: {found}");
    plate.macros.loaded = Some(example("chamfer-large-holes.macro.json"));
    let before = volume(&mut plate);
    let r = run(&mut plate, "macro.play", json!({}));
    assert_eq!(r["ran"], 2, "{r}");
    assert!(volume(&mut plate) < before, "the sample plate holes were chamfered");

    let dir = temp_dir("dxf");
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [50, 30]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sheet.flange", json!({"sketch": "Base"}));
    run(&mut s, "sheet.flange", json!({"edges": [[50, 0, 2.5]], "height": 20}));
    s.macros.loaded = Some(example("flatten-export-dxf.macro.json"));
    let path = dir.join("flat.dxf");
    let r = run(&mut s, "macro.play", json!({"params": {"path": path.display().to_string()}}));
    assert_eq!(r["ran"], 2, "{r}");
    let dxf = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(dxf.contains("ENTITIES"), "{dxf}");
}

fn example(name: &str) -> super::macros::Macro {
    let file = format!("{}/../../examples/macros/{}", env!("CARGO_MANIFEST_DIR"), name);
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file}: {e}"))).unwrap();
    super::macros::parse_macro("macro.open", &raw).unwrap()
}
