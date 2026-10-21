//! `macro.*`: recording captures design commands, pause skips, replay rolls back a failure,
//! and a Rhai macro can flatten a sheet, build a plate, or chamfer holes by size.

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
    run(&mut s, "select.set", json!({"items": []}));
    run(&mut s, "solid.sphere", json!({"radius": 5}));
    run(&mut s, "macro.pause", json!({}));
    run(&mut s, "solid.cylinder", json!({"radius": 2, "height": 4}));
    run(&mut s, "macro.list", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "macro.resume", json!({}));
    run(&mut s, "parameters.add", json!({"name": "height", "expression": "3"}));
    let stop = run(&mut s, "macro.stop", json!({}));
    assert_eq!(stop["steps"], 3, "{stop}");
    let src = s.macros.loaded.as_ref().map(|m| m.source.as_str()).unwrap_or("");
    assert!(src.contains("select.set"), "{src}");
    assert!(src.contains("solid.sphere"), "{src}");
    assert!(src.contains("parameters.add"), "{src}");
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
        json!({"source": "run(\"solid.box\", #{ length: 10, width: 10, height: 10 });\nrun(\"solid.sphere\", #{ radius: 3 });\n"}),
    );
    assert_eq!(r["ran"], true);
    assert_eq!(s.doc.features.len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.features.len(), 0);

    run(&mut s, "solid.sphere", json!({"radius": 1}));
    let before = s.doc.features.len();
    let err = error(
        &mut s,
        "macro.play",
        json!({"source": "run(\"solid.box\", #{ length: 10, width: 10, height: 10 });\nrun(\"sketch.create\", #{ plane: \"NOPE\" });\n"}),
    );
    assert!(err.contains("macro"), "{err}");
    assert_eq!(s.doc.features.len(), before, "the box the first step made is gone");
}

#[test]
fn playback_refuses_bad_steps_before_running_any() {
    let mut s = Session::default();
    let e = error(&mut s, "macro.play", json!({"source": "run(\"no.such.command\", #{});\n"}));
    assert!(e.contains("no.such.command") || e.contains("Unknown") || e.contains("macro"), "{e}");
    assert_eq!(s.doc.features.len(), 0);
    assert!(error(&mut s, "macro.play", json!({"source": "run(\"macro.record\", #{});\n"})).contains("macro"));
    assert!(error(&mut s, "macro.play", json!({})).contains("nothing has been recorded"));
}

