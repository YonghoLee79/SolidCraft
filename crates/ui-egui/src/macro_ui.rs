//! MACRO tab buttons (`macro.*`, docs/macros.md): recording is session state, so Record, Pause,
//! Resume and Stop run the commands directly. Play replays the loaded macro, asking for its
//! parameters first, or opens a file when nothing is loaded. Save writes the recording.

use egui::{Align2, FontId, Sense, vec2};
use serde_json::{Value, json};

use crate::SolveApp;
use crate::theme::Tokens;

pub(crate) fn toolbar(app: &mut SolveApp, id: &str) {
    match id {
        "macro.record" => {
            if app.run(id, json!({})).is_ok() {
                app.set_status(app.tr("macro.recording_started").to_string(), false);
            }
        }
        "macro.pause" => {
            if app.run(id, json!({})).is_ok() {
                let steps = app.session.macros.steps();
                app.set_status(crate::i18n::format("macro.paused", &[("steps", &steps.to_string())]), false);
            }
        }
        "macro.resume" => {
            if app.run(id, json!({})).is_ok() {
                app.set_status(app.tr("macro.resumed").to_string(), false);
            }
        }
        "macro.stop" => {
            if let Ok(r) = app.run(id, json!({})) {
                let steps = r.get("steps").and_then(Value::as_u64).unwrap_or(0);
                if steps == 0 {
                    app.set_status(app.tr("macro.stopped_empty").to_string(), false);
                } else {
                    app.set_status(crate::i18n::format("macro.stopped", &[("steps", &steps.to_string())]), false);
                }
            }
        }
        "macro.play" => play_button(app),
        "macro.open" => {
            app.pick_open_for(macro_extensions(), |app, path| {
                let Some(path) = path else { return };
                open_picked(app, path, false);
            });
        }
        "macro.save" => {
            let name = format!("{}-macro.json", design_stem(app));
            app.pick_save(&name, &["json", "rs"], |app, path| {
                let Some(path) = path else { return };
                let mut params = json!({"path": path.clone()});
                if path.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("rs"))
                    && let Ok(client) = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../macro-client").canonicalize()
                {
                    params["client_path"] = json!(client.display().to_string());
                }
                match app.run("macro.save", params) {
                    Ok(r) => {
                        let steps = r.get("steps").and_then(Value::as_u64).unwrap_or(0);
                        let saved = r.get("path").and_then(Value::as_str).unwrap_or(&path);
                        app.set_status(crate::i18n::format("macro.saved", &[("steps", &steps.to_string()), ("path", saved)]), false);
                    }
                    Err(e) => app.set_status(format!("{}: {e}", app.tr("cmd.macro_save")), true),
                }
            });
        }
        "macro.clear" => {
            app.rust_macro = None;
            app.macro_run = None;
            if app.run(id, json!({})).is_ok() {
                app.set_status(app.tr("macro.cleared").to_string(), false);
            }
        }
        _ => {
            let _ = app.run(id, json!({}));
        }
    }
}

fn play_button(app: &mut SolveApp) {
    if app.session.macros.playing {
        app.set_status("a macro is playing".to_string(), true);
        return;
    }
    if let Some(path) = app.rust_macro.clone() {
        spawn_rust_macro(app, &path);
        return;
    }
    let loaded = app.session.macros.loaded.clone();
    match loaded {
        Some(m) if !m.params.is_empty() => crate::macro_run::open(app),
        Some(m) if !m.commands.is_empty() => play(app, json!({})),
        _ => {
            app.pick_open_for(macro_extensions(), |app, path| {
                let Some(path) = path else { return };
                open_picked(app, path, true);
            });
        }
    }
}

/// JSON and Rust (`.rs`, and `.exe` on Windows) so the picker opens on both.
fn macro_extensions() -> &'static [&'static str] {
    if cfg!(windows) { &["json", "rs", "exe"] } else { &["json", "rs"] }
}

