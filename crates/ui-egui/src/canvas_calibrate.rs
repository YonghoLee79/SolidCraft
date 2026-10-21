//! Calibrating a canvas (Browser ▸ canvas ▸ Calibrate): click point A on the image, then point
//! B, type their real distance into the box over the line A–B and press Enter. That runs
//! `canvas.calibrate`, which scales the image evenly about A (one undo step). Esc at any time,
//! or starting another tool or dialog, ends it with nothing changed. An open Edit Canvas card
//! closes keeping its edits.

use egui::{Pos2, Stroke};
use serde_json::json;
use solidcraft_engine::doc::expr::Kind as ValueKind;
use solidcraft_engine::geom::Vec2;

use crate::SolveApp;
use crate::i18n::tr_label;
use crate::theme::Tokens;
use crate::viewport::Proj;

/// A calibration in progress (`app.tree.calibration`): the points picked so far, in the
/// canvas's plane coordinates, and the distance being typed.
#[derive(Clone, Debug)]
pub struct Calibration {
    pub canvas: u64,
    pub a: Option<Vec2>,
    pub b: Option<Vec2>,
    pub distance: String,
}

const BOX_ID: &str = "sc_calibrate_value";

/// Start calibrating `canvas`; whatever tool or dialog is open is put away (one at a time).
/// The image must be showing: the clicks are picked on it.
pub fn start(app: &mut SolveApp, canvas: u64) {
    let Some(c) = app.session.doc.canvases.iter().find(|c| c.id == canvas) else {
        app.set_status(format!("Calibrate: there is no canvas {canvas}"), true);
        return;
    };
    if !c.visible || c.opacity <= 0.0 || app.session.folder_hidden(crate::browser::CANVASES) {
        app.set_status(tr_label("Calibrate: show the canvas first"), true);
        return;
    }
    // One at a time: the Edit Canvas card closes keeping its edits; a feature dialog is cancelled.
    app.tree.canvas_panel = None;
    if app.dialog.is_some() {
        crate::dialogs::cancel(app);
    }
    app.tool = None;
    app.tree.calibration = Some(Calibration { canvas, a: None, b: None, distance: String::new() });
    app.set_status(tr_label("Calibrate: click point A on the image"), false);
}

/// End it, changing nothing.
pub fn end(app: &mut SolveApp) {
    app.tree.calibration = None;
}

/// A click in the view while calibrating: it takes point A or B when it lands on the canvas's
/// image (bodies in front do not block it; nothing snaps). True when the calibration used it.
pub fn click(app: &mut SolveApp, proj: &Proj, pos: Pos2) -> bool {
    let Some(cal) = app.tree.calibration.as_ref() else { return false };
    if cal.b.is_some() {
        return true;
    }
    let canvas = cal.canvas;
    let Some((_, p)) = crate::ref_images::canvas_at(app, proj, pos, |c| c.id == canvas) else { return true };
    let distance = cal.a.map(|a| crate::prefs::show_mm(app, a.dist(p), 1));
    let Some(cal) = app.tree.calibration.as_mut() else { return true };
    let next = match distance {
        None => {
            cal.a = Some(p);
            "Calibrate: click point B on the image"
        }
        Some(d) => {
            cal.b = Some(p);
            cal.distance = d;
            "Calibrate: type the real distance from A to B and press Enter"
        }
    };
    app.set_status(tr_label(next), false);
    true
}

/// Run `canvas.calibrate` with the typed distance. A refusal says why in the status line and
/// keeps the box open.
fn apply(app: &mut SolveApp) {
    let Some(Calibration { canvas, a: Some(a), b: Some(b), distance }) = app.tree.calibration.clone() else { return };
    let d = match app.session.doc.eval(&distance, ValueKind::Length) {
        Ok(d) => d,
        Err(e) => return app.set_status(format!("Calibrate: {e}"), true),
    };
    // A non-finite distance becomes null in JSON, which the command refuses.
    if app.run("canvas.calibrate", json!({"canvas": canvas, "a": [a.x, a.y], "b": [b.x, b.y], "distance": d})).is_ok() {
        // The step-by-step hint is done with.
        app.status = None;
        end(app);
    }
}

