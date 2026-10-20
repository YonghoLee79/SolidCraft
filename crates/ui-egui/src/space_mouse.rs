//! 3D mouse input, shared by the desktop and web apps. Each app reads the device its own way (the
//! desktop through HID, the web app through the browser's Gamepad API) and hands its six raw axes
//! to [`navigate_frame`] once per frame. This module maps them, applies the dead zone and drops
//! 3DxWare's emulated wheel; the page showing the camera moves it (`navigation::device`), so the
//! cap does what that page's camera may do.

use std::time::Duration;

use crate::SolveApp;

pub use solidcraft_space_mouse::{AxisCalibration, Calibration};

/// What a 3Dconnexion device reports at full deflection, in raw units, on every axis.
pub const FULL_SCALE: i16 = 350;

/// One reading of the six axes in the device's raw units: X right, Y towards the user, Z down,
/// and the rotations about those axes (right-handed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axes {
    pub translation: [i16; 3],
    pub rotation: [i16; 3],
}

impl Axes {
    /// Some axis is past the dead zone: the cap is being moved.
    pub fn moving(self) -> bool {
        self.translation.iter().chain(&self.rotation).any(|&v| normalized(v) != 0.0)
    }
}

impl From<Axes> for [i16; 6] {
    fn from(axes: Axes) -> Self {
        let ([x, y, z], [rx, ry, rz]) = (axes.translation, axes.rotation);
        [x, y, z, rx, ry, rz]
    }
}

impl From<[i16; 6]> for Axes {
    fn from([x, y, z, rx, ry, rz]: [i16; 6]) -> Self {
        Self { translation: [x, y, z], rotation: [rx, ry, rz] }
    }
}

/// The 3D mouse as of this frame.
#[derive(Debug, Default)]
pub struct Input {
    /// The device being read: where from and its vendor and product ids (`hid 256f:c63a`,
    /// `web 256f:c63a`), or where from alone (`spacenavd`). Its calibration is saved under it.
    pub device: Option<String>,
    /// This frame's reading, before calibration.
    pub raw: Axes,
    /// The guided calibration while its window is open, and the device it is for.
    pub calibrating: Option<(String, solidcraft_space_mouse::Wizard)>,
    /// Drops readings that have stopped changing close to the dead zone.
    pub settle: solidcraft_space_mouse::Settle,
    /// The buttons down as of the last frame, one bit each (bit N for button N + 1).
    pub buttons: u32,
    /// Every button pressed since the app started, so Preferences can list the ones the device has.
    pub seen: u32,
    /// The modifiers the buttons hold down, as of the last frame.
    pub held: egui::Modifiers,
    /// The modifiers the keyboard holds, as the window last reported them (without `held`).
    pub keyboard: egui::Modifiers,
    /// The device `seen` and `settle` are about.
    pub read_device: Option<String>,
}

/// A key a 3D mouse button can stand in for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonKey {
    Esc,
    Shift,
    Ctrl,
    Alt,
}

/// What a 3D mouse button does.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonJob {
    /// Opens Preferences at Navigation, where the 3D mouse's settings are.
    Settings,
    /// A view as `SolveApp::animate_view` takes it: `fit`, `home` or a standard view (`top`, …).
    View(String),
    /// Stands in for a key: Esc when pressed, Shift, Ctrl and Alt for as long as it's held.
    Key(ButtonKey),
    /// Runs a command, as its keyboard shortcut would.
    Command(String),
}

/// What each of `device`'s buttons does, by number from 1: what the user set, else for a device
/// we have measured, its printed keys' jobs.
pub fn button_jobs(app: &SolveApp, device: &str) -> std::collections::BTreeMap<u8, ButtonJob> {
    app.preferences.space_mouse_buttons.get(device).cloned().unwrap_or_else(|| default_button_jobs(device))
}

/// The printed keys' jobs on a device we have measured (`solidcraft_space_mouse::buttons`), from
/// its key (`hid 046d:c62b`, `web 046d:c62b`); nothing for any other device, spacenavd included,
/// which numbers buttons its own way. Keys with no job here (Roll, the rotation lock) start empty.
pub fn default_button_jobs(device: &str) -> std::collections::BTreeMap<u8, ButtonJob> {
    use solidcraft_space_mouse::buttons::{Action, defaults};
    let ids = device.split_once(' ').and_then(|(_, ids)| ids.split_once(':'));
    let table = ids.and_then(|(vendor, product)| Some((u16::from_str_radix(vendor, 16).ok()?, u16::from_str_radix(product, 16).ok()?)));
    let Some(table) = table.and_then(|(vendor, product)| defaults(vendor, product)) else { return Default::default() };
    table
        .iter()
        .enumerate()
        .filter_map(|(index, action)| {
            let job = match (*action)? {
                Action::Menu => ButtonJob::Settings,
                Action::Fit => ButtonJob::View("fit".into()),
                Action::ViewTop => ButtonJob::View("top".into()),
                Action::ViewRight => ButtonJob::View("right".into()),
                Action::ViewFront => ButtonJob::View("front".into()),
                Action::Esc => ButtonJob::Key(ButtonKey::Esc),
                Action::Shift => ButtonJob::Key(ButtonKey::Shift),
                Action::Ctrl => ButtonJob::Key(ButtonKey::Ctrl),
                Action::Alt => ButtonJob::Key(ButtonKey::Alt),
                Action::Roll | Action::RotationLock => return None,
            };
            Some((u8::try_from(index + 1).ok()?, job))
        })
        .collect()
}

/// Whether a button may do anything now: only where navigation and the keyboard shortcuts would
/// work (`navigate_frame`, `toolbar::shortcuts`), so a press in the background, with a picker open
/// or while a field takes the keyboard doesn't move the view or run a command.
fn buttons_active(app: &SolveApp, ctx: &egui::Context, focused: bool) -> bool {
    app.preferences.space_mouse_enabled
        && focused
        && !app.prefs_window.open
        && app.space_mouse.calibrating.is_none()
        && !app.home.open
        && !app.file_picker_busy()
        && !app.file_load.busy()
        && !crate::keymap::capturing(app)
        && !egui::Popup::is_any_open(ctx)
        && !ctx.egui_wants_keyboard_input()
        && !crate::dim_view::editing()
        && !crate::dialogs_save_mesh::popup_open(ctx)
        && crate::toolbar::design_shown(app)
}

