use serde_json::json;

use super::*;
use crate::context_menu::{self, Target};
use crate::scenario::Harness;

fn harness_with_body() -> Harness {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 40, "width": 20, "height": 10, "body_name": "Duct"})).unwrap();
    h.app.session.mark_saved();
    h.frames(3);
    h
}

/// The design as saved, plus its history: what a workspace switch must never change.
fn design_of(h: &Harness) -> (String, usize, usize, bool) {
    let s = &h.app.session;
    (serde_json::to_string(&*s.doc).unwrap(), s.undo.len(), s.redo.len(), s.is_dirty())
}

fn workspace_item<'a>(items: &'a [context_menu::Item], label: &str) -> &'a context_menu::Item {
    items.iter().find(|i| i.label == label).unwrap_or_else(|| panic!("no `{label}` in the workspace menu"))
}

#[test]
fn ids_tabs_and_workspaces_round_trip() {
    for w in Workspace::ALL {
        assert_eq!(Workspace::from_id(w.id()), Some(w));
        assert_eq!(Workspace::of_tab(w.tab()), w);
    }
    assert_eq!(Workspace::of_tab("SURFACE"), Workspace::Design);
    assert_eq!(Workspace::of_tab("SKETCH"), Workspace::Design);
    assert_eq!(Workspace::from_id("animation"), None);
}

#[test]
fn the_simulation_menu_item_is_enabled_like_render_and_marks_the_current_workspace() {
    let mut h = harness_with_body();
    let items = context_menu::items(&h.app, &Target::Workspace);
    assert!(workspace_item(&items, "Simulation").enabled);
    assert_eq!(workspace_item(&items, "Simulation").id, "ui.workspace.simulation");
    assert_eq!(workspace_item(&items, "Design").shortcut, "current");
    for later in ["Animation", "Manufacture", "Drawing"] {
        assert!(!workspace_item(&items, later).enabled, "{later} is still to come");
    }
    assert_eq!(items.iter().filter(|i| i.label == "Simulation").count(), 1, "no second, greyed Simulation entry");

    // A sketch being edited holds Render and Simulation back alike.
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    let items = context_menu::items(&h.app, &Target::Workspace);
    assert!(!workspace_item(&items, "Simulation").enabled);
    assert!(!workspace_item(&items, "Render").enabled);
    assert!(workspace_item(&items, "Design").enabled);
    assert!(enter(&mut h.app, Workspace::Simulation).is_err());
    assert_eq!(current(&h.app), Workspace::Design);
}

#[test]
fn design_to_simulation_and_back_leaves_the_design_untouched() {
    let mut h = harness_with_body();
    let before = design_of(&h);
    let camera = h.app.cam;
    let at = crate::scenario::rect_of("workspace_switcher").unwrap().center();
    h.step(&json!({"click": [at.x, at.y]})).unwrap();
    h.step(&json!({"click": {"handle": "menu:ui.workspace.simulation"}})).unwrap();
    assert_eq!(current(&h.app), Workspace::Simulation);
    assert_eq!(h.app.ui.tab, "SIMULATION");
    assert!(crate::scenario::rect_of("toolbar_tab:SIMULATION").is_some(), "the SIMULATION tab is shown");
    assert!(crate::scenario::rect_of("toolbar_tab:SOLID").is_none(), "Design's tabs are not");
    assert_eq!(design_of(&h), before);

    let reply = h.call("ui.workspace", json!({"workspace": "design"}));
    assert_eq!(reply["result"]["workspace"], "design", "{reply}");
    h.frames(2);
    assert_eq!(current(&h.app), Workspace::Design);
    assert_eq!(h.app.ui.tab, "SOLID");
    assert_eq!(h.app.cam, camera);
    assert_eq!(design_of(&h), before);
}

