//! Passive measurements follow selection without opening or changing the Measure tool.
use serde_json::json;
use solidcraft_engine::{Sel, geom::Vec3};
use solidcraft_ui_egui::scenario::{Harness, rect_of};

fn box_view() -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("engine.execute", json!({"command":"solid.box","params":{"length":40,"width":30,"height":20,"body_name":"Box"}}));
    h.call("ui.view", json!({"view":"fit"}));
    h
}

fn long_edge(h: &Harness) -> Sel {
    let world = h.app.session.world_state();
    let mesh = world.body("Box").unwrap().mesh();
    let (index, points) = mesh.edges.iter().enumerate().find(|(_, p)| (p.first().unwrap().dist(*p.last().unwrap()) - 40.0).abs() < 1e-8).unwrap();
    Sel::Edge { body: "Box".into(), index, point: points.first().unwrap().lerp(*points.last().unwrap(), 0.5) }
}

#[test]
fn selected_edge_shows_a_passive_measurement_without_opening_measure() {
    let mut h = box_view();
    h.app.session.selection = vec![long_edge(&h)];
    h.frames(3);
    assert!(h.widget("Length: ≈ 40.00 mm").is_some(), "show an explicitly approximate length as soon as an edge is selected");
    assert!(rect_of("viewport:quick_measure").is_some());
    assert!(h.app.dialog.is_none());
}

#[test]
fn pair_distance_follows_units_and_clears_with_the_selection() {
    let mut h = box_view();
    h.app.session.selection =
        vec![Sel::Vertex { body: "Box".into(), point: Vec3::ZERO }, Sel::Vertex { body: "Box".into(), point: Vec3::new(40.0, 30.0, 20.0) }];
    h.frames(3);
    // sqrt(40² + 30² + 20²), independent of any CAD reference value.
    assert!(h.widget("Minimum distance: ≈ 53.85 mm").is_some());
    h.call("engine.execute", json!({"command":"document.units","params":{"units":"in"}}));
    assert!(h.widget("Minimum distance: ≈ 2.1201 in").is_some());
    h.app.session.selection.clear();
    h.frames(2);
    assert!(rect_of("viewport:quick_measure").is_none());
}

#[test]
fn commands_and_unsupported_selections_hide_the_readout() {
    let mut h = box_view();
    h.app.session.selection = vec![long_edge(&h)];
    h.frames(2);
    assert!(rect_of("viewport:quick_measure").is_some());
    h.call("ui.start", json!({"command":"inspect.measure"}));
    assert!(rect_of("viewport:quick_measure").is_none());
    h.call("ui.key", json!({"key":"Escape"}));
    h.app.session.selection = vec![Sel::Body { name: "Box".into() }];
    h.frames(2);
    assert!(rect_of("viewport:quick_measure").is_none());
}

#[test]
fn overlay_fits_both_themes_and_avoids_navigation_and_frame_rate() {
    let mut h = box_view();
    h.app.session.selection = vec![long_edge(&h)];
    for theme in ["light", "dark"] {
        for (width, height, scale) in [(1600, 1000, 1.0), (900, 650, 1.5), (700, 450, 2.0)] {
            h.call("ui.prefs", json!({"theme":theme,"ui_scale":scale,"show_frame_rate":true}));
            h.call("ui.resize", json!({"width":width,"height":height}));
            h.frames(3);
            if let Some(r) = rect_of("viewport:quick_measure") {
                let view = h.app.viewport.rect.unwrap();
                assert!(view.contains_rect(r));
                for name in ["axis_view:outer", "nav:outer", "viewport:frame_rate"] {
                    if let Some(other) = rect_of(name) {
                        assert!(!r.intersect(other).is_positive(), "{theme}/{width}/{scale}: {name}");
                    }
                }
            } else {
                assert!(h.app.viewport.rect.unwrap().width() < 300.0, "hide only when the text cannot fit");
            }
        }
    }
}

#[test]
fn the_readout_does_not_take_clicks_from_the_viewport() {
    let mut h = box_view();
    h.app.session.selection = vec![long_edge(&h)];
    h.frames(3);
    let at = rect_of("viewport:quick_measure").unwrap().center();
    h.call("ui.click", json!({"x":at.x,"y":at.y}));
    assert!(h.app.session.selection.is_empty(), "clicking the readout on empty space still clears selection");
    assert!(rect_of("viewport:quick_measure").is_none());
}