#[test]
fn a_macro_asks_for_its_parameters_and_reads_an_earlier_step() {
    let mut s = Session::default();
    let src = "// @param length Length | 40 mm\n// @param width Width\n// @param thick Thickness | 5\nrun(\"solid.box\", #{ length: length, width: width, height: thick });\n";
    let dir = temp_dir("params");
    let path = dir.join("plate.rhai");
    std::fs::write(&path, src).unwrap();
    run(&mut s, "macro.open", json!({"path": path.display().to_string()}));
    let err = error(&mut s, "macro.play", json!({}));
    assert!(err.contains("width"), "{err}");
    let r = run(&mut s, "macro.play", json!({"params": {"width": "20 mm"}}));
    assert_eq!(r["ran"], true);
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

    s.macros.loaded = Some(super::macros::parse_rhai_macro("macro.open", r#"let holes = run("inspect.find_holes", #{ max_diameter: "8 mm", feature: "Bore" }); run("solid.chamfer", #{ edges: holes.edges, distance: "1 mm" });"#).unwrap());
    let before = volume(&mut s);
    let r = run(&mut s, "macro.play", json!({}));
    assert_eq!(r["ran"], true, "{r}");
    assert!(volume(&mut s) < before, "the chamfer cut material on both ends");
}

#[test]
fn saved_macros_write_and_hostile_names_are_refused() {
    let dir = temp_dir("save");
    let path = dir.join("box.rhai");
    let mut s = Session::default();
    run(&mut s, "macro.record", json!({}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "macro.stop", json!({}));
    let saved = run(&mut s, "macro.save", json!({"path": path.display().to_string()}));
    assert!(saved["steps"].as_u64().unwrap() >= 1, "{saved}");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("solid.box"), "{text}");
    for name in ["../evil.rhai", "a/b.rhai", "", "no extension"] {
        assert!(error(&mut s, "macro.save", json!({"name": name})).contains("name"), "{name}");
    }
    std::fs::write(&path, "").unwrap();
    assert!(
        error(&mut s, "macro.open", json!({"path": path.display().to_string()})).contains("source")
            || error(&mut s, "macro.open", json!({"path": path.display().to_string()})).contains("rhai")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn chamfer_of_no_edges_is_a_no_op() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    let r = run(&mut s, "solid.chamfer", json!({"edges": [], "distance": "1 mm"}));
    assert_eq!(r["skipped"], true, "{r}");
    assert_eq!(s.doc.features.len(), 1);
}

#[test]
fn the_start_parts_build() {
    let mut holes = Session::default();
    load_start(&mut holes, "chamfer-holes.start.json");
    let found = run(&mut holes, "inspect.find_holes", json!({"max_diameter": "20 mm"}));
    assert_eq!(found["count"], 3, "{found}");
    let mut names: Vec<_> = found["holes"].as_array().unwrap().iter().filter_map(|h| h["feature"]["name"].as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Big", "Small", "Wide"], "{found}");

    let mut sheet = Session::default();
    load_start(&mut sheet, "flatten-export-dxf.start.json");
    let m = run(&mut sheet, "inspect.measure", json!({}));
    assert_eq!(m["body_count"], 1, "{m}");
    assert_eq!(m["bodies"][0]["sheet_metal"], true, "{m}");

    let mut cube = Session::default();
    load_start(&mut cube, "batch-mass/cube.start.json");
    assert!((volume(&mut cube) - 1000.0).abs() < 1e-6, "{}", volume(&mut cube));
    let mut cyl = Session::default();
    load_start(&mut cyl, "batch-mass/cylinder.start.json");
    assert!(volume(&mut cyl) > 0.0, "{}", volume(&mut cyl));

    let mut enclosure = Session::default();
    load_start(&mut enclosure, "enclosure.start.json");
    let m = run(&mut enclosure, "inspect.measure", json!({}));
    assert!(m["body_count"].as_u64().unwrap() >= 1, "{m}");
    let holes = run(&mut enclosure, "inspect.find_holes", json!({"min_diameter": "2 mm"}));
    assert_eq!(holes["count"], 2, "two Ø2.5 bosses: {holes}");

    let mut flange = Session::default();
    load_start(&mut flange, "flange-blank.start.json");
    let m = run(&mut flange, "inspect.measure", json!({}));
    assert_eq!(m["body_count"], 1, "{m}");
    let bolts = run(&mut flange, "inspect.find_holes", json!({"max_diameter": "12 mm"}));
    assert_eq!(bolts["count"], 0, "no bolt holes: {bolts}");
    let bore = run(&mut flange, "inspect.find_holes", json!({"min_diameter": "20 mm"}));
    assert_eq!(bore["count"], 1, "the Ø24 bore: {bore}");
}

#[test]
fn the_example_macros_run() {
    let mut s = Session::default();
    s.macros.loaded = Some(example("plate-generator.rhai"));
    let r = run(&mut s, "macro.play", json!({}));
    assert_eq!(r["ran"], true, "{r}");
    let found = run(&mut s, "inspect.find_holes", json!({"max_diameter": "7 mm"}));
    assert_eq!(found["count"], 4, "a hole in each corner: {found}");
    let v = volume(&mut s);
    let expected = 120.0 * 80.0 * 8.0 - 4.0 * std::f64::consts::PI * 3.25_f64.powi(2) * 8.0;
    assert!((v - expected).abs() < 4000.0, "{v} vs {expected} holes={found}");

    let mut holes = Session::default();
    load_start(&mut holes, "chamfer-holes.start.json");
    holes.macros.loaded = Some(example("chamfer-holes.rhai"));
    let before = volume(&mut holes);
    let r = run(&mut holes, "macro.play", json!({}));
    assert_eq!(r["ran"], true, "{r}");
    assert!(volume(&mut holes) < before, "the holes between 5 mm and 20 mm were chamfered");
    let tiny = run(&mut holes, "inspect.find_holes", json!({"feature": "Small"}));
    assert_eq!(tiny["count"], 1, "the 4 mm hole is still a hole: {tiny}");

    let dir = temp_dir("dxf");
    let mut s = Session::default();
    load_start(&mut s, "flatten-export-dxf.start.json");
    s.macros.loaded = Some(example("flatten-export-dxf.rhai"));
    let path = dir.join("flat.dxf");
    let r = run(&mut s, "macro.play", json!({"params": {"path": path.display().to_string()}}));
    assert_eq!(r["ran"], true, "{r}");
    let dxf = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(dxf.contains("ENTITIES"), "{dxf}");
}

#[test]
fn the_mounting_plate_even_spacing_count() {
    let mut s = Session::default();
    s.macros.loaded = Some(example("mounting-plate.rhai"));
    run(&mut s, "macro.play", json!({"params": {"length": 200, "width": 150}}));
    let found = run(&mut s, "inspect.find_holes", json!({"min_diameter": "8 mm"}));
    assert_eq!(found["count"], 14, "even M8 ring: {found}");
}

fn examples_dir() -> String {
    format!("{}/../../examples/macros", env!("CARGO_MANIFEST_DIR"))
}

fn example(name: &str) -> super::macros::Macro {
    let file = format!("{}/{}", examples_dir(), name);
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file}: {e}"));
    super::macros::parse_rhai_macro("macro.open", &text).unwrap()
}

fn load_start(s: &mut Session, name: &str) {
    let file = format!("{}/{}", examples_dir(), name);
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file}: {e}"))).unwrap();
    s.run_script(&raw).unwrap_or_else(|e| panic!("{name}: {e}"));
}
