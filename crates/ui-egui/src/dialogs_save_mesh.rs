//! The Save as mesh dialog (`file.save_mesh`): which bodies, the mesh format, and optionally a
//! slicer to open the saved file in. Open in Slicer (`file.open_in_slicer`) shares its slicer
//! list: it opens the selected bodies in the last slicer used, and asks (with this dialog, minus
//! the format) only when none is remembered or the selection is empty.
//!
//! Every place that saves a mesh opens this dialog: File › Save as mesh, the Mesh tab, and the
//! Save As Mesh item of the body menus. The slicer list holds the programs the user added (kept
//! between runs, newest first), then the slicers found on this machine, then "Custom…", which asks
//! for a program and adds it at the top; "Custom…" itself is never the selection. Pressing Delete
//! over a program the user added removes it.

use std::collections::HashMap;

use egui::{Color32, Sense, vec2};
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::slicers::{self, Slicer};

use crate::SolveApp;
use crate::dialogs::{Kind, combo, row_label};
use crate::selection::{BODIES, SelInput};
use crate::theme::Tokens;

/// The formats, as shown and as `file.save_mesh` names them.
/// (label, `format` parameter, file extension).
const FORMATS: [(&str, &str, &str); 3] = [("STL", "stl", "stl"), ("3MF", "3mf", "3mf"), ("3MF with face tags", "3mf-tagged", "3mf")];
/// Edge of a slicer's icon in the list.
const ICON: f32 = 16.0;
/// Most programs the user can keep in the list.
const MAX_CUSTOM: usize = 24;

/// The dialog's values.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveMesh {
    format: usize,
    send: bool,
    /// Open in Slicer: only the bodies and the slicer are asked.
    open: bool,
    /// The chosen slicer: an installed one's id, or the path of one the user added.
    slicer: String,
    /// The slicer list has not been read for this opening yet.
    stale: bool,
}

/// One line of the slicer list.
struct Entry {
    key: String,
    slicer: Slicer,
    /// Added by the user (can be deleted).
    user: bool,
}

/// The slicers known to the dialog, and their icons.
#[derive(Default)]
pub struct SlicerMenu {
    detected: Vec<Slicer>,
    custom: Vec<(String, Slicer)>,
    icons: HashMap<String, Option<egui::TextureHandle>>,
    /// A program chosen in the Custom… file dialog, waiting for the open dialog to take it.
    picked: Option<String>,
}

impl SlicerMenu {
    /// Look at the machine and at the saved programs again.
    pub fn refresh(&mut self, custom_paths: &[String]) {
        self.detected = slicers::detect();
        self.set_custom(custom_paths);
    }

    fn set_custom(&mut self, paths: &[String]) {
        // A program that has been uninstalled drops out of the list (it stays in the settings).
        self.custom = paths.iter().filter_map(|p| slicers::custom(p).map(|s| (p.clone(), s))).collect();
    }

    /// The list key for a remembered one: itself, or for a slicer remembered before it had
    /// several installs (`prusa`), its first install.
    fn key_for(&self, key: &str) -> Option<String> {
        let entries = self.entries();
        let hit = entries.iter().find(|e| e.key == key).or_else(|| entries.iter().find(|e| !e.user && e.slicer.base_id() == key));
        hit.map(|e| e.key.clone())
    }

    fn entries(&self) -> Vec<Entry> {
        let user = self.custom.iter().map(|(path, s)| Entry { key: path.clone(), slicer: s.clone(), user: true });
        let found = self.detected.iter().map(|s| Entry { key: s.id.clone(), slicer: s.clone(), user: false });
        user.chain(found).collect()
    }
}

/// The dialog for `file.save_mesh` or `file.open_in_slicer`, with the choices of last time.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    let open = match id {
        "file.save_mesh" => false,
        "file.open_in_slicer" => true,
        _ => return None,
    };
    let p = &app.preferences;
    let format = FORMATS.iter().position(|f| f.1 == p.save_mesh_format).unwrap_or(0);
    let slicer = if open && !p.open_slicer.is_empty() { p.open_slicer.clone() } else { p.save_mesh_slicer.clone() };
    let kind = Kind::SaveMesh(SaveMesh { format, send: open || p.save_mesh_send, open, slicer, stale: true });
    Some((kind, vec![SelInput::new("Bodies", BODIES, true)]))
}