/// The 3D mouse's buttons this frame (`buttons`, bit N for button N + 1), before the app sees the
/// frame's input: each button just pressed does its job, and those standing in for Shift, Ctrl or
/// Alt hold them while down. Nothing runs where navigation wouldn't (`buttons_active`), so pressing
/// a button in Preferences finds it in the list instead, and a press made then isn't run later.
pub fn buttons_hook(app: &mut SolveApp, ctx: &egui::Context, buttons: u32, raw: &mut egui::RawInput) {
    let pressed = buttons & !app.space_mouse.buttons;
    app.space_mouse.buttons = buttons;
    app.space_mouse.seen |= buttons;
    // The keyboard's own modifiers, from what the window was told before anything is added to it.
    // egui drops its modifiers when the window loses focus, so those go too.
    let mut focused = ctx.input(|i| i.focused);
    for event in &raw.events {
        match event {
            egui::Event::ModifiersChanged(modifiers) => app.space_mouse.keyboard = *modifiers,
            egui::Event::WindowFocused(now) => {
                focused = *now;
                if !now {
                    app.space_mouse.keyboard = egui::Modifiers::NONE;
                }
            }
            _ => {}
        }
    }
    let keyboard = app.space_mouse.keyboard;
    let mut held = egui::Modifiers::NONE;
    let device = app.space_mouse.device.clone().filter(|_| buttons_active(app, ctx, focused));
    let jobs = device.map(|device| button_jobs(app, &device)).unwrap_or_default();
    let before = std::mem::replace(&mut app.space_mouse.held, egui::Modifiers::NONE);
    for (number, job) in jobs {
        let Some(bit) = number.checked_sub(1).and_then(|index| 1_u32.checked_shl(u32::from(index))) else { continue };
        if buttons & bit == 0 {
            continue;
        }
        match job {
            ButtonJob::Key(ButtonKey::Shift) => held.shift = true,
            ButtonJob::Key(ButtonKey::Alt) => held.alt = true,
            ButtonJob::Key(ButtonKey::Ctrl) => {
                held.ctrl = true;
                // Ctrl is the command key everywhere but on a Mac.
                held.command |= !cfg!(target_os = "macos");
            }
            _ => {}
        }
        if pressed & bit != 0 {
            run_button_job(app, &job, keyboard, raw);
        }
    }
    app.space_mouse.held = held;
    let add = |modifiers: &mut egui::Modifiers| {
        modifiers.shift |= held.shift;
        modifiers.alt |= held.alt;
        modifiers.ctrl |= held.ctrl;
        modifiers.command |= held.command;
    };
    // egui keeps modifiers from frame to frame and takes new ones from `ModifiersChanged`: the
    // keyboard's get the held ones added, and a change in what's held is one of its own.
    let mut changed = false;
    for event in &mut raw.events {
        match event {
            egui::Event::ModifiersChanged(modifiers) => {
                add(modifiers);
                changed = true;
            }
            egui::Event::PointerButton { modifiers, .. } | egui::Event::Key { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. } => {
                add(modifiers);
            }
            _ => {}
        }
    }
    if !changed && held != before {
        let mut modifiers = keyboard;
        add(&mut modifiers);
        raw.events.insert(0, egui::Event::ModifiersChanged(modifiers));
    }
}

fn run_button_job(app: &mut SolveApp, job: &ButtonJob, modifiers: egui::Modifiers, raw: &mut egui::RawInput) {
    match job {
        // Navigation, where the 3D mouse's section is.
        ButtonJob::Settings => crate::prefs::open_at(app, 1),
        ButtonJob::View(view) => app.animate_view(view),
        ButtonJob::Key(ButtonKey::Esc) => {
            for pressed in [true, false] {
                raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers });
            }
        }
        ButtonJob::Key(_) => {}
        ButtonJob::Command(id) => {
            if solidcraft_engine::find_command(id).is_some_and(|c| c.info(&app.session).enabled) {
                app.start(id);
            }
        }
    }
}

/// Seconds after the cap last moved that wheel events are still taken for 3DxWare's: one can be
/// queued just before the cap stops and only reach the next frame.
pub const WHEEL_QUIET_S: f64 = 0.15;

/// 3DxWare turns the cap's motion into mouse-wheel turns for programs it has no profile for (its
/// keyboard and mouse emulation), browsers included (#1336, #1380). Both apps read the cap
/// themselves, so those are dropped: wheel events while the cap is moving and for
/// [`WHEEL_QUIET_S`] after. 3DxWare sends them only well past the dead zone (from about a third
/// of full deflection), never at rest, so a wheel turn with the cap at rest is the user's.
#[derive(Debug, Default)]
pub struct WheelGuard {
    moved_at: Option<f64>,
}

impl WheelGuard {
    /// Takes this frame's reading (`now` in seconds) and drops the frame's wheel events if they
    /// are 3DxWare's.
    pub fn filter(&mut self, moving: bool, now: f64, events: &mut Vec<egui::Event>) {
        if moving {
            self.moved_at = Some(now);
        }
        let recent = self.moved_at.is_some_and(|at| (0.0..=WHEEL_QUIET_S).contains(&(now - at)));
        if recent {
            events.retain(|e| !matches!(e, egui::Event::MouseWheel { .. } | egui::Event::Zoom(_)));
        }
    }
}

/// Where one physical axis goes: its logical axis, and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRule {
    pub output: usize,
    pub enabled: bool,
    pub reversed: bool,
    pub scale: f64,
}

/// The physical-to-logical axis mapping 3DxWare applies (the desktop reads it from the driver's
/// profile; direct reads bypass the driver).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriverMapping {
    pub axes: [AxisRule; 6],
    pub overall_scale: f64,
}

