//! The Move window's main value: the one the canvas box shows and the keyboard types into.
//! It follows the move type, because each type uses a different value.
use serde_json::json;
use solidcraft_engine::Session;

use super::*;
use crate::Services;
use crate::dialogs::Dialog;

fn move_dialog(mode: usize) -> Dialog {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 20, "width": 20, "height": 20})).unwrap();
    let mut app = SolveApp::new(s, Services::default());
    app.start("solid.move");
    let mut d = app.dialog.clone().unwrap();
    d.kind = kind(mode);
    d.inputs = inputs(mode);
    if let Some(inp) = d.inputs.first_mut() {
        inp.items = vec![Sel::Body { name: "Body1".into() }];
    }
    d
}

#[test]
fn the_main_value_follows_the_move_type() {
    for mode in [FREE, TRANSLATE, POINT_TO_POSITION] {
        let mut d = move_dialog(mode);
        let got = d.primary().map(|p| (p.0, p.1));
        assert_eq!(got, Some(("X", ValueKind::Length)), "{} types a distance", TYPES[mode]);
    }

    let mut d = move_dialog(ROTATE);
    let got = d.primary().map(|p| (p.0, p.1));
    assert_eq!(got, Some(("Angle", ValueKind::Angle)), "Rotate types an angle, not an X distance");

    let mut d = move_dialog(POINT_TO_POINT);
    assert!(d.primary().is_none(), "Point to Point is two picks with nothing to type");
}

#[test]
fn the_main_value_writes_through_to_the_command() {
    let mut d = move_dialog(ROTATE);
    if let Some((_, _, value)) = d.primary() {
        *value = "90 deg".into();
    }
    let Kind::Move { angle, .. } = &d.kind else { panic!("not a move") };
    assert_eq!(angle, "90 deg", "the box edits the angle itself, not a copy of it");

    let mut d = move_dialog(TRANSLATE);
    if let Some((_, _, value)) = d.primary() {
        *value = "30 mm".into();
    }
    let Kind::Move { x, y, z, .. } = &d.kind else { panic!("not a move") };
    assert_eq!((x.as_str(), y.as_str(), z.as_str()), ("30 mm", "0 mm", "0 mm"), "the box edits X alone");
}

/// Free Move and Translate carry the triad. The other three have no manipulator yet, so their
/// value box is the only way to drive them: it must still have somewhere to sit.
#[test]
fn every_move_type_has_somewhere_to_put_its_value_box() {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 20, "width": 20, "height": 20})).unwrap();
    let app = SolveApp::new(s, Services::default());
    for mode in [FREE, TRANSLATE, ROTATE, POINT_TO_POINT, POINT_TO_POSITION] {
        let d = move_dialog(mode);
        let has_triad = triad(&app, &d).is_some();
        assert_eq!(has_triad, matches!(mode, FREE | TRANSLATE), "{} triad", TYPES[mode]);
        let at = triad(&app, &d).map(|t| t.center).or_else(|| crate::canvas::anchor(&app, &d).map(|(base, _)| base));
        assert!(at.is_some(), "{} has nowhere to put its value box", TYPES[mode]);
    }
}

/// #3486: Point to Point from a pin's bottom rim to a bushing's top rim put the pin on the rim,
/// where the edges were clicked; a round edge stands for its centre, so the pin goes into it.
/// A straight edge is still taken where it was picked.
#[test]
fn point_to_point_from_round_edges_moves_centre_to_centre() {
    let mut s = Session::default();
    s.execute("solid.cylinder", &json!({"radius": 5, "height": 20, "base": [0, 0, 0], "operation": "new_body"})).unwrap();
    s.execute("solid.box", &json!({"length": 30, "width": 30, "height": 10, "corner": [40, -15, 0], "operation": "new_body"})).unwrap();
    s.execute("solid.cylinder", &json!({"radius": 5, "height": 10, "base": [55, 0, 0], "operation": "cut"})).unwrap();
    let mut app = SolveApp::new(s, Services::default());
    app.start("solid.move");
    let mut d = app.dialog.clone().unwrap();
    let st = app.session.model.state();
    let names: Vec<String> = st.bodies.iter().map(|b| b.name.clone()).collect();
    // A mesh edge of a body all of whose points satisfy `on`, and a point on it.
    let edge = |body: &str, on: &dyn Fn(Vec3) -> bool| -> Sel {
        let m = st.body(body).unwrap().mesh();
        let (index, pts) = m.edges.iter().enumerate().find(|(_, e)| e.iter().all(|p| on(*p))).unwrap();
        Sel::Edge { body: body.into(), index, point: pts[pts.len() / 2] }
    };
    let rim = |c: Vec3, z: f64| move |p: Vec3| (p.z - z).abs() < 1e-6 && ((p - c).len() - 5.0).abs() < 1e-6;
    let (pin, block) = (names[0].as_str(), names[1].as_str());
    let from = edge(pin, &rim(Vec3::new(0.0, 0.0, 0.0), 0.0));
    let to = edge(block, &rim(Vec3::new(55.0, 0.0, 10.0), 10.0));
    d.kind = kind(POINT_TO_POINT);
    d.inputs = inputs(POINT_TO_POINT);
    d.inputs[0].items = vec![Sel::Body { name: pin.into() }];
    d.inputs[1].items = vec![from];
    d.inputs[2].items = vec![to];
    let p = params(&app, &d).unwrap();
    assert_eq!(p["translate"], json!(["55 mm", "0 mm", "10 mm"]), "{p}");
    // The block's straight top edge along y = 15, picked at its middle.
    let straight = edge(block, &|p: Vec3| (p.z - 10.0).abs() < 1e-6 && (p.y - 15.0).abs() < 1e-6);
    let Sel::Edge { point, .. } = &straight else { panic!("not an edge") };
    // From the pin's centre at the origin.
    let want = *point;
    d.inputs[2].items = vec![straight.clone()];
    let p = params(&app, &d).unwrap();
    let got: Vec<f64> = p["translate"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().trim_end_matches(" mm").parse().unwrap()).collect();
    assert!((got[0] - want.x).abs() < 1e-5 && (got[1] - want.y).abs() < 1e-5 && (got[2] - want.z).abs() < 1e-5, "{p}");
}