/// Open in Slicer needs a way to start a program: a browser has none.
pub fn available(app: &SolveApp) -> bool {
    app.services.pick_program.is_some()
}

/// A slicer that was started alone and is being waited for, off the UI thread, until it can take
/// the file (`slicers::WAIT_FOR_READY`). One at a time: a second send while it lasts does nothing.
pub struct SlicerWait {
    done: std::sync::mpsc::Receiver<bool>,
    /// The `file.open_in_slicer` parameters, with the wait switched off, to send once it is ready.
    params: Value,
    slicer: String,
    path: String,
}

/// Look at `ready` on another thread until it says so or `limit` runs out; the answer arrives on the
/// channel.
fn spawn_wait(
    ready: impl FnMut() -> bool + Send + 'static,
    limit: std::time::Duration,
    poll: std::time::Duration,
) -> std::sync::mpsc::Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(slicers::wait_until(ready, limit, poll));
    });
    rx
}

/// Run `file.open_in_slicer` and report what happened. A slicer that has to be started first is
/// started and waited for in the background.
fn send_to_slicer(app: &mut SolveApp, params: Value) {
    let _language = crate::i18n::scope(&app.preferences.language);
    if let Some(wait) = &app.slicer_wait {
        let line = crate::i18n::format("ui.slicer_starting", &[("slicer", &wait.slicer)]);
        app.set_status(line, false);
        return;
    }
    let mut params = params;
    params["single_instance"] = json!(app.preferences.slicer_single_instance);
    params["wait"] = json!("return");
    let Ok(result) = app.run("file.open_in_slicer", params.clone()) else { return };
    let Some(name) = result.get("starting").and_then(Value::as_str) else { return report(app, &result) };
    let spec = params["slicer"].as_str().unwrap_or_default().to_string();
    params["wait"] = json!("no");
    let path = result.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    let Some(slicer) = slicers::resolve(&spec) else { return };
    let done = spawn_wait(move || slicer.is_ready() != Some(false), slicers::READY_LIMIT, slicers::READY_POLL);
    app.slicer_wait = Some(SlicerWait { done, params, slicer: name.to_string(), path });
    app.set_status(crate::i18n::format("ui.slicer_starting", &[("slicer", name)]), false);
}

/// Per frame: when the slicer being waited for is ready, send the file; when the wait ran out, say so.
pub fn tick_wait(app: &mut SolveApp, ctx: &egui::Context) {
    use std::sync::mpsc::TryRecvError;
    let Some(wait) = app.slicer_wait.as_ref() else { return };
    let ready = match wait.done.try_recv() {
        Err(TryRecvError::Empty) => return ctx.request_repaint_after(std::time::Duration::from_millis(200)),
        Ok(ready) => ready,
        Err(TryRecvError::Disconnected) => false,
    };
    let Some(wait) = app.slicer_wait.take() else { return };
    let _language = crate::i18n::scope(&app.preferences.language);
    if ready {
        if let Ok(result) = app.run("file.open_in_slicer", wait.params) {
            report(app, &result);
        }
    } else {
        app.set_status(crate::i18n::format("ui.slicer_no_response", &[("slicer", &wait.slicer), ("path", &wait.path)]), true);
    }
}

/// A popup with the slicer list is open (Delete belongs to it, not to the design's selection).
pub fn popup_open(ctx: &egui::Context) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let at = ctx.data(|d| d.get_temp::<u64>(egui::Id::new("sc_slicer_popup")));
    at.is_some_and(|n| n + 1 >= frame)
}

/// The icon of an entry as a texture (`None`: no icon could be read).
fn icon(menu: &mut SlicerMenu, ctx: &egui::Context, e: &Entry) -> Option<egui::TextureId> {
    let tex = menu.icons.entry(e.key.clone()).or_insert_with(|| {
        let bytes = e.slicer.icon.as_ref()?.load()?;
        let img = decode(&bytes)?;
        Some(ctx.load_texture(format!("slicer-icon-{}", e.key), img, egui::TextureOptions::LINEAR))
    });
    tex.as_ref().map(egui::TextureHandle::id)
}

