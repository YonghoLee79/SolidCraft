//! A round face or a circular edge picked where a tool wants an axis stands for that axis (the
//! cylinder's, or the circle's through its centre and square to its plane), as it already did
//! for Revolve: Circular Pattern, Move's Rotate and Surface Revolve take them
//! too, and the direction inputs take their axis's direction.

use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::Session;
use solidcraft_engine::geom::Vec3;

use crate::dialogs::{apply_commands, face_sel};
use crate::{Services, SolveApp};

/// A hub (a cylinder of radius 5 standing on (50, 0, 0), 20 high) and a 4 mm tab whose centre is
/// 12 mm from the hub's axis along +x.
fn hub_and_tab() -> SolveApp {
    let mut s = Session::default();
    for (id, p) in [
        ("solid.cylinder", json!({"radius": 5, "height": 20, "base": [50, 0, 0], "body_name": "Hub"})),
        ("solid.box", json!({"length": 4, "width": 4, "height": 4, "corner": [60, -2, 0], "body_name": "Tab"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    SolveApp::new(s, Services::default())
}

/// The hub's top rim: its circular edge at z = 20.
fn rim(app: &SolveApp) -> Sel {
    let st = app.session.world_state();
    let m = st.body("Hub").unwrap().mesh();
    let (index, e) = m.edges.iter().enumerate().find(|(_, e)| e.len() > 8 && e.iter().all(|p| (p.z - 20.0).abs() < 1e-6)).expect("a rim");
    Sel::Edge { body: "Hub".into(), index, point: e[0] }
}

fn side(app: &SolveApp) -> Sel {
    face_sel(&app.session, Vec3::new(55.0, 0.0, 10.0)).expect("the hub's side")
}

/// The axis parameter is the hub's: through (50, 0), along ±z.
fn assert_hub_axis(axis: &Value) {
    let v = |k: &str, i: usize| axis[k][i].as_f64().unwrap_or(f64::NAN);
    assert!((v("origin", 0) - 50.0).abs() < 1e-6 && v("origin", 1).abs() < 1e-6, "{axis}");
    assert!(v("dir", 0).abs() < 1e-9 && v("dir", 1).abs() < 1e-9 && (v("dir", 2).abs() - 1.0).abs() < 1e-9, "{axis}");
}

/// The pick a selection came from.
fn hit(sel: &Sel) -> crate::viewport::Hit {
    match sel {
        Sel::Face { body, index, point } => crate::viewport::Hit::Face { body: body.clone(), index: *index, point: *point },
        Sel::Edge { body, index, point } => crate::viewport::Hit::Edge { body: body.clone(), index: *index, mid: *point },
        _ => panic!("{sel:?}"),
    }
}

fn bounds(app: &SolveApp) -> solidcraft_engine::geom::Aabb3 {
    app.session
        .world_state()
        .bodies
        .iter()
        .filter(|b| b.name != "Hub")
        .fold(solidcraft_engine::geom::Aabb3::EMPTY, |a, b| a.union(&b.mesh().bounds()))
}

fn run(app: &mut SolveApp, cmds: Vec<(String, Value)>) {
    for (id, p) in cmds {
        app.run(&id, p.clone()).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }
}

#[test]
fn a_circular_pattern_turns_about_a_round_face_or_a_circular_edge() {
    for by_rim in [false, true] {
        let mut app = hub_and_tab();
        app.session.selection = vec![Sel::Body { name: "Tab".into() }];
        app.start("solid.pattern.circular");
        let axis = if by_rim { rim(&app) } else { side(&app) };
        let mut d = app.dialog.clone().unwrap();
        assert!(d.inputs[1].accepts(&hit(&axis), |_, _| false).is_some(), "the Axis input takes {axis:?}");
        d.inputs[1].items = vec![axis];
        if let crate::dialogs::Kind::PatternCirc { count, .. } = &mut d.kind {
            *count = "4".into();
        }
        let cmds = apply_commands(&app, &d).unwrap();
        assert_hub_axis(&cmds[0].1["axis"]);
        run(&mut app, cmds);
        // Four tabs a quarter turn apart round the hub: 12 ± 2 mm out on ±x and ±y.
        let b = bounds(&app);
        for (got, want) in [(b.min.x, 36.0), (b.max.x, 64.0), (b.min.y, -14.0), (b.max.y, 14.0)] {
            assert!((got - want).abs() < 1e-6, "by the rim: {by_rim}: {b:?}");
        }
    }
}

#[test]
fn a_rotate_move_turns_about_a_round_face() {
    let mut app = hub_and_tab();
    app.session.selection = vec![Sel::Body { name: "Tab".into() }];
    app.start("solid.move");
    let mut d = app.dialog.clone().unwrap();
    d.kind = crate::dialogs_move::kind(crate::dialogs_move::ROTATE);
    d.inputs = crate::dialogs_move::inputs(crate::dialogs_move::ROTATE);
    d.inputs[0].items = vec![Sel::Body { name: "Tab".into() }];
    d.inputs[1].items = vec![side(&app)];
    if let crate::dialogs::Kind::Move { angle, .. } = &mut d.kind {
        *angle = "90 deg".into();
    }
    let cmds = apply_commands(&app, &d).unwrap();
    run(&mut app, cmds);
    // A quarter turn about the hub's axis: the tab now sits 12 mm out along ±y.
    let b = bounds(&app);
    assert!((b.min.x - 48.0).abs() < 1e-6 && (b.max.x - 52.0).abs() < 1e-6 && (b.min.y.abs() - 10.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn a_linear_pattern_runs_along_a_round_face_s_axis() {
    let mut app = hub_and_tab();
    app.session.selection = vec![Sel::Body { name: "Tab".into() }];
    app.start("solid.pattern.rectangular");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[1].items = vec![side(&app)];
    if let crate::dialogs::Kind::PatternRect { count, spacing, .. } = &mut d.kind {
        *count = "3".into();
        *spacing = "10 mm".into();
    }
    let cmds = apply_commands(&app, &d).unwrap();
    run(&mut app, cmds);
    // Three tabs stacked along the hub's axis, 10 apart.
    let b = bounds(&app);
    assert!((b.max.z - b.min.z - 24.0).abs() < 1e-6 && (b.max.x - 64.0).abs() < 1e-6, "{b:?}");
}

fn body_bounds(app: &SolveApp, name: &str) -> solidcraft_engine::geom::Aabb3 {
    app.session.world_state().body(name).unwrap_or_else(|| panic!("no {name}")).mesh().bounds()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Align with the dialog's picks in place of the user's clicks.
fn align(app: &SolveApp, body: &str, from: Sel, to: Sel, slide: bool) -> Vec<(String, Value)> {
    let mut d = crate::dialogs::Dialog::for_command(app, "solid.align").unwrap();
    d.inputs[0].items = vec![Sel::Body { name: body.into() }];
    d.inputs[1].items = vec![from];
    d.inputs[2].items = vec![to];
    if slide {
        d.extra.insert("slide".into(), json!(true));
    }
    apply_commands(app, &d).unwrap()
}

/// Face onto face, the tab turns over and comes onto the hub's top in one direction only (up),
/// staying where it was across; asked to, it also slides over to the point picked on the hub.
#[test]
fn align_face_onto_face_moves_only_along_the_normal() {
    for slide in [false, true] {
        let mut app = hub_and_tab();
        let (from, to) = (face_sel(&app.session, Vec3::new(62.0, 0.0, 4.0)).unwrap(), face_sel(&app.session, Vec3::new(52.0, 1.0, 20.0)).unwrap());
        let cmds = align(&app, "Tab", from, to, slide);
        run(&mut app, cmds);
        let b = body_bounds(&app, "Tab");
        assert!(near(b.min.z, 20.0) && near(b.max.z, 24.0), "on the hub's top: {b:?}");
        let (cx, cy) = (0.5 * (b.min.x + b.max.x), 0.5 * (b.min.y + b.max.y));
        if slide {
            assert!(near(cx, 52.0) && near(cy, 1.0), "slid over to the picked point: {b:?}");
        } else {
            assert!(near(cx, 62.0) && near(cy, 0.0), "only lifted: {b:?}");
        }
    }
}

/// Rim onto rim: a peg's bottom circle onto the hub's top circle puts the peg's axis on the
/// hub's, standing on it, whichever point of each rim was clicked.
#[test]
fn align_rim_onto_rim_lines_up_the_circles_centres() {
    let mut app = hub_and_tab();
    app.run("solid.cylinder", json!({"radius": 2, "height": 6, "base": [80, 0, 0], "body_name": "Peg"})).unwrap();
    let st = app.session.world_state();
    let m = st.body("Peg").unwrap().mesh();
    let (index, e) = m.edges.iter().enumerate().find(|(_, e)| e.len() > 8 && e.iter().all(|p| p.z.abs() < 1e-6)).expect("the peg's bottom rim");
    let peg = Sel::Edge { body: "Peg".into(), index, point: e[e.len() / 3] };
    let cmds = align(&app, "Peg", peg, rim(&app), false);
    run(&mut app, cmds);
    // To the facets' accuracy across (the bounds of a faceted circle), exactly along the axis.
    let b = body_bounds(&app, "Peg");
    let close = |a: f64, b: f64| (a - b).abs() < 1e-2;
    assert!(close(b.min.x, 48.0) && close(b.max.x, 52.0) && close(b.min.y, -2.0) && near(b.min.z, 20.0) && near(b.max.z, 26.0), "{b:?}");
}

/// Move's pivot picked on a rim is the rim's centre, and the triad sits there.
#[test]
fn a_move_pivot_on_a_rim_is_its_centre() {
    let mut app = hub_and_tab();
    app.session.selection = vec![Sel::Body { name: "Tab".into() }];
    app.start("solid.move");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[1].items = vec![rim(&app)];
    let t = crate::dialogs_move::triad(&app, &d).expect("a triad");
    assert!(t.center.dist(Vec3::new(50.0, 0.0, 20.0)) < 1e-6, "{:?}", t.center);
}

/// On a part in a turned component (an imported plate set upright), Move's triad sits at the
/// part's centre where the view shows it, not in the component's own frame.
#[test]
fn the_move_triad_sits_on_a_turned_component_s_part() {
    let mut s = Session::default();
    let made = s.execute("component.create", &json!({"name": "Plate"})).unwrap();
    s.execute("solid.box", &json!({"length": 20, "width": 8, "height": 30, "corner": [0, 0, -30], "body_name": "Plate"})).unwrap();
    s.execute("occurrence.move", &json!({"occurrence": made["occurrence"], "axis": [1, 0, 0], "angle": "90 deg"})).unwrap();
    s.execute("component.activate", &json!({"component": 0})).unwrap();
    let mut app = SolveApp::new(s, Services::default());
    app.session.selection = vec![Sel::Body { name: "Plate".into() }];
    app.start("solid.move");
    let d = app.dialog.clone().unwrap();
    let t = crate::dialogs_move::triad(&app, &d).expect("a triad");
    assert!(t.center.dist(Vec3::new(10.0, 15.0, 4.0)) < 1e-6, "at the plate's centre as shown: {:?}", t.center);
}
