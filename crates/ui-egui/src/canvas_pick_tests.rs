//! Canvases in the 3D view: a see-through one is drawn over the model, a selectable one is
//! picked by a click, and what is under the click otherwise wins.

use serde_json::json;
use solidcraft_engine::Sel;
use solidcraft_engine::geom::Vec3;

use crate::scenario::Harness;
use crate::viewport::projection;

/// A small real PNG (so it decodes and draws), base64.
fn png() -> String {
    let mut bytes = Vec::new();
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 30, 30, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    solidcraft_engine::doc::canvas::base64_encode(&bytes)
}

fn canvas(h: &mut Harness, flags: serde_json::Value) -> u64 {
    let mut p = json!({"data": png(), "plane": "XY", "width": 100});
    p.as_object_mut().unwrap().extend(flags.as_object().unwrap().clone());
    h.app.run("canvas.insert", p).unwrap()["canvas"].as_u64().unwrap()
}

fn top(h: &mut Harness) {
    // The origin planes would take the click first.
    h.app.ui.show_origin = false;
    h.call("ui.view", json!({"view": "top"}));
    h.frames(3);
}

fn screen(h: &Harness, p: Vec3) -> egui::Pos2 {
    projection(&h.app, h.app.viewport.rect.unwrap()).to_screen(p).unwrap()
}

fn click(h: &mut Harness, at: egui::Pos2) {
    h.call("ui.click", json!({"x": at.x, "y": at.y}));
    h.frames(2);
}

#[test]
fn clicking_a_selectable_canvas_selects_it() {
    let mut h = Harness::new();
    let id = canvas(&mut h, json!({"selectable": true}));
    top(&mut h);
    let at = screen(&h, Vec3::new(5.0, 5.0, 0.0));
    click(&mut h, at);
    assert_eq!(h.app.tree.picked_canvases, vec![id]);
}

#[test]
fn a_canvas_that_is_not_selectable_lets_the_click_through() {
    let mut h = Harness::new();
    canvas(&mut h, json!({"selectable": false}));
    top(&mut h);
    let at = screen(&h, Vec3::new(5.0, 5.0, 0.0));
    click(&mut h, at);
    assert!(h.app.tree.picked_canvases.is_empty());
}

#[test]
fn other_hits_win_over_a_canvas() {
    let mut h = Harness::new();
    canvas(&mut h, json!({"selectable": true}));
    h.app.run("solid.box", json!({"corner": [-10, -10, 0], "length": 20, "width": 20, "height": 20})).unwrap();
    top(&mut h);
    let at = screen(&h, Vec3::new(0.0, 0.0, 20.0));
    click(&mut h, at);
    assert!(h.app.tree.picked_canvases.is_empty());
    assert!(matches!(h.app.session.selection.first(), Some(Sel::Face { .. })), "{:?}", h.app.session.selection);
    // The box gone and the origin plane shown: its square is picked, not the canvas under it.
    h.app.run("edit.undo", json!({})).unwrap();
    h.app.ui.show_origin = true;
    h.frames(2);
    let proj = projection(&h.app, h.app.viewport.rect.unwrap());
    let at = crate::viewport::origin_plane_target(&h.app, &proj, "XY").unwrap();
    let at = screen(&h, at);
    click(&mut h, at);
    assert!(h.app.tree.picked_canvases.is_empty());
    assert!(h.app.session.selection.iter().any(|s| matches!(s, Sel::Plane { name } if name == "XY")), "{:?}", h.app.session.selection);
}

#[test]
fn canvas_whose_plane_is_lost_or_size_is_hostile_is_skipped() {
    let mut h = Harness::new();
    canvas(&mut h, json!({"selectable": true}));
    canvas(&mut h, json!({"selectable": true}));
    top(&mut h);
    let proj = projection(&h.app, h.app.viewport.rect.unwrap());
    let centre = screen(&h, Vec3::new(5.0, 5.0, 0.0));
    assert!(crate::ref_images::canvas_at(&h.app, &proj, centre, |_| true).is_some());
    // The first one's plane no longer resolves, the second's stored width is not a number.
    let doc = std::sync::Arc::make_mut(&mut h.app.session.doc);
    doc.canvases[0].plane = solidcraft_engine::doc::PlaneRef::Origin { name: "nowhere".into() };
    doc.canvases[1].width = f64::NAN;
    assert!(crate::ref_images::canvas_at(&h.app, &proj, centre, |_| true).is_none());
    assert!(crate::ref_images::gpu_images(&h.app).is_empty());
    h.frames(3);
    click(&mut h, centre);
    assert!(h.app.tree.picked_canvases.is_empty());
}

#[test]
fn see_through_reaches_the_gpu_image() {
    let mut h = Harness::new();
    canvas(&mut h, json!({"see_through": true}));
    canvas(&mut h, json!({"see_through": false}));
    let images = crate::ref_images::gpu_images(&h.app);
    assert_eq!(images.iter().map(|i| i.on_top).collect::<Vec<_>>(), vec![true, false]);
}