/// An icon file (PNG or ICO) as a small square image.
fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    const SIDE: u32 = 48;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let small = image::imageops::resize(&img.to_rgba8(), SIDE, SIDE, image::imageops::FilterType::Lanczos3);
    Some(egui::ColorImage::from_rgba_unmultiplied([SIDE as usize, SIDE as usize], small.as_raw()))
}

/// An icon, or the first letter of the name on a tile when there is none.
fn paint_icon(ui: &egui::Ui, rect: egui::Rect, tex: Option<egui::TextureId>, name: &str) {
    match tex {
        Some(id) => {
            ui.painter().image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let t = Tokens::get();
            ui.painter().rect_filled(rect, 3.0, t.chip);
            let letter = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(11.0), t.text);
        }
    }
}

/// The rows after the Bodies input; true when Enter was pressed in a field.
pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut SaveMesh) -> bool {
    if k.stale {
        // Look at the machine once per opening, then settle on a slicer that exists.
        k.stale = false;
        app.slicer_menu.picked = None;
        app.slicer_menu.refresh(&app.preferences.custom_slicers);
        let entries = app.slicer_menu.entries();
        k.slicer = app.slicer_menu.key_for(&k.slicer).or_else(|| entries.first().map(|e| e.key.clone())).unwrap_or_default();
        k.send &= !entries.is_empty() || k.open;
    }
    // A browser has no installed programs to send a file to.
    let can_send = app.services.pick_program.is_some();
    if !k.open {
        row_label(ui, "Format");
        combo(ui, "sm_format", &FORMATS.map(|f| f.0), &mut k.format);
        ui.end_row();
    }
    if !can_send {
        k.send = false;
        return false;
    }
    if !k.open {
        row_label(ui, "Send to slicer");
        ui.checkbox(&mut k.send, "");
        ui.end_row();
    }

    row_label(ui, "Slicer");
    if let Some(path) = app.slicer_menu.picked.take() {
        match add_custom(app, &path) {
            Ok(()) => k.slicer = path,
            Err(e) => app.set_status(e, true),
        }
    }
    let choosing = app.file_picker_busy();
    ui.add_enabled_ui(k.send && !choosing, |ui| slicer_combo(app, ui, k));
    ui.end_row();
    false
}

fn slicer_combo(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut SaveMesh) {
    let entries = app.slicer_menu.entries();
    let ctx = ui.ctx().clone();
    let current = entries.iter().find(|e| e.key == k.slicer);
    let pad = "\u{2003}\u{2003}";
    let text = current.map_or_else(|| crate::i18n::tr_label("No slicer found").to_string(), |e| format!("{pad}{}", e.slicer.name));
    let mut pick_custom = false;
    let mut delete: Option<String> = None;
    let mut chosen: Option<String> = None;
    let r = egui::ComboBox::from_id_salt("sm_slicer").selected_text(text).width(crate::dialogs::FIELD_W).show_ui(ui, |ui| {
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("sc_slicer_popup"), frame));
        for e in &entries {
            let tex = icon(&mut app.slicer_menu, &ctx, e);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            let selected = e.key == k.slicer;
            if resp.hovered() || selected {
                let v = ui.visuals();
                let fill = if resp.hovered() { v.widgets.hovered.bg_fill } else { v.selection.bg_fill.gamma_multiply(0.5) };
                ui.painter().rect_filled(rect, 2.0, fill);
            }
            paint_icon(ui, egui::Rect::from_min_size(rect.left_center() + vec2(3.0, -ICON / 2.0), vec2(ICON, ICON)), tex, &e.slicer.name);
            ui.painter().text(
                rect.left_center() + vec2(ICON + 9.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &e.slicer.name,
                egui::FontId::proportional(13.0),
                ui.visuals().text_color(),
            );
            crate::scenario::publish_handle(&format!("dialog:Slicer:{}", e.slicer.name), rect.center());
            if e.user {
                let hint = crate::i18n::format("ui.slicer_delete_tip", &[("program", e.slicer.command.first().map_or("", String::as_str))]);
                resp.clone().on_hover_text(hint);
                if resp.hovered() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
                    delete = Some(e.key.clone());
                }
            }
            if resp.clicked() {
                chosen = Some(e.key.clone());
                ui.close();
            }
        }
        if !entries.is_empty() {
            ui.separator();
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        ui.painter().text(
            rect.left_center() + vec2(ICON + 9.0, 0.0),
            egui::Align2::LEFT_CENTER,
            crate::i18n::tr_label("Custom…"),
            egui::FontId::proportional(13.0),
            Tokens::get().text,
        );
        crate::scenario::publish_handle("dialog:Slicer:Custom", rect.center());
        if resp.on_hover_text(crate::i18n::tr("ui.custom_program_tip")).clicked() {
            pick_custom = true;
            ui.close();
        }
    });
    if let Some(e) = current {
        let tex = icon(&mut app.slicer_menu, &ctx, e);
        let rect = r.response.rect;
        let at = egui::Rect::from_min_size(rect.left_center() + vec2(ui.spacing().button_padding.x + 1.0, -ICON / 2.0), vec2(ICON, ICON));
        paint_icon(ui, at, tex, &e.slicer.name);
    }
    if let Some(key) = chosen {
        k.slicer = key;
    }
    if let Some(key) = delete {
        app.preferences.custom_slicers.retain(|p| *p != key);
        app.slicer_menu.set_custom(&app.preferences.custom_slicers);
        if k.slicer == key {
            k.slicer = app.slicer_menu.entries().first().map(|e| e.key.clone()).unwrap_or_default();
        }
    }
    if pick_custom {
        // Closing the file dialog without a program leaves the choice as it was.
        app.pick_program(|app, path| app.slicer_menu.picked = path);
    }
}

