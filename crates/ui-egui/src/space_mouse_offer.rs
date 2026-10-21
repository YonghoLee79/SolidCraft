//! The offer to calibrate a 3D mouse that has no calibration yet: people don't know calibration
//! exists, and it is what fixes swapped or reversed axes. It shows once per device, as a notice
//! small card above the History bar, until the user calibrates or closes it.

use std::collections::BTreeSet;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit;

/// Whether to offer calibration for `device`: 3D mouse navigation is on, there is a device, it
/// has no calibration saved, and the offer has not been made (and dismissed) for it. The 3Dconnexion
/// driver (macOS, key `3dxware`) already applies its own settings, so calibration isn't offered
/// for it.
pub fn should_offer(device: Option<&str>, calibrated: bool, offered: &BTreeSet<String>, enabled: bool) -> bool {
    let Some(device) = device else { return false };
    enabled && !calibrated && !offered.contains(device) && !device.starts_with("3dxware")
}

/// How long the notice stays when nobody is pointing at it: long enough to read it, find the
/// mouse and decide, which 15 seconds was too short for.
const SHOW_SECONDS: f64 = 30.0;

/// Whether the notice, first shown or last pointed at `since`, has been up long enough at `now`.
pub fn expired(since: f64, now: f64) -> bool {
    now - since >= SHOW_SECONDS
}