fn is_json(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// Windows may append the first filter (`.json`) to a selected `.exe` or `.rs`.
fn actual_macro_path(path: String) -> String {
    let p = std::path::Path::new(&path);
    if p.is_file() {
        return path;
    }
    let stripped = path.strip_suffix(".json").or_else(|| path.strip_suffix(".JSON"));
    let Some(stem) = stripped else { return path };
    let alt = std::path::Path::new(stem);
    if !alt.is_file() {
        return path;
    }
    let ext = alt.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("rs") { stem.to_string() } else { path }
}

/// `play_json`: Play runs a JSON macro after loading it. Open of JSON only loads it.
/// Open of a Rust program remembers it; Play starts it.
fn open_picked(app: &mut SolveApp, path: String, play_json: bool) {
    let path = actual_macro_path(path);
    if is_json(&path) {
        app.rust_macro = None;
        match app.run("macro.open", json!({"path": path})) {
            Ok(r) => {
                let steps = r.get("steps").and_then(Value::as_u64).unwrap_or(0);
                app.set_status(crate::i18n::format("macro.opened", &[("steps", &steps.to_string())]), false);
                if play_json {
                    if app.session.macros.loaded.as_ref().is_some_and(|m| !m.params.is_empty()) {
                        crate::macro_run::open(app);
                    } else {
                        play(app, json!({}));
                    }
                }
            }
            Err(e) => app.set_status(format!("{}: {e}", app.tr("cmd.macro_open")), true),
        }
        return;
    }
    app.macro_run = None;
    app.rust_macro = Some(path.clone());
    if play_json {
        spawn_rust_macro(app, &path);
    } else {
        app.set_status(format!("Opened {path}"), false);
    }
}

/// How to start a Rust macro the user picked (a binary, or a `.rs` source).
#[cfg(not(target_arch = "wasm32"))]
struct RustLaunch {
    program: String,
    args: Vec<String>,
    dir: Option<std::path::PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
fn rust_launch(path: &std::path::Path) -> Result<RustLaunch, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "rs" {
        if let Some(exe) = sibling_program(path).or_else(|| built_binary(path)) {
            return Ok(direct(exe));
        }
        return cargo_launch(path);
    }
    if ext == "exe" {
        return if path.is_file() { Ok(direct(path.to_path_buf())) } else { Err(format!("no program {}", path.display())) };
    }
    if path.is_file() {
        return Ok(direct(path.to_path_buf()));
    }
    if let Some(exe) = sibling_program(path) {
        return Ok(direct(exe));
    }
    Err(format!("not a Rust macro: {}", path.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn direct(exe: std::path::PathBuf) -> RustLaunch {
    let dir = exe.parent().map(std::path::Path::to_path_buf);
    RustLaunch { program: exe.display().to_string(), args: Vec::new(), dir }
}

/// `tool` beside `tool.exe`, or `tool.exe` beside `tool.rs`. Windows will not run a `.rs` file.
#[cfg(not(target_arch = "wasm32"))]
fn sibling_program(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let stem = path.file_stem()?;
    let dir = path.parent()?;
    let bare = dir.join(stem);
    let exe = bare.with_extension("exe");
    let cands = if cfg!(windows) { [exe, bare] } else { [bare, exe] };
    cands.into_iter().find(|p| p.is_file() && p != path)
}

#[cfg(not(target_arch = "wasm32"))]
fn built_binary(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let names = candidate_names(path);
    if names.is_empty() {
        return None;
    }
    let mut dir = path.parent()?;
    for _ in 0..8 {
        for profile in ["debug", "release"] {
            let base = dir.join("target").join(profile);
            for name in &names {
                let bare = base.join(name);
                let exe = bare.with_extension("exe");
                let cands = if cfg!(windows) { [exe, bare.clone()] } else { [bare, exe] };
                if let Some(found) = cands.into_iter().find(|p| p.is_file()) {
                    return Some(found);
                }
            }
        }
        dir = dir.parent()?;
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn candidate_names(path: &std::path::Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && stem != "main"
        && stem != "lib"
        && !names.contains(&stem.to_string())
    {
        names.push(stem.to_string());
    }
    if let Some(manifest) = find_manifest(path)
        && let Some(pkg) = package_name(&manifest)
        && !names.contains(&pkg)
    {
        names.push(pkg);
    }
    names
}

#[cfg(not(target_arch = "wasm32"))]
fn find_manifest(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = path.parent()?;
    for _ in 0..8 {
        let cargo = dir.join("Cargo.toml");
        if cargo.is_file() {
            return Some(cargo);
        }
        dir = dir.parent()?;
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn package_name(manifest: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = line.strip_prefix("name") else { continue };
        let rest = rest.trim().strip_prefix('=').unwrap_or(rest).trim().trim_matches('"');
        if !rest.is_empty() && !rest.contains(' ') {
            return Some(rest.to_string());
        }
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn cargo_launch(path: &std::path::Path) -> Result<RustLaunch, String> {
    let manifest = find_manifest(path).ok_or_else(|| format!("no Cargo.toml above {}", path.display()))?;
    let mut args = vec!["run".to_string(), "--manifest-path".to_string(), manifest.display().to_string()];
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && stem != "main"
        && stem != "lib"
    {
        args.push("--bin".into());
        args.push(stem.to_string());
    }
    args.push("--quiet".into());
    Ok(RustLaunch { program: "cargo".into(), args, dir: manifest.parent().map(std::path::Path::to_path_buf) })
}

/// A spawned Rust Play; `tick` waits and collapses undo.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct RustPlay {
    child: std::process::Child,
    checkpoint: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for RustPlay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn tick(app: &mut SolveApp, ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ctx.request_repaint();
        let result = {
            let Some(play) = app.rust_play.as_mut() else { return };
            match play.child.try_wait() {
                Ok(None) => return,
                Ok(Some(status)) => Ok((play.checkpoint, status.success())),
                Err(e) => Err((play.checkpoint, e.to_string())),
            }
        };
        app.rust_play = None;
        match result {
            Ok((id, ok)) => finish_play(app, id, ok),
            Err((id, e)) => {
                let _ = app.session.finish_external_play(id, false);
                app.set_status(format!("Rust macro: {e}"), true);
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn finish_play(app: &mut SolveApp, checkpoint: u64, ok: bool) {
    match app.session.finish_external_play(checkpoint, ok) {
        Ok(()) if ok => app.set_status("Played the Rust macro as one undo step".to_string(), false),
        Ok(()) => app.set_status("Rust macro failed; the design was restored".to_string(), true),
        Err(e) => app.set_status(format!("Rust macro: {e}"), true),
    }
}

fn spawn_rust_macro(app: &mut SolveApp, path: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        app.set_status("Rust macros run in the desktop app", true);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.session.macros.playing || app.rust_play.is_some() {
            app.set_status("a macro is playing".to_string(), true);
            return;
        }
        let Some(port) = app.control_port.or_else(|| std::env::var("SOLIDCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok()).filter(|p| *p != 0))
        else {
            app.set_status("SolidCraft could not start a control port for Rust macros", true);
            return;
        };
        let launch = match rust_launch(std::path::Path::new(path)) {
            Ok(l) => l,
            Err(e) => {
                app.set_status(format!("Rust macro: {e}"), true);
                return;
            }
        };
        let checkpoint = app.session.checkpoint();
        app.session.macros.playing = true;
        let mut cmd = std::process::Command::new(&launch.program);
        cmd.args(&launch.args).env("SOLIDCRAFT_CONTROL_PORT", port.to_string());
        if let Some(dir) = &launch.dir {
            cmd.current_dir(dir);
        }
        if let Some(token) = solidcraft_engine::control_auth::client_token(port, None) {
            cmd.env("SOLIDCRAFT_CONTROL_TOKEN", token);
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            // Release builds have no console. A console-subsystem macro still has to start.
            cmd.creation_flags(0x0800_0000);
        }
        let shown = if launch.program == "cargo" { format!("cargo run {path}") } else { launch.program.clone() };
        match cmd.spawn() {
            Ok(child) => {
                app.rust_play = Some(RustPlay { child, checkpoint });
                app.set_status(format!("Started {shown}"), false);
            }
            Err(e) => {
                let _ = app.session.finish_external_play(checkpoint, false);
                app.set_status(format!("Rust macro: {e}"), true);
            }
        }
    }
}

pub(crate) fn play(app: &mut SolveApp, params: Value) {
    match app.run("macro.play", params) {
        Ok(r) => {
            let ran = r.get("ran").and_then(Value::as_u64).unwrap_or(0);
            app.set_status(crate::i18n::format("macro.played", &[("steps", &ran.to_string())]), false);
        }
        Err(e) => app.set_status(crate::i18n::format("macro.failed", &[("error", &e)]), true),
    }
}

/// A chip that stays up for the whole recording, including while it is paused, and while a Rust Play runs.
pub(crate) fn chip(app: &SolveApp, ctx: &egui::Context) {
    let rust_running = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            app.rust_play.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    };
    if !app.session.macros.recording && !rust_running {
        return;
    }
    let text = if rust_running {
        "Playing Rust macro…".to_string()
    } else {
        let steps = app.session.macros.steps();
        let paused = app.session.macros.paused;
        let id = if paused { "macro.chip_paused" } else { "macro.chip_recording" };
        crate::i18n::format(id, &[("steps", &steps.to_string())])
    };
    let paused = app.session.macros.paused;
    let t = Tokens::get();
    egui::Area::new(egui::Id::new("sc_macro_recording")).anchor(Align2::CENTER_TOP, vec2(0.0, 36.0)).interactable(false).show(ctx, |ui| {
        let color = if rust_running || paused { t.warning } else { t.error };
        let galley = ui.painter().layout_no_wrap(text, FontId::proportional(t.font_label), t.text);
        let size = galley.size() + vec2(22.0, 8.0);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, t.radius_sm, t.overlay);
        ui.painter().circle_filled(rect.left_center() + vec2(10.0, 0.0), 4.0, color);
        ui.painter().galley(rect.min + vec2(18.0, 4.0), galley, t.text);
        crate::scenario::publish_rect("macro:chip", rect);
    });
}

fn design_stem(app: &SolveApp) -> String {
    let name: String = app.session.doc.name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '-' }).collect();
    let name = name.trim_matches([' ', '-']);
    if name.is_empty() { "design".to_string() } else { name.to_string() }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{actual_macro_path, rust_launch};

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sc-macro-launch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_json_suffix_appended_to_an_exe_is_stripped() {
        let d = dir("suffix");
        let exe = d.join("tool.exe");
        std::fs::write(&exe, "bin").unwrap();
        let fake = format!("{}.json", exe.display());
        assert_eq!(actual_macro_path(fake), exe.display().to_string());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_bare_name_uses_the_exe_beside_it() {
        let d = dir("bare");
        let exe = d.join("tool.exe");
        std::fs::write(&exe, "bin").unwrap();
        let launch = rust_launch(&d.join("tool")).unwrap();
        assert_eq!(launch.program, exe.display().to_string());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_sibling_exe_is_preferred_over_the_rs_source() {
        let d = dir("sibling");
        let exe = d.join("tool.exe");
        std::fs::write(d.join("tool.rs"), "fn main() {}").unwrap();
        std::fs::write(&exe, "bin").unwrap();
        let launch = rust_launch(&d.join("tool.rs")).unwrap();
        assert_eq!(launch.program, exe.display().to_string());
        assert!(launch.args.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_built_binary_above_the_source_is_used() {
        let d = dir("built");
        let rs = d.join("examples/macros/bolt-circle.rs");
        let exe = d.join("target/debug/bolt-circle.exe");
        std::fs::create_dir_all(rs.parent().unwrap()).unwrap();
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&rs, "fn main() {}").unwrap();
        std::fs::write(&exe, "bin").unwrap();
        std::fs::write(d.join("Cargo.toml"), "[workspace]\n").unwrap();
        let launch = rust_launch(&rs).unwrap();
        assert_eq!(launch.program, exe.display().to_string());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_source_with_no_binary_runs_through_cargo() {
        let d = dir("cargo");
        let rs = d.join("src/main.rs");
        std::fs::create_dir_all(rs.parent().unwrap()).unwrap();
        std::fs::write(&rs, "fn main() {}").unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"saved-macro\"\nversion = \"0.1.0\"\n").unwrap();
        let launch = rust_launch(&rs).unwrap();
        assert_eq!(launch.program, "cargo");
        assert!(launch.args.iter().any(|a| a == &d.join("Cargo.toml").display().to_string()), "{:?}", launch.args);
        assert!(!launch.args.iter().any(|a| a == "--bin"), "{:?}", launch.args);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_named_source_asks_cargo_for_that_bin() {
        let d = dir("bin");
        std::fs::write(d.join("bolt-circle.rs"), "fn main() {}").unwrap();
        std::fs::write(d.join("Cargo.toml"), "[workspace]\n").unwrap();
        let launch = rust_launch(&d.join("bolt-circle.rs")).unwrap();
        assert_eq!(launch.program, "cargo");
        assert!(launch.args.windows(2).any(|w| w == ["--bin", "bolt-circle"]), "{:?}", launch.args);
        let _ = std::fs::remove_dir_all(&d);
    }
}