/// Put a program the user picked at the top of the list (and keep it).
fn add_custom(app: &mut SolveApp, path: &str) -> Result<(), String> {
    if slicers::custom(path).is_none() {
        return Err(crate::i18n::format("ui.not_a_program", &[("path", path)]));
    }
    let list = &mut app.preferences.custom_slicers;
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(MAX_CUSTOM);
    app.slicer_menu.set_custom(&app.preferences.custom_slicers);
    Ok(())
}

fn bodies(inputs: &[SelInput]) -> Vec<String> {
    inputs
        .first()
        .map(|i| i.items.iter().filter_map(|s| if let Sel::Body { name } = s { Some(name.clone()) } else { None }).collect())
        .unwrap_or_default()
}

/// What the Save (or Send) button asks `file.save_mesh` or `file.open_in_slicer` to do; Save is
/// without the file name.
pub fn commands(k: &SaveMesh, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let format = FORMATS.get(k.format).map_or("stl", |f| f.1);
    let mut p = json!({ "format": format });
    let names = bodies(inputs);
    if names.is_empty() {
        return Err(crate::i18n::tr("ui.save_mesh_pick_bodies").into());
    }
    p["bodies"] = json!(names);
    if k.open || k.send {
        if k.slicer.is_empty() {
            return Err(crate::i18n::tr(if k.open { "ui.save_mesh_no_slicer_open" } else { "ui.save_mesh_no_slicer" }).into());
        }
        p["slicer"] = json!(k.slicer);
        if k.open {
            p["format"] = json!("3mf");
        }
        return Ok(vec![("file.open_in_slicer".into(), p)]);
    }
    Ok(vec![("file.save_mesh".into(), p)])
}

/// Save and Send need at least one body.
pub fn ready(inputs: &[SelInput]) -> bool {
    !bodies(inputs).is_empty()
}

/// The button's label: Save, or Send when the file goes to a slicer.
pub fn button(k: &SaveMesh) -> &'static str {
    if k.open {
        "Open"
    } else if k.send {
        "Send"
    } else {
        "Save"
    }
}

/// Save: ask for a file name and write the file. Send: write it to a temporary file and start the
/// slicer with it, with no file dialog.
pub fn run(app: &mut SolveApp, k: &SaveMesh, inputs: &[SelInput]) -> Result<(), String> {
    let (_, params) = commands(k, inputs)?.into_iter().next().ok_or("nothing to save")?;
    if k.open {
        app.preferences.open_slicer = k.slicer.clone();
        send_to_slicer(app, params);
        return Ok(());
    }
    let (format, ext) = FORMATS.get(k.format).map_or(("stl", "stl"), |f| (f.1, f.2));
    let name = bodies(inputs).first().cloned().unwrap_or_else(|| app.session.doc.name.clone());
    let p = &mut app.preferences;
    p.save_mesh_format = format.into();
    p.save_mesh_send = k.send;
    if !k.slicer.is_empty() {
        p.save_mesh_slicer = k.slicer.clone();
    }
    if k.send {
        send_to_slicer(app, params);
    } else {
        // The file dialog answers later; the dialog closes meanwhile, as for every export.
        app.pick_save(&format!("{name}.{ext}"), &[ext], move |app, path| {
            if let Some(path) = path {
                save(app, params, &path);
            }
        });
    }
    Ok(())
}