impl Default for DriverMapping {
    fn default() -> Self {
        Self { axes: std::array::from_fn(|output| AxisRule { output, enabled: true, reversed: false, scale: 1.0 }), overall_scale: 1.0 }
    }
}

impl DriverMapping {
    /// What 3DxWare applies by default in object mode (#1172): the physical Ry and Rz swapped and
    /// reversed, so a tilt left/right rolls and a twist spins.
    pub fn object_mode() -> Self {
        let mut mapping = Self::default();
        mapping.axes[4] = AxisRule { output: 5, enabled: true, reversed: true, scale: 1.0 };
        mapping.axes[5] = AxisRule { output: 4, enabled: true, reversed: true, scale: 1.0 };
        mapping
    }

    /// The mapping with the user's per-axis choices folded in: `reverse` flips and `speed` scales
    /// each physical axis of the cap. A speed that is not a finite number counts as 1.
    pub fn with_user_axes(mut self, reverse: [bool; 6], speed: [f32; 6]) -> Self {
        for ((rule, reverse), speed) in self.axes.iter_mut().zip(reverse).zip(speed) {
            rule.reversed ^= reverse;
            rule.scale *= if speed.is_finite() { f64::from(speed).clamp(0.0, 3.0) } else { 1.0 };
        }
        self
    }

    pub fn apply(self, axes: Axes) -> Motion {
        let raw = [
            normalized(axes.translation[0]),
            normalized(axes.translation[1]),
            normalized(axes.translation[2]),
            normalized(axes.rotation[0]),
            normalized(axes.rotation[1]),
            normalized(axes.rotation[2]),
        ];
        let mut logical = [0.0; 6];
        for (input, rule) in self.axes.iter().enumerate() {
            if rule.enabled {
                let sign = if rule.reversed { -1.0 } else { 1.0 };
                if let Some(slot) = logical.get_mut(rule.output) {
                    *slot = raw[input] * sign * rule.scale * self.overall_scale;
                }
            }
        }
        Motion { translation: [logical[0], logical[1], logical[2]], rotation: [logical[3], logical[4], logical[5]] }
    }
}

/// The logical axes after the mapping, each in -1..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub translation: [f64; 3],
    pub rotation: [f64; 3],
}

/// One frame of 3D mouse motion, waiting for the page that shows the camera: the logical axes,
/// and how many seconds of motion they stand for (the frame's time, scaled by the speed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeviceMotion {
    pub motion: Motion,
    pub seconds: f64,
}

/// The saved calibration of the device being read, when it has one.
pub fn calibration(app: &SolveApp) -> Option<Calibration> {
    app.space_mouse.device.as_ref().and_then(|device| app.preferences.space_mouse_calibrations.get(device)).copied()
}

/// A reading on 3Dconnexion's axes: through `calibration` when there is one. Whether the cap is
/// moving is asked of this, never of the raw reading, so a device resting off zero isn't "moving"
/// and a short-range one moves as soon as it does.
pub fn calibrated(axes: Axes, calibration: Option<Calibration>) -> Axes {
    calibration.map_or(axes, |calibration| calibration.apply(axes.into()).into())
}

/// Hands one frame of 3D mouse input, `seconds` after the previous frame, to the page showing
/// the camera. Nothing happens with navigation switched off, on the start page or while a file
/// picker is open, or while the reading is inside the dead zone (after calibration, which takes
/// off the device's own rest offset), so a cap resting just off zero doesn't keep redrawing.
/// Returns whether there was motion to hand over; it then asks for the next frame itself. The
/// reading goes through the device's saved calibration first, when it has one; while the
/// calibration window is open, the reading goes to it instead and the view stays put.
pub fn navigate_frame(app: &mut SolveApp, ctx: &egui::Context, mapping: DriverMapping, axes: Axes, seconds: f64) -> bool {
    app.space_mouse.raw = axes;
    if app.space_mouse.read_device != app.space_mouse.device {
        // Another device: its buttons and its resting hand are not the last one's.
        app.space_mouse.read_device = app.space_mouse.device.clone();
        app.space_mouse.seen = 0;
        app.space_mouse.settle = Default::default();
    }
    if let Some((device, wizard)) = &mut app.space_mouse.calibrating {
        if app.space_mouse.device.as_ref() == Some(device) {
            wizard.feed(axes.into(), seconds);
            ctx.request_repaint_after(Duration::from_millis(16));
            return false;
        }
        // Another device is being read now, or none: what was recorded isn't its.
        app.space_mouse.calibrating = None;
    }
    let axes = calibrated(axes, calibration(app));
    // A hand leaning on the cap, holding it just past the dead zone, doesn't creep the view.
    let axes: Axes = app.space_mouse.settle.filter(axes.into(), seconds).into();
    // Only the window with focus: with two windows open, the cap moves the one being used, not both.
    if !app.preferences.space_mouse_enabled || app.home.open || app.file_picker_busy() || !axes.moving() || !ctx.input(|i| i.focused) {
        return false;
    }
    let speed = f64::from(app.preferences.space_mouse_speed);
    if !speed.is_finite() {
        return false;
    }
    app.begin_external_navigation();
    // A long gap (the first frame after a pause) moves as far as a 50 ms one.
    let step = if seconds.is_finite() { seconds.min(0.05) } else { 0.0 };
    app.viewport.device_motion = Some(DeviceMotion {
        motion: mapping.with_user_axes(app.preferences.space_mouse_reverse, app.preferences.space_mouse_axis_speed).apply(axes),
        seconds: step * speed.clamp(0.1, 5.0),
    });
    ctx.request_repaint_after(Duration::from_millis(16));
    true
}