/// Draw the points, the line A–B and its value box, which takes the keyboard: Enter applies,
/// Esc ends.
pub fn show(app: &mut SolveApp, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj) {
    let Some(mut cal) = app.tree.calibration.clone() else { return };
    let doc = &app.session.doc;
    let plane = doc.canvases.iter().find(|c| c.id == cal.canvas).and_then(|c| doc.canvas_plane(&doc.param_values().0, c).ok());
    // The canvas is gone, or another tool or dialog started.
    let (Some(plane), None, None, None) = (plane, &app.tool, &app.dialog, &app.tree.canvas_panel) else {
        return end(app);
    };
    // The box had the keyboard, so the Esc shortcut did not see the key.
    if !app.esc_handled && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.esc_handled = true;
        return end(app);
    }
    let t = Tokens::get();
    let at = |p: Vec2| proj.to_screen(plane.to_world(p));
    let (a, b) = (cal.a.and_then(at), cal.b.and_then(at));
    if let (Some(a), Some(b)) = (a, b) {
        painter.line_segment([a, b], Stroke::new(2.0, t.manipulator));
    }
    for p in [a, b].into_iter().flatten() {
        painter.circle(p, 4.0, t.manipulator, Stroke::new(1.0, t.text));
    }
    let Some(mid) = a.zip(b).map(|(a, b)| a.lerp(b, 0.5)) else { return };
    let id = egui::Id::new(BOX_ID);
    let mut enter = false;
    crate::frame::area(egui::Id::new("sc_calibrate_area")).fixed_pos(mid + egui::vec2(8.0, 8.0)).order(egui::Order::Foreground).show(
        ui.ctx(),
        |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr_label("Distance")).color(t.text_dim).size(t.font_small));
                    let r = ui.add(egui::TextEdit::singleline(&mut cal.distance).id(id).desired_width(80.0));
                    enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    // The keyboard comes here, all of the value selected, unless another field has it.
                    if !enter && ui.ctx().memory(|m| m.focused().is_none()) {
                        r.request_focus();
                        let n = cal.distance.chars().count();
                        let mut st = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                        st.cursor
                            .set_char_range(Some(egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                        st.store(ui.ctx(), id);
                    }
                });
            });
        },
    );
    app.tree.calibration = Some(cal);
    if enter {
        apply(app);
    }
}

#[cfg(test)]
mod tests {
    //! Calibrate from the Browser: two clicks on the image, the typed distance, one undo step;
    //! clicks off the image, Esc, refused values and other tools leave the design alone.
    use serde_json::json;
    use solidcraft_engine::geom::Vec3;

    use crate::scenario::Harness;
    use crate::viewport::projection;

    fn harness() -> Harness {
        let mut h = Harness::new();
        h.app.home.open = false;
        h.frames(2);
        h
    }

    fn click_at(h: &mut Harness, p: Vec3) {
        let at = projection(&h.app, h.app.viewport.rect.unwrap()).to_screen(p).unwrap();
        h.call("ui.click", json!({"x": at.x, "y": at.y}));
        h.frames(2);
    }

    fn canvas(h: &Harness, id: u64) -> solidcraft_engine::doc::canvas::Canvas {
        h.app.session.doc.canvases.iter().find(|c| c.id == id).cloned().unwrap()
    }

    /// A 4 x 2 picture 100 mm wide at the origin of XY, looked at from the top, with Calibrate
    /// picked from its Browser menu.
    fn calibrating(h: &mut Harness) -> u64 {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let data = solidcraft_engine::doc::canvas::base64_encode(&png);
        let id =
            h.app.run("canvas.insert", json!({"data": data, "plane": "XY", "center": [0, 0], "width": 100})).unwrap()["canvas"].as_u64().unwrap();
        h.call("ui.view", json!({"view": "top"}));
        h.frames(3);
        let items = crate::context_menu::items(&h.app, &crate::context_menu::Target::Canvas { id });
        let item = items.iter().find(|i| i.label == "Calibrate").unwrap().clone();
        crate::context_menu::run_item(&mut h.app, &item, egui::pos2(0.0, 0.0));
        h.frames(2);
        assert_eq!(h.app.tree.calibration.as_ref().map(|c| c.canvas), Some(id));
        id
    }

    fn calibration(h: &Harness) -> super::Calibration {
        h.app.tree.calibration.clone().expect("calibrating")
    }

    /// Type `text` into the value box (empty: clear it) and press Enter.
    fn type_enter(h: &mut Harness, text: &str) {
        h.frames(2);
        if text.is_empty() {
            h.step(&json!({"key": "Backspace"})).unwrap();
        } else {
            h.step(&json!({"text": text})).unwrap();
        }
        h.step(&json!({"key": "Enter"})).unwrap();
        h.frames(2);
    }