/// Draws the offer as a small card in the bottom right of `free` (the part of the view the
/// overlays leave, so above the History bar), and takes the room it uses off `free`. It is due
/// when `should_offer` says so, the window has focus, no picker or load has the app, and neither
/// Preferences nor the calibration window is open. It goes after `SHOW_SECONDS` unless the pointer
/// is on it (then only for this run: it is offered again next time until closed or calibrated).
pub fn toast(app: &mut SolveApp, ui: &egui::Ui, free: &mut egui::Rect) {
    let ctx = ui.ctx();
    let calibrated = app.space_mouse.device.as_ref().is_some_and(|device| app.preferences.space_mouse_calibrations.contains_key(device));
    let Some(device) = app.space_mouse.device.clone().filter(|device| {
        should_offer(Some(device), calibrated, &app.preferences.space_mouse_offered, app.preferences.space_mouse_enabled)
            && !app.space_mouse.offer_hidden.contains(device)
    }) else {
        return;
    };
    if !ctx.input(|i| i.focused)
        || app.file_picker_busy()
        || app.file_load.busy()
        || app.prefs_window.open
        || app.space_mouse.calibrating.is_some()
        || !free.is_finite()
        || free.width() <= 120.0
    {
        return;
    }
    let (t, now) = (Tokens::get(), ctx.input(|i| i.time));
    let since = match &app.space_mouse.offer_since {
        Some((shown, since)) if *shown == device => *since,
        _ => now,
    };
    app.space_mouse.offer_since = Some((device.clone(), since));
    if expired(since, now) {
        app.space_mouse.offer_hidden.insert(device);
        return;
    }
    let (mut calibrate, mut close) = (false, false);
    let width = (free.width() - 2.0 * t.space_8).clamp(120.0, 380.0);
    let shown = egui::Area::new(egui::Id::new("sc_space_mouse_offer"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(free.right_bottom() - egui::vec2(t.space_8, t.space_8))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.dialog_body)
                .stroke(egui::Stroke::new(1.0, t.dialog_rule))
                .corner_radius(t.radius_lg)
                .inner_margin(t.space_12)
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * t.space_12);
                    ui_kit::text(ui, app.tr("ui.space_mouse_offer"), ui_kit::Text::Report("space_mouse_offer", false));
                    ui.horizontal(|ui| {
                        calibrate = ui_kit::button(ui, app.tr("ui.space_mouse_calibrate"), ui_kit::Button::Normal).clicked();
                        close = ui_kit::button(ui, app.tr("ui.close"), ui_kit::Button::Normal).clicked();
                    });
                });
        });
    let rect = shown.response.rect;
    // Pointing at it keeps it.
    if ctx.input(|i| i.pointer.hover_pos()).is_some_and(|pos| rect.contains(pos)) {
        app.space_mouse.offer_since = app.space_mouse.offer_since.take().map(|(device, _)| (device, now));
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    crate::scenario::publish_rect("toast:space_mouse_offer", rect);
    free.max.y = rect.top() - t.space_8;
    if (calibrate || close)
        && let Some((device, _)) = app.space_mouse.offer_since.take()
    {
        app.preferences.space_mouse_offered.insert(device);
    }
    if calibrate {
        // The calibration window lives with Preferences, at Navigation.
        crate::prefs::open_at(app, 1);
        crate::space_mouse_calibration::start(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offer_is_made_once_to_an_uncalibrated_device_with_navigation_on() {
        let none = BTreeSet::new();
        let seen: BTreeSet<String> = ["hid 046d:c62b".to_string()].into();
        for device in [Some("hid 046d:c62b"), Some("spacenavd"), Some("web 256f:c63a"), None] {
            for calibrated in [false, true] {
                for offered in [&none, &seen] {
                    for enabled in [false, true] {
                        let expected = device.is_some() && !calibrated && enabled && !device.is_some_and(|d| offered.contains(d));
                        assert_eq!(should_offer(device, calibrated, offered, enabled), expected, "{device:?} {calibrated} {offered:?} {enabled}");
                    }
                }
            }
        }
        // The driver on macOS has its own settings; there is nothing to calibrate.
        assert!(!should_offer(Some("3dxware"), false, &none, true));
    }

    fn press(h: &mut crate::scenario::Harness, widget: &str) {
        h.frames(3);
        h.step(&serde_json::json!({"widget": widget})).unwrap_or_else(|e| panic!("{widget}: {e}"));
        h.frames(2);
    }

    fn offered() -> crate::scenario::Harness {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.space_mouse.device = Some("hid 046d:c62b".into());
        h.frames(3);
        h
    }

    #[test]
    fn the_notice_goes_after_fifteen_seconds() {
        assert!(!expired(100.0, 129.9) && expired(100.0, 130.0));
    }

    #[test]
    fn closing_the_offer_records_the_device_so_it_does_not_come_back() {
        let mut h = offered();
        assert!(crate::scenario::rect_of("toast:space_mouse_offer").is_some(), "the notice is drawn");
        press(&mut h, "Close");
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"));
        assert!(h.app.space_mouse.calibrating.is_none());
        h.frames(3);
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"), "gone, and it stays gone");
    }

    #[test]
    fn calibrating_from_the_offer_opens_the_calibration_and_records_the_device() {
        let mut h = offered();
        press(&mut h, "Calibrate…");
        h.frames(3);
        assert!(h.app.space_mouse.calibrating.is_some() && h.app.prefs_window.open);
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"));
    }

    #[test]
    fn it_stays_away_from_a_calibrated_device_and_navigation_off() {
        let axes =
            std::array::from_fn(|source| solidcraft_space_mouse::AxisCalibration { source, reversed: false, rest: 0, dead_zone: 0, full: [350; 2] });
        let calibration = solidcraft_space_mouse::Calibration { dead_zone_share: 0.0, kind: solidcraft_space_mouse::Kind::ReturnToCenter, axes };
        for (enabled, calibrated) in [(false, false), (true, true)] {
            let mut h = crate::scenario::Harness::new();
            h.app.home.open = false;
            h.app.space_mouse.device = Some("hid 046d:c62b".into());
            h.app.preferences.space_mouse_enabled = enabled;
            if calibrated {
                h.app.preferences.space_mouse_calibrations.insert("hid 046d:c62b".into(), calibration);
            }
            h.frames(4);
            assert!(h.app.space_mouse.offer_since.is_none(), "no notice for enabled {enabled}, calibrated {calibrated}");
        }
    }
}
