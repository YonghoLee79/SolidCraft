//! The offer to calibrate a 3D mouse that has no calibration yet: people don't know calibration
//! exists, and it is what fixes swapped or reversed axes. It shows once per device, as a notice
//! like `prefs_notice`, until the user calibrates or dismisses it.

use std::collections::BTreeSet;

use crate::SolveApp;
use crate::ui_kit;

/// Whether to offer calibration for `device`: 3D mouse navigation is on, there is a device, it
/// has no calibration saved, and the offer has not been made (and dismissed) for it. The 3Dconnexion
/// driver (macOS, key `3dxware`) already applies its own settings, so calibration isn't offered
/// for it.
pub fn should_offer(device: Option<&str>, calibrated: bool, offered: &BTreeSet<String>, enabled: bool) -> bool {
    let Some(device) = device else { return false };
    enabled && !calibrated && !offered.contains(device) && !device.starts_with("3dxware")
}

/// Draws the offer when it is due. Not while the window is in the background, a picker or load
/// has the app, or Preferences or the calibration window is open (the user is already there).
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let calibrated = app.space_mouse.device.as_ref().is_some_and(|device| app.preferences.space_mouse_calibrations.contains_key(device));
    if !should_offer(app.space_mouse.device.as_deref(), calibrated, &app.preferences.space_mouse_offered, app.preferences.space_mouse_enabled)
        || !ctx.input(|i| i.focused)
        || app.file_picker_busy()
        || app.file_load.busy()
        || app.prefs_window.open
        || app.space_mouse.calibrating.is_some()
        || app.home.open
    {
        return;
    }
    let Some(device) = app.space_mouse.device.clone() else { return };
    let (closed, action) = ui_kit::Window::new(egui::Id::new("sc_space_mouse_offer"), "settings", app.tr("ui.space_mouse_section"))
        .actions(app.tr("ui.space_mouse_calibrate"), app.tr("ui.close"), None)
        .show(ctx, true, |ui| {
            ui_kit::text(ui, app.tr("ui.space_mouse_offer"), ui_kit::Text::Report("space_mouse_offer", false));
        });
    if closed || action != ui_kit::Action::None {
        app.preferences.space_mouse_offered.insert(device);
    }
    if action == ui_kit::Action::Ok {
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
    fn closing_the_offer_records_the_device_so_it_does_not_come_back() {
        let mut h = offered();
        assert!(crate::scenario::handle_at("dialog:cancel").is_some(), "the notice is drawn");
        press(&mut h, "Close");
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"));
        assert!(h.app.space_mouse.calibrating.is_none());
        h.frames(3);
        assert!(crate::scenario::handle_at("dialog:cancel").is_none(), "gone, and it stays gone");
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
    fn it_stays_away_from_a_background_window_a_calibrated_device_and_navigation_off() {
        let mut h = offered();
        h.app.preferences.space_mouse_enabled = false;
        h.frames(3);
        assert!(crate::scenario::handle_at("dialog:cancel").is_none());
        let mut h = offered();
        let axes =
            std::array::from_fn(|source| solidcraft_space_mouse::AxisCalibration { source, reversed: false, rest: 0, dead_zone: 0, full: [350; 2] });
        let calibration = solidcraft_space_mouse::Calibration { dead_zone_share: 0.0, kind: solidcraft_space_mouse::Kind::ReturnToCenter, axes };
        h.app.preferences.space_mouse_calibrations.insert("hid 046d:c62b".into(), calibration);
        h.frames(3);
        assert!(crate::scenario::handle_at("dialog:cancel").is_none());
    }
}