/// A small dead zone avoids drift at rest. A full cap deflection takes roughly one viewport
/// per second to pan and about two radians per second to turn, independent of frame rate.
fn normalized(value: i16) -> f64 {
    let v = f64::from(value);
    if v.abs() <= 20.0 { 0.0 } else { v.signum() * ((v.abs() - 20.0) / 330.0).min(1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use solidcraft_engine::Session;
    use solidcraft_engine::render::Camera;

    /// The camera math the page applies the motion with (`navigation::device`): `sketch` keeps
    /// the orientation, as editing a sketch does.
    fn navigate(cam: &mut Camera, motion: Motion, seconds: f64, height: f64, sketch: bool, reverse_zoom: bool) {
        crate::navigation::device_camera(cam, motion, seconds, height, !sketch, reverse_zoom);
    }

    /// A window that has run a frame, with or without focus. A context that has never run one
    /// reads as unfocused, so the tests run one first, as the app always has.
    fn window(focused: bool) -> egui::Context {
        let ctx = crate::fonts::test_context();
        let mut output = ctx.run_ui(egui::RawInput { focused, ..Default::default() }, |_| {});
        output.textures_delta.clear();
        ctx
    }

    /// One frame: the 3D mouse hands its motion over, and the modelling viewport applies it.
    fn frame(app: &mut SolveApp, ctx: &egui::Context, mapping: DriverMapping, axes: Axes, seconds: f64) -> bool {
        let handed = navigate_frame(app, ctx, mapping, axes, seconds);
        let page = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        crate::navigation::device(app, page, crate::navigation::NavCaps::MODEL);
        handed
    }

    fn raw(axes: Axes) -> Motion {
        DriverMapping::default().apply(axes)
    }

    fn app() -> SolveApp {
        let mut app = SolveApp::new(Session::default(), Services::default());
        app.home.open = false;
        app
    }

    #[test]
    fn a_measured_device_starts_with_its_printed_keys() {
        let jobs = default_button_jobs("hid 046d:c62b");
        assert_eq!(jobs.get(&1), Some(&ButtonJob::Settings));
        assert_eq!(jobs.get(&3), Some(&ButtonJob::View("top".into())));
        assert_eq!(jobs.get(&25), Some(&ButtonJob::Key(ButtonKey::Shift)));
        // Roll and the rotation lock have nothing to do here yet; the numbered keys start empty.
        assert_eq!((jobs.get(&9), jobs.get(&27), jobs.get(&13)), (None, None, None));
        // Unmeasured, spacenavd (its own numbers) and nonsense keys start empty.
        for device in ["hid 256f:c63a", "spacenavd", "web 046d", "hid zz:c62b", ""] {
            assert!(default_button_jobs(device).is_empty(), "{device}");
        }
    }

    /// One frame of the window: the hook sees the frame's input, then egui runs it, as the app does.
    fn press(app: &mut SolveApp, ctx: &egui::Context, buttons: u32, events: Vec<egui::Event>) -> egui::RawInput {
        let mut raw = egui::RawInput { focused: true, events, ..Default::default() };
        buttons_hook(app, ctx, buttons, &mut raw);
        ctx.run_ui(raw.clone(), |_| {}).textures_delta.clear();
        raw
    }

    fn escapes(raw: &egui::RawInput) -> usize {
        raw.events.iter().filter(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })).count()
    }

    const ESC: u32 = 1 << 22;
    const SHIFT: u32 = 1 << 24;

    #[test]
    fn a_button_does_its_job_once_per_press_and_holds_its_modifier() {
        let mut app = app();
        app.space_mouse.device = Some("hid 046d:c62b".into());
        let ctx = window(true);
        let shifted = || ctx.input(|i| i.modifiers.shift);
        // Esc (button 23) presses Escape once, not again while it's held.
        assert_eq!(escapes(&press(&mut app, &ctx, ESC, vec![])), 1);
        assert_eq!(escapes(&press(&mut app, &ctx, ESC, vec![])), 0);
        // Shift (button 25) holds Shift for as long as it's down.
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(shifted());
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(shifted());
        press(&mut app, &ctx, 0, vec![]);
        assert!(!shifted());
        // Menu (button 1) opens Preferences; there, buttons only light up their rows.
        press(&mut app, &ctx, 1, vec![]);
        assert!(app.prefs_window.open);
        press(&mut app, &ctx, 0, vec![]);
        assert_eq!(escapes(&press(&mut app, &ctx, ESC, vec![])), 0);
        assert_eq!(app.space_mouse.seen & ESC, ESC);
        // A job the user set replaces the defaults.
        app.prefs_window.open = false;
        press(&mut app, &ctx, 0, vec![]);
        app.preferences.space_mouse_buttons.insert("hid 046d:c62b".into(), [(23, ButtonJob::Key(ButtonKey::Alt))].into());
        let raw = press(&mut app, &ctx, ESC, vec![]);
        assert_eq!((escapes(&raw), ctx.input(|i| i.modifiers.alt)), (0, true));
    }

    /// The keyboard and a button hold the same modifier: it stays down until both let go, whichever
    /// lets go first, for each of Shift, Ctrl and Alt.
    #[test]
    fn a_modifier_the_keyboard_and_a_button_share_stays_down_until_both_let_go() {
        for key in [ButtonKey::Shift, ButtonKey::Ctrl, ButtonKey::Alt] {
            let down = |ctx: &egui::Context| {
                let m = ctx.input(|i| i.modifiers);
                match key {
                    ButtonKey::Shift => m.shift,
                    ButtonKey::Ctrl => m.ctrl,
                    _ => m.alt,
                }
            };
            let keyboard = |on: bool| {
                let mut m = egui::Modifiers::NONE;
                match key {
                    ButtonKey::Shift => m.shift = on,
                    ButtonKey::Ctrl => m.ctrl = on,
                    _ => m.alt = on,
                }
                vec![egui::Event::ModifiersChanged(m)]
            };
            for button_first in [true, false] {
                let mut app = app();
                app.space_mouse.device = Some("hid 046d:c62b".into());
                app.preferences.space_mouse_buttons.insert("hid 046d:c62b".into(), [(1, ButtonJob::Key(key))].into());
                let ctx = window(true);
                press(&mut app, &ctx, 0, keyboard(true));
                press(&mut app, &ctx, 1, vec![]);
                assert!(down(&ctx), "{key:?} both");
                if button_first {
                    press(&mut app, &ctx, 0, vec![]);
                    assert!(down(&ctx), "{key:?} the keyboard still holds it");
                    press(&mut app, &ctx, 0, keyboard(false));
                } else {
                    press(&mut app, &ctx, 1, keyboard(false));
                    assert!(down(&ctx), "{key:?} the button still holds it");
                    press(&mut app, &ctx, 0, vec![]);
                }
                assert!(!down(&ctx), "{key:?} neither");
            }
        }
    }

    #[test]
    fn buttons_do_nothing_where_navigation_would_not() {
        let mut app = app();
        app.space_mouse.device = Some("hid 046d:c62b".into());
        app.preferences
            .space_mouse_buttons
            .insert("hid 046d:c62b".into(), [(1, ButtonJob::Key(ButtonKey::Esc)), (2, ButtonJob::Key(ButtonKey::Shift))].into());
        let (focused, unfocused) = (window(true), window(false));
        // A press while the window is in the background does nothing, and isn't run on refocus.
        assert_eq!(escapes(&press(&mut app, &unfocused, 1, vec![])), 0);
        assert_eq!(escapes(&press(&mut app, &focused, 1, vec![])), 0, "still held: not a new press");
        press(&mut app, &focused, 0, vec![]);
        assert_eq!(escapes(&press(&mut app, &focused, 1, vec![])), 1, "a press that is its own");
        // A held modifier button lets go in the background, and takes hold again on refocus.
        press(&mut app, &focused, 2, vec![]);
        assert!(focused.input(|i| i.modifiers.shift));
        press(&mut app, &unfocused, 2, vec![]);
        assert!(!unfocused.input(|i| i.modifiers.shift));
        // A file picker being open is no place for one either.
        press(&mut app, &focused, 0, vec![]);
        let slot: crate::picker_test_support::Slot = Default::default();
        crate::picker_test_support::open(&mut app, &slot);
        app.pick_open(|_, _| {});
        assert_eq!(escapes(&press(&mut app, &focused, 1, vec![])), 0);
        app.file_picker = None;
        press(&mut app, &focused, 0, vec![]);
        assert_eq!(escapes(&press(&mut app, &focused, 1, vec![])), 1);
    }

    #[test]
    fn preferences_and_calibration_let_go_of_a_held_modifier_button() {
        let mut app = app();
        app.space_mouse.device = Some("hid 046d:c62b".into());
        let ctx = window(true);
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(ctx.input(|i| i.modifiers.shift));
        app.prefs_window.open = true;
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(!ctx.input(|i| i.modifiers.shift), "Preferences open");
        app.prefs_window.open = false;
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(ctx.input(|i| i.modifiers.shift));
        crate::space_mouse_calibration::start(&mut app);
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(!ctx.input(|i| i.modifiers.shift), "calibrating");
    }

    #[test]
    fn losing_focus_clears_what_the_buttons_hold_and_the_keyboard_is_remembered_again_on_regain() {
        let mut app = app();
        app.space_mouse.device = Some("hid 046d:c62b".into());
        let ctx = window(true);
        press(&mut app, &ctx, SHIFT, vec![]);
        assert!(app.space_mouse.held.shift);
        press(&mut app, &ctx, SHIFT, vec![egui::Event::WindowFocused(false)]);
        assert_eq!(app.space_mouse.held, egui::Modifiers::NONE);
        assert!(!ctx.input(|i| i.modifiers.shift));
        // Back, the button still down: it holds Shift again, and a keyboard Shift lost with the
        // focus stays lost until the window says it is down.
        press(&mut app, &ctx, SHIFT, vec![egui::Event::WindowFocused(true)]);
        assert!(ctx.input(|i| i.modifiers.shift));
        assert_eq!(escapes(&press(&mut app, &ctx, SHIFT, vec![])), 0);
    }

    #[test]
    fn wheel_events_carry_the_held_modifiers() {
        let mut app = app();
        app.space_mouse.device = Some("hid 046d:c62b".into());
        let ctx = window(true);
        let raw = press(&mut app, &ctx, SHIFT, vec![wheel(1.0)]);
        assert!(raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { modifiers, .. } if modifiers.shift)));
    }

    #[test]
    fn another_device_starts_with_no_buttons_seen_and_no_settling() {
        let mut app = app();
        let ctx = window(true);
        app.space_mouse.device = Some("hid 046d:c62b".into());
        frame(&mut app, &ctx, DriverMapping::default(), Axes::default(), 0.016);
        app.space_mouse.seen = 0b101;
        frame(&mut app, &ctx, DriverMapping::default(), Axes::default(), 0.016);
        assert_eq!(app.space_mouse.seen, 0b101);
        app.space_mouse.device = Some("hid 256f:c63a".into());
        frame(&mut app, &ctx, DriverMapping::default(), Axes::default(), 0.016);
        assert_eq!(app.space_mouse.seen, 0);
    }

    #[test]
    fn per_axis_choices_flip_and_scale_one_physical_axis() {
        let mapping = DriverMapping::default().with_user_axes([false, true, false, false, false, false], [1.0, 1.0, 1.0, 0.5, f32::NAN, 1.0]);
        let still = Motion::default();
        assert_eq!(mapping.apply(Axes { translation: [0, 350, 0], rotation: [0; 3] }).translation[1], -raw(PUSH).translation[1]);
        assert_eq!(
            mapping.apply(Axes { translation: [350, 0, 0], rotation: [0; 3] }).translation[0],
            raw(Axes { translation: [350, 0, 0], rotation: [0; 3] }).translation[0]
        );
        assert_eq!(mapping.apply(Axes { translation: [0; 3], rotation: [350, 0, 0] }).rotation[0], 0.5);
        assert_eq!(mapping.apply(Axes { translation: [0; 3], rotation: [0, 350, 0] }).rotation[1], 1.0);
        assert_eq!(mapping.apply(Axes::default()), still);
        let off = DriverMapping::default().with_user_axes([false; 6], [0.0; 6]);
        assert_eq!(off.apply(PUSH), still);
    }

    const PUSH: Axes = Axes { translation: [0, 350, 0], rotation: [0; 3] };

    fn wheel(dy: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        }
    }

    fn other() -> egui::Event {
        egui::Event::PointerMoved(egui::pos2(1.0, 2.0))
    }

    #[test]
    fn six_axes_move_the_camera_and_sketches_keep_their_orientation() {
        let mut cam = Camera::default();
        let before = cam;
        navigate(&mut cam, raw(Axes { translation: [350, 350, 350], rotation: [350, 350, 350] }), 0.02, 600.0, false, false);
        assert_ne!(cam.target, before.target);
        assert!(cam.distance < before.distance);
        assert_ne!(cam.yaw, before.yaw);
        assert_ne!(cam.pitch, before.pitch);
        assert_ne!(cam.roll, before.roll);
        let orientation = (cam.yaw, cam.pitch, cam.roll);
        navigate(&mut cam, raw(Axes { translation: [350, 0, 0], rotation: [350, 350, 350] }), 0.02, 600.0, true, false);
        assert_eq!((cam.yaw, cam.pitch, cam.roll), orientation);
        assert!(cam.is_valid());
    }

    #[test]
    fn individual_axes_have_physical_spacemouse_meaning() {
        let before = Camera::default();

        let mut slide = before;
        navigate(&mut slide, raw(Axes { translation: [350, 0, 0], rotation: [0; 3] }), 0.02, 600.0, false, false);
        assert_ne!(slide.target, before.target);
        assert_eq!(slide.distance, before.distance);

        let mut push = before;
        navigate(&mut push, raw(Axes { translation: [0, 350, 0], rotation: [0; 3] }), 0.02, 600.0, false, false);
        assert_eq!(push.target, before.target);
        assert!(push.distance < before.distance);

        let mut lift = before;
        navigate(&mut lift, raw(Axes { translation: [0, 0, 350], rotation: [0; 3] }), 0.02, 600.0, false, false);
        assert_ne!(lift.target, before.target);
        assert_eq!(lift.distance, before.distance);

        let mut tilt = before;
        navigate(&mut tilt, raw(Axes { translation: [0; 3], rotation: [350, 0, 0] }), 0.02, 600.0, false, false);
        assert_eq!((tilt.yaw, tilt.roll), (before.yaw, before.roll));
        assert_ne!(tilt.pitch, before.pitch);

        let mut roll = before;
        navigate(&mut roll, raw(Axes { translation: [0; 3], rotation: [0, 350, 0] }), 0.02, 600.0, false, false);
        assert_eq!((roll.pitch, roll.roll), (before.pitch, before.roll));
        assert_ne!(roll.yaw, before.yaw);

        let mut twist = before;
        navigate(&mut twist, raw(Axes { translation: [0; 3], rotation: [0, 0, 350] }), 0.02, 600.0, false, false);
        assert_eq!((twist.yaw, twist.pitch), (before.yaw, before.pitch));
        assert_ne!(twist.roll, before.roll);
    }

    #[test]
    fn reverse_zoom_setting_applies_to_spacemouse() {
        let before = Camera::default();
        let motion = raw(Axes { translation: [0, 350, 0], rotation: [0; 3] });
        let mut normal = before;
        navigate(&mut normal, motion, 0.02, 600.0, false, false);
        let mut reversed = before;
        navigate(&mut reversed, motion, 0.02, 600.0, false, true);
        assert!(normal.distance < before.distance);
        assert!(reversed.distance > before.distance);
    }

    #[test]
    fn object_mode_tilts_roll_and_twists_spin() {
        let before = Camera::default();
        let mut tilt = before;
        navigate(&mut tilt, DriverMapping::object_mode().apply(Axes { translation: [0; 3], rotation: [0, 350, 0] }), 0.02, 600.0, false, false);
        assert_eq!((tilt.yaw, tilt.pitch), (before.yaw, before.pitch));
        assert_ne!(tilt.roll, before.roll);
        let mut twist = before;
        navigate(&mut twist, DriverMapping::object_mode().apply(Axes { translation: [0; 3], rotation: [0, 0, 350] }), 0.02, 600.0, false, false);
        assert_ne!(twist.yaw, before.yaw);
        assert_eq!((twist.pitch, twist.roll), (before.pitch, before.roll));
        // The physical tilt right (Ry−) rolls one way and the clockwise twist (Rz+) spins the
        // other, as 3DxWare's profile has them (#1172).
        let tilt_right = DriverMapping::object_mode().apply(Axes { translation: [0; 3], rotation: [0, -350, 0] });
        assert_eq!(tilt_right, Motion { translation: [0.0; 3], rotation: [0.0, 0.0, 1.0] });
        let twist_clockwise = DriverMapping::object_mode().apply(Axes { translation: [0; 3], rotation: [0, 0, 350] });
        assert_eq!(twist_clockwise, Motion { translation: [0.0; 3], rotation: [0.0, -1.0, 0.0] });
        // Translations and the forward tilt are the raw axes.
        let all = Axes { translation: [100, -200, 300], rotation: [-150, 0, 0] };
        assert_eq!(DriverMapping::object_mode().apply(all), DriverMapping::default().apply(all));
    }

    #[test]
    fn the_dead_zone_is_rest() {
        assert!(!Axes::default().moving());
        assert!(!Axes { translation: [20, -20, 0], rotation: [0, 20, -20] }.moving());
        for i in 0..6 {
            let mut values = [0_i16; 6];
            values[i] = 21;
            let axes = Axes { translation: [values[0], values[1], values[2]], rotation: [values[3], values[4], values[5]] };
            assert!(axes.moving(), "axis {i}");
            values[i] = -21;
            let axes = Axes { translation: [values[0], values[1], values[2]], rotation: [values[3], values[4], values[5]] };
            assert!(axes.moving(), "axis {i} reversed");
        }
        assert!(Axes { translation: [i16::MIN, 0, 0], rotation: [0, 0, i16::MAX] }.moving());
    }

    #[test]
    fn a_mapping_never_reaches_past_the_six_axes() {
        let mut mapping = DriverMapping::default();
        mapping.axes[0].output = 6;
        mapping.axes[1].output = usize::MAX;
        let motion = mapping.apply(Axes { translation: [350, 350, 350], rotation: [350; 3] });
        assert_eq!(motion.translation, [0.0, 0.0, 1.0]);
        assert_eq!(motion.rotation, [1.0; 3]);
    }

    #[test]
    fn a_frame_moves_the_view_and_keeps_the_next_frame_coming() {
        let ctx = window(true);
        let mut app = app();
        let before = app.cam;
        assert!(frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02));
        assert!(app.cam.distance < before.distance);
        // At rest, or with no time passed, nothing moves.
        let still = app.cam;
        assert!(!frame(&mut app, &ctx, DriverMapping::default(), Axes::default(), 0.02));
        assert!(frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.0));
        assert_eq!(app.cam.distance, still.distance);
    }

    #[test]
    fn only_the_focused_window_moves() {
        // Two windows side by side (#2077): the cap moves the one with focus and leaves the other.
        let mut app = app();
        let before = app.cam;
        assert!(!frame(&mut app, &window(false), DriverMapping::default(), PUSH, 0.02));
        assert_eq!(app.cam.distance, before.distance);
        assert!(frame(&mut app, &window(true), DriverMapping::default(), PUSH, 0.02));
        assert!(app.cam.distance < before.distance);
    }

    #[test]
    fn an_open_file_picker_holds_the_view_until_it_closes() {
        let ctx = window(true);
        let mut app = app();
        let slot: crate::picker_test_support::Slot = Default::default();
        crate::picker_test_support::open(&mut app, &slot);
        app.pick_open(|_, _| {});
        assert!(app.file_picker_busy());
        let before = app.cam;
        assert!(!frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02));
        assert_eq!(app.cam.distance, before.distance);
        // Cancelled: the 3D mouse moves the view again.
        *slot.borrow_mut() = Some(None);
        crate::file_picker::tick(&mut app, &ctx);
        assert!(!app.file_picker_busy());
        assert!(frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02));
        assert!(app.cam.distance < before.distance);
    }

    #[test]
    fn a_long_gap_between_frames_moves_as_far_as_a_short_one() {
        let ctx = window(true);
        let (mut short, mut long) = (app(), app());
        frame(&mut short, &ctx, DriverMapping::default(), PUSH, 0.05);
        frame(&mut long, &ctx, DriverMapping::default(), PUSH, 30.0);
        assert_eq!(short.cam.distance, long.cam.distance);
        for seconds in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let mut app = app();
            let before = app.cam;
            frame(&mut app, &ctx, DriverMapping::default(), PUSH, seconds);
            assert_eq!(app.cam.distance, before.distance, "{seconds}");
        }
    }

    #[test]
    fn navigation_off_the_start_page_and_a_hostile_speed_leave_the_view_alone() {
        let ctx = window(true);
        let mut off = app();
        off.preferences.space_mouse_enabled = false;
        let mut home = app();
        home.home.open = true;
        let mut nan = app();
        nan.preferences.space_mouse_speed = f32::NAN;
        for mut app in [off, home, nan] {
            let before = app.cam;
            assert!(!frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02));
            assert_eq!(app.cam.distance, before.distance);
        }
        // Speed follows the preference, within 0.1 to 5 times.
        let speeds = [0.5, 2.0, 5.0, f32::MAX, 0.25, 0.0, -3.0];
        let distances: Vec<f64> = speeds
            .iter()
            .map(|&speed| {
                let mut app = app();
                app.preferences.space_mouse_speed = speed;
                frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02);
                assert!(app.cam.is_valid(), "{speed}");
                app.cam.distance
            })
            .collect();
        assert!(distances[1] < distances[0]);
        assert_eq!(distances[3], distances[2], "past 5 times is 5 times");
        // Under 0.1 times is 0.1 times (the f32 preference can't hold 0.1 exactly): still a
        // motion, less than at 0.25.
        assert_eq!(distances[6], distances[5], "under 0.1 times is 0.1 times");
        assert!(distances[4] < distances[5] && distances[5] < app().cam.distance);
    }

    #[test]
    fn wheel_turns_are_dropped_while_the_cap_moves_and_kept_at_rest() {
        let mut guard = WheelGuard::default();
        // At rest, the wheel is the user's.
        let mut events = vec![wheel(100.0), other()];
        guard.filter(false, 10.0, &mut events);
        assert_eq!(events.len(), 2);
        // Moving: 3DxWare's wheel (and a pinch zoom) goes; everything else stays.
        let mut events = vec![wheel(-100.0), other(), egui::Event::Zoom(1.1), wheel(100.0)];
        guard.filter(true, 11.0, &mut events);
        assert_eq!(events, vec![other()]);
        // Just after the cap stops, a queued one still goes.
        let mut events = vec![wheel(100.0)];
        guard.filter(false, 11.0 + WHEEL_QUIET_S - 0.01, &mut events);
        assert!(events.is_empty());
        // Later, the wheel is the user's again.
        let mut events = vec![wheel(100.0)];
        guard.filter(false, 11.0 + WHEEL_QUIET_S + 0.01, &mut events);
        assert_eq!(events.len(), 1);
        // A clock that goes back never swallows the user's wheel.
        let mut events = vec![wheel(100.0)];
        guard.filter(false, 5.0, &mut events);
        assert_eq!(events.len(), 1);
        let mut events = vec![wheel(100.0)];
        guard.filter(false, f64::NAN, &mut events);
        assert_eq!(events.len(), 1);
    }

    /// A homemade device that sends push forward/back as its fifth axis, the other way round, at
    /// ten times 3Dconnexion's range.
    fn homemade() -> solidcraft_space_mouse::Calibration {
        let axis = |source, reversed| solidcraft_space_mouse::AxisCalibration { source, reversed, rest: 0, dead_zone: 0, full: [3500; 2] };
        solidcraft_space_mouse::Calibration {
            dead_zone_share: 0.0,
            kind: Default::default(),
            axes: [axis(0, false), axis(4, true), axis(2, false), axis(3, false), axis(1, false), axis(5, false)],
        }
    }

    #[test]
    fn a_devices_saved_calibration_puts_its_axes_on_3dconnexions() {
        let ctx = window(true);
        let mut reference = app();
        frame(&mut reference, &ctx, DriverMapping::default(), PUSH, 0.02);
        let sent = Axes { translation: [0; 3], rotation: [0, -3500, 0] };
        let mut calibrated = app();
        calibrated.space_mouse.device = Some("hid 1234:5678".into());
        calibrated.preferences.space_mouse_calibrations.insert("hid 1234:5678".into(), homemade());
        assert!(frame(&mut calibrated, &ctx, DriverMapping::default(), sent, 0.02));
        assert_eq!(calibrated.cam, reference.cam);
        // Another device's calibration isn't applied: the reading turns the view as it comes.
        let mut other = app();
        other.space_mouse.device = Some("hid 256f:c63a".into());
        other.preferences.space_mouse_calibrations.insert("hid 1234:5678".into(), homemade());
        frame(&mut other, &ctx, DriverMapping::default(), sent, 0.02);
        assert_ne!(other.cam, reference.cam);
    }

    #[test]
    fn while_calibrating_the_cap_feeds_the_calibration_and_the_view_stays() {
        let ctx = window(true);
        let mut app = app();
        crate::space_mouse_calibration::start(&mut app);
        assert!(app.space_mouse.calibrating.is_none(), "nothing to calibrate without a device");
        app.space_mouse.device = Some("hid 256f:c63a".into());
        crate::space_mouse_calibration::start(&mut app);
        // It asks what kind of device it is first.
        let Some((_, wizard)) = &mut app.space_mouse.calibrating else { panic!("calibration didn't start") };
        assert_eq!(wizard.stage(), solidcraft_space_mouse::Stage::Choose);
        wizard.choose(solidcraft_space_mouse::Kind::ReturnToCenter);
        let before = app.cam;
        assert!(!frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02));
        assert_eq!(app.cam, before);
        assert_eq!(app.space_mouse.raw, PUSH);
        assert!(app.space_mouse.calibrating.as_ref().is_some_and(|(_, wizard)| wizard.progress() > 0.0));
        // Closing Preferences ends it, so the cap moves the view again.
        app.prefs_window.open = false;
        crate::prefs::show(&mut app, &ctx);
        assert!(app.space_mouse.calibrating.is_none());
        assert!(frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02));
    }

    #[test]
    fn a_reading_resting_off_zero_is_kept_for_calibration_and_doesnt_move_the_view() {
        let ctx = window(true);
        // Inside the dead zone and not calibrated: no motion, so no redraw either.
        let mut app = app();
        let resting = Axes { translation: [0, 15, 0], rotation: [0; 3] };
        let before = app.cam;
        assert!(!frame(&mut app, &ctx, DriverMapping::default(), resting, 0.02));
        assert_eq!(app.cam, before);
        // The calibration window gets it as it is, to measure where the cap rests.
        app.space_mouse.device = Some("web 1234:5678".into());
        crate::space_mouse_calibration::start(&mut app);
        frame(&mut app, &ctx, DriverMapping::default(), resting, 0.02);
        assert_eq!(app.space_mouse.raw, resting);
        app.space_mouse.calibrating = None;
        // A device resting far off zero, calibrated: still at rest, moving when pushed.
        let mut offset = homemade();
        for axis in &mut offset.axes {
            axis.rest = 1000;
        }
        app.preferences.space_mouse_calibrations.insert("web 1234:5678".into(), offset);
        let at_rest = Axes { translation: [1000; 3], rotation: [1000; 3] };
        assert!(!frame(&mut app, &ctx, DriverMapping::default(), at_rest, 0.02));
        assert_eq!(app.cam, before);
        assert!(frame(&mut app, &ctx, DriverMapping::default(), Axes { rotation: [1000, 1000 - 3500, 1000], ..at_rest }, 0.02));
    }

    #[test]
    fn another_device_turning_up_mid_calibration_ends_it() {
        let ctx = window(true);
        let mut app = app();
        app.space_mouse.device = Some("hid 256f:c63a".into());
        crate::space_mouse_calibration::start(&mut app);
        frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02);
        app.space_mouse.device = Some("spacenavd".into());
        assert!(frame(&mut app, &ctx, DriverMapping::default(), PUSH, 0.02), "the new device moves the view");
        assert!(app.space_mouse.calibrating.is_none());
        assert!(app.preferences.space_mouse_calibrations.is_empty());
    }

    #[test]
    fn calibrations_are_saved_with_the_preferences() {
        let mut prefs = crate::prefs::Prefs::default();
        prefs.space_mouse_calibrations.insert("hid 1234:5678".into(), homemade());
        let mut saved = serde_json::to_value(&prefs).unwrap();
        assert_eq!(crate::prefs::lenient(&saved), prefs);
        // One that can't be read, or that could spin the view, is dropped on its own.
        let mut spinning = homemade();
        spinning.axes[0].full = [0; 2];
        saved["space_mouse_calibrations"]["spacenavd"] = serde_json::to_value(spinning).unwrap();
        saved["space_mouse_calibrations"]["web 046d:c62b"] = serde_json::json!({ "axes": "garbled" });
        assert_eq!(crate::prefs::lenient(&saved), prefs);
        // One saved before the dead zone setting existed reads with none.
        let mut older = serde_json::to_value(homemade()).unwrap();
        older.as_object_mut().unwrap().remove("dead_zone_share");
        saved["space_mouse_calibrations"]["hid 1234:5678"] = older;
        assert_eq!(crate::prefs::lenient(&saved), prefs);
    }
}