/// Run `file.save_mesh` into `path` and report what happened.
fn save(app: &mut SolveApp, mut params: Value, path: &str) {
    // Runs from the file picker's answer, outside the frame's language scope.
    let _language = crate::i18n::scope(&app.preferences.language);
    params["path"] = json!(path);
    let Ok(result) = app.run("file.save_mesh", params) else { return };
    report(app, &result);
}

/// The status line after a save or a send.
fn report(app: &mut SolveApp, result: &Value) {
    let path = result.get("path").and_then(Value::as_str).unwrap_or_default();
    match result.get("slicer_error").and_then(Value::as_str) {
        Some(e) => app.set_status(e.to_string(), true),
        None => match result.get("slicer").and_then(Value::as_str) {
            Some(s) => {
                let mut line = match (result.get("numbered").and_then(Value::as_bool), std::path::Path::new(path).file_name()) {
                    (Some(true), Some(file)) => crate::i18n::format("ui.sent_to_slicer_copy", &[("slicer", s), ("file", &file.to_string_lossy())]),
                    _ => crate::i18n::format("ui.sent_to_slicer", &[("slicer", s)]),
                };
                // Said once: sending the same bodies again overwrites the file the slicer has open.
                if result.get("numbered").is_some() && !std::mem::replace(&mut app.preferences.slicer_resend_hint_seen, true) {
                    line = format!("{line}. {}", crate::i18n::tr("ui.slicer_resend_hint"));
                }
                if result.get("stl_fallback").and_then(Value::as_bool) == Some(true) {
                    line = format!("{line}. {}", crate::i18n::tr("ui.slicer_stl_fallback"));
                }
                app.set_status(line, false);
            }
            None => app.set_status(crate::i18n::format("ui.saved_to", &[("path", path)]), false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_program_dialog_open_in_slicer_is_unavailable_and_starts_nothing() {
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        assert!(app.services.pick_program.is_none());
        assert!(!available(&app));
        app.preferences.open_slicer = "prusa".into();
        app.start("file.open_in_slicer");
        assert!(app.dialog.is_none(), "no dialog to choose a slicer in");
        assert!(matches!(&app.status, Some((_, _, true))));
        app.services.pick_program = Some(Box::new(|| Box::pin(async { None })));
        assert!(available(&app));
    }

    /// An app with a box in it, `fake-slicer` (a script that writes its arguments to `seen`) among the
    /// user's programs, and the parameters Open in Slicer would send.
    #[cfg(unix)]
    fn app_with_fake_slicer(tag: &str) -> (SolveApp, std::path::PathBuf, Value) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("solidcraft-slicer-wait-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (script, seen) = (dir.join("fake-slicer"), dir.join("seen"));
        std::fs::write(&script, format!("#!/bin/sh\necho \"$@\" >> {}\n", seen.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        app.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Part"})).unwrap();
        app.preferences.custom_slicers = vec![script.to_string_lossy().into_owned()];
        let params = json!({"slicer": script, "bodies": ["Part"], "wait": "no"});
        (app, seen, params)
    }

    #[cfg(unix)]
    fn sent(seen: &std::path::Path) -> usize {
        for _ in 0..100 {
            if seen.exists() && std::fs::metadata(seen).map(|m| m.len() > 0).unwrap_or(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        std::fs::read_to_string(seen).map(|t| t.lines().count()).unwrap_or(0)
    }

    #[cfg(unix)]
    #[test]
    fn the_menu_entries_always_ask_and_a_script_with_a_slicer_does_not() {
        let (mut app, seen, mut params) = app_with_fake_slicer("ask");
        app.services.pick_program = Some(Box::new(|| Box::pin(async { None })));
        app.run("select.set", json!({"items": [{"type": "body", "name": "Part"}]})).unwrap();
        // Last time's slicer is remembered and bodies are selected: still the picker, with it pre-selected.
        let key = params["slicer"].as_str().unwrap().to_string();
        app.preferences.open_slicer = key.clone();
        app.start("file.open_in_slicer");
        let Some(Kind::SaveMesh(k)) = app.dialog.as_ref().map(|d| d.kind.clone()) else { panic!("no picker opened") };
        assert!(k.open);
        assert_eq!(k.slicer, key, "pre-selected");
        assert!(!seen.exists(), "nothing sent until the user confirms");
        // A script names the slicer: no dialog, the file goes.
        app.dialog = None;
        params["wait"] = json!("no");
        app.run("file.open_in_slicer", params).unwrap();
        assert!(app.dialog.is_none());
        assert_eq!(sent(&seen), 1);
        let _ = std::fs::remove_dir_all(seen.parent().unwrap());
        solidcraft_engine::slicers::set_chosen_programs(&[]);
    }

    #[cfg(unix)]
    #[test]
    fn a_started_slicer_is_waited_for_and_then_sent_the_file_once() {
        let ctx = egui::Context::default();
        let (mut app, seen, params) = app_with_fake_slicer("ready");
        // Ready on the third look; a second click while it waits starts and sends nothing.
        let mut looks = 0;
        let done = spawn_wait(
            move || {
                looks += 1;
                looks == 3
            },
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(5),
        );
        app.slicer_wait = Some(SlicerWait { done, params: params.clone(), slicer: "Fake".into(), path: "/p".into() });
        send_to_slicer(&mut app, params);
        assert!(app.slicer_wait.is_some(), "still the one wait");
        assert!(app.status.as_ref().is_some_and(|(m, _, error)| !*error && m.contains("Fake")), "{:?}", app.status);
        assert!(!seen.exists(), "the second click sent nothing");
        for _ in 0..200 {
            tick_wait(&mut app, &ctx);
            if app.slicer_wait.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.slicer_wait.is_none());
        assert_eq!(sent(&seen), 1, "one send");
        assert!(app.status.as_ref().is_some_and(|(_, _, error)| !*error), "{:?}", app.status);
        let _ = std::fs::remove_dir_all(seen.parent().unwrap());
        solidcraft_engine::slicers::set_chosen_programs(&[]);
    }

    #[cfg(unix)]
    #[test]
    fn a_slicer_that_never_comes_up_is_reported_and_nothing_is_sent() {
        let ctx = egui::Context::default();
        let (mut app, seen, params) = app_with_fake_slicer("late");
        let done = spawn_wait(|| false, std::time::Duration::from_millis(30), std::time::Duration::from_millis(5));
        app.slicer_wait = Some(SlicerWait { done, params, slicer: "Fake".into(), path: "/tmp/part.3mf".into() });
        for _ in 0..400 {
            tick_wait(&mut app, &ctx);
            if app.slicer_wait.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.slicer_wait.is_none());
        let status = app.status.clone().unwrap();
        assert!(status.2 && status.0.contains("/tmp/part.3mf"), "{status:?}");
        assert!(!seen.exists());
        let _ = std::fs::remove_dir_all(seen.parent().unwrap());
    }

    #[test]
    fn open_in_slicer_asks_for_bodies_and_a_slicer_but_not_a_format() {
        let k = SaveMesh { format: 0, send: true, open: true, slicer: "prusa".into(), stale: false };
        let mut input = SelInput::new("Bodies", BODIES, true);
        assert!(commands(&k, std::slice::from_ref(&input)).is_err());
        input.items = vec![Sel::Body { name: "Part".into() }];
        let cmds = commands(&k, std::slice::from_ref(&input)).unwrap();
        assert_eq!(cmds, [("file.open_in_slicer".to_string(), json!({"slicer": "prusa", "bodies": ["Part"], "format": "3mf"}))]);
        // Send to slicer in the Save as mesh dialog goes the same way, in the chosen format.
        let send = SaveMesh { format: 2, open: false, ..k.clone() };
        let (id, p) = commands(&send, std::slice::from_ref(&input)).unwrap().remove(0);
        assert_eq!((id.as_str(), p["format"].as_str()), ("file.open_in_slicer", Some("3mf-tagged")));
        assert_eq!(button(&k), "Open");
        assert!(commands(&SaveMesh { slicer: String::new(), ..k }, &[input]).is_err());
    }
}