#[test]
fn control_channel_names_the_simulation_workspace() {
    let mut h = harness_with_body();
    let reply = h.call("ui.workspace", json!({"workspace": "simulation"}));
    assert_eq!(reply["result"]["workspace"], "simulation", "{reply}");
    assert_eq!(h.call("ui.workspace", json!({}))["result"]["workspace"], "simulation");
    let wrong = h.call("ui.workspace", json!({"workspace": "animation"}));
    assert_eq!(wrong["error"], "workspace must be design, render or simulation", "{wrong}");
    // `ui.set {tab}` switches workspaces as the menu does, both ways.
    h.call("ui.set", json!({"tab": "SURFACE"}));
    assert_eq!((current(&h.app), h.app.ui.tab.as_str()), (Workspace::Design, "SURFACE"));
    h.call("ui.set", json!({"tab": "SIMULATION"}));
    assert_eq!(current(&h.app), Workspace::Simulation);
}

#[test]
fn both_studies_open_their_docked_dialog_and_ok_reports_the_missing_solver() {
    let mut h = harness_with_body();
    let before = design_of(&h);
    enter(&mut h.app, Workspace::Simulation).unwrap();
    h.frames(2);
    for (id, title) in [("sim.flow.new", "AIRFLOW STUDY"), ("sim.gas.new", "GAS DYNAMICS STUDY")] {
        h.step(&json!({"start": id})).unwrap();
        h.frames(3);
        let d = h.app.dialog.as_ref().unwrap_or_else(|| panic!("{id} opens a dialog"));
        assert_eq!(crate::dialogs::title(&d.kind), title);
        h.step(&json!({"expect": {"dialog_docked": {}}})).unwrap();
        assert!(h.widget(crate::i18n::tr("sim.solver_not_built")).is_some(), "{id}: the note says the solver is not built");
        // OK with nothing picked asks for the body; with the body picked the command refuses.
        h.step(&json!({"widget": "OK"})).unwrap();
        h.frames(2);
        let error = h.app.dialog.as_ref().and_then(|d| d.error.clone()).unwrap_or_default();
        assert!(error.contains("select the body to study"), "{id}: {error}");
        h.step(&json!({"click": {"world": [20, 10, 10]}})).unwrap();
        h.step(&json!({"widget": "OK"})).unwrap();
        h.frames(2);
        let refused = h.app.dialog.as_ref().and_then(|d| d.error.clone()).or_else(|| h.app.status.as_ref().map(|s| s.0.clone()));
        assert!(refused.as_deref().is_some_and(|e| e.contains("not supported yet")), "{id}: {refused:?}");
        crate::dialogs::cancel(&mut h.app);
        h.frames(2);
        assert!(h.app.dialog.is_none());
    }
    assert_eq!(design_of(&h), before, "an unbuilt study changes nothing");
}

#[test]
fn leaving_simulation_closes_an_open_study_dialog() {
    let mut h = harness_with_body();
    enter(&mut h.app, Workspace::Simulation).unwrap();
    h.step(&json!({"start": "sim.gas.new"})).unwrap();
    assert!(h.app.dialog.is_some());
    // A dialog is open, so Render waits; Design is always reachable and closes the study.
    assert!(enter(&mut h.app, Workspace::Render).is_err());
    enter(&mut h.app, Workspace::Design).unwrap();
    assert!(h.app.dialog.is_none());
    assert_eq!(current(&h.app), Workspace::Design);
}

#[test]
fn render_and_simulation_switch_directly_and_design_gets_its_camera_back() {
    let mut h = harness_with_body();
    let camera = h.app.cam;
    enter(&mut h.app, Workspace::Render).unwrap();
    enter(&mut h.app, Workspace::Simulation).unwrap();
    assert_eq!(current(&h.app), Workspace::Simulation);
    assert_eq!(h.app.cam, camera, "Simulation shows the Design view");
    enter(&mut h.app, Workspace::Render).unwrap();
    enter(&mut h.app, Workspace::Design).unwrap();
    assert_eq!(h.app.cam, camera);
}