    #[test]
    fn calibrate_by_clicking_two_marks_and_typing_the_distance() {
        let mut h = harness();
        let id = calibrating(&mut h);
        let undo = h.app.session.undo.len();
        click_at(&mut h, Vec3::new(-25.0, 0.0, 0.0));
        let a = calibration(&h).a.expect("point A");
        assert!((a.x + 25.0).abs() < 1e-3 && a.y.abs() < 1e-3, "{a:?}");
        assert!(calibration(&h).b.is_none());
        click_at(&mut h, Vec3::new(25.0, 0.0, 0.0));
        assert!(calibration(&h).b.is_some_and(|b| (b.x - 25.0).abs() < 1e-3));
        h.frames(2);
        assert!(calibration(&h).distance.starts_with("50"), "the box holds the current distance: {}", calibration(&h).distance);
        type_enter(&mut h, "100");
        assert!(h.app.tree.calibration.is_none(), "{:?}", h.app.status);
        assert!(h.app.status.is_none(), "the hint is cleared: {:?}", h.app.status);
        let c = canvas(&h, id);
        assert!((c.width - 200.0).abs() < 1e-3, "{c:?}");
        // Point A stays where it is: it was a quarter of the width left of the centre, and still is.
        assert!((c.center.x - 25.0).abs() < 1e-3 && c.center.y.abs() < 1e-3, "{:?}", c.center);
        assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
        h.app.run("edit.undo", json!({})).unwrap();
        assert!((canvas(&h, id).width - 100.0).abs() < 1e-9);
    }

    #[test]
    fn calibrate_ignores_clicks_off_the_image_and_esc_changes_nothing() {
        let mut h = harness();
        let id = calibrating(&mut h);
        let (before, undo) = (h.app.session.doc.to_json(), h.app.session.undo.len());
        click_at(&mut h, Vec3::new(80.0, 0.0, 0.0));
        assert!(calibration(&h).a.is_none(), "off the image");
        click_at(&mut h, Vec3::new(-25.0, 0.0, 0.0));
        assert!(calibration(&h).a.is_some());
        h.step(&json!({"key": "Escape"})).unwrap();
        h.frames(2);
        assert!(h.app.tree.calibration.is_none());
        assert_eq!(h.app.session.doc.to_json(), before);
        assert_eq!(h.app.session.undo.len(), undo);
        // Esc with the value box open (it has the keyboard) ends it too.
        super::start(&mut h.app, id);
        click_at(&mut h, Vec3::new(-25.0, 0.0, 0.0));
        click_at(&mut h, Vec3::new(25.0, 0.0, 0.0));
        h.frames(2);
        h.step(&json!({"key": "Escape"})).unwrap();
        h.frames(2);
        assert!(h.app.tree.calibration.is_none());
        assert_eq!(h.app.session.doc.to_json(), before);
        // Starting another tool or dialog ends it.
        super::start(&mut h.app, id);
        h.app.start("solid.extrude");
        h.frames(2);
        assert!(h.app.tree.calibration.is_none());
    }

    #[test]
    fn calibrate_refuses_equal_points_and_bad_distances() {
        let mut h = harness();
        let id = calibrating(&mut h);
        let before = h.app.session.doc.to_json();
        click_at(&mut h, Vec3::new(-25.0, 0.0, 0.0));
        click_at(&mut h, Vec3::new(25.0, 0.0, 0.0));
        for text in ["0", "", "abc", "-5"] {
            h.app.status = None;
            type_enter(&mut h, text);
            assert!(h.app.tree.calibration.as_ref().is_some_and(|c| c.b.is_some()), "`{text}`: the box stays open");
            assert!(h.app.status.as_ref().is_some_and(|(_, _, error)| *error), "`{text}`: {:?}", h.app.status);
            assert_eq!(h.app.session.doc.to_json(), before, "`{text}`");
        }
        h.step(&json!({"key": "Escape"})).unwrap();
        h.frames(2);
        // A and B on the same spot.
        super::start(&mut h.app, id);
        click_at(&mut h, Vec3::new(10.0, 5.0, 0.0));
        click_at(&mut h, Vec3::new(10.0, 5.0, 0.0));
        h.app.status = None;
        type_enter(&mut h, "10");
        assert!(h.app.tree.calibration.is_some());
        assert!(h.app.status.as_ref().is_some_and(|(_, _, error)| *error), "{:?}", h.app.status);
        assert_eq!(h.app.session.doc.to_json(), before);
    }

    #[test]
    fn calibrate_closes_the_edit_canvas_card_keeping_its_edits() {
        let mut h = harness();
        let id = calibrating(&mut h);
        super::end(&mut h.app);
        h.app.tree.canvas_panel = Some(crate::browser::CanvasPanel::new(&h.app, id));
        h.app.run("canvas.edit", json!({"canvas": id, "opacity": 0.8})).unwrap();
        super::start(&mut h.app, id);
        assert!(h.app.tree.canvas_panel.is_none() && h.app.tree.calibration.is_some());
        assert!((canvas(&h, id).opacity - 0.8).abs() < 1e-9);
    }
}
