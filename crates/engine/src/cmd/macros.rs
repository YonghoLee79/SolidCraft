//! UI macros: record the commands a session runs, keep them as a script and replay them.
//!
//! [`Recorder`] lives on the [`Session`]. [`Session::execute`] feeds it every recordable command
//! while a recording runs and is not paused (not `macro.*` itself, not undo/redo, not `live`
//! gesture frames). A saved macro is the script format (`timeline.export_script`, docs/scripts.md)
//! plus a name and named parameters (`{{name}}`), so one file can drive any design. An earlier
//! step's result is kept with `"let"` and read back as `{{name.field}}`.
//!
//! `macro.play` replays steps as commands, each mapped from the world the way a command run from
//! the UI is ([`crate::frames::TopLevel`]). A step that fails rolls the whole playback back, and
//! a successful playback is a single undo step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::frames::TopLevel;
use crate::params::{bad, str_};
use crate::{EngineError, Result, Session};

/// The most commands one macro holds. Recording stops at it; a file with more is refused.
pub const MAX_STEPS: usize = 10_000;
/// The largest macro file read (bytes).
const MAX_MACRO_BYTES: usize = 4 << 20;
/// The most entries `macro.list` reports.
const MAX_LISTED: usize = 200;

/// A recorded or loaded macro: what `macro.play` plays and `macro.save` writes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Macro {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Values `macro.play` binds to `{{name}}` before its steps run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<MacroParam>,
    /// Steps as the script format holds them: `{command, params?, let?}`.
    #[serde(default)]
    pub commands: Vec<Value>,
}

/// One named value a macro asks for when it runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacroParam {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

/// The session's macro recorder. Not part of the design: opening another design starts afresh.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    /// `macro.record` ran and `macro.stop` has not since.
    pub recording: bool,
    /// Recording is on, but commands are not captured until `macro.resume`.
    pub paused: bool,
    /// `macro.play` is replaying (no playback nests inside a playback).
    pub playing: bool,
    /// Steps captured since `macro.record` (kept after stop until the next recording).
    pub captured: Macro,
    /// The macro `macro.play` runs when no file is given: the last recording, or `macro.open`.
    pub loaded: Option<Macro>,
}

/// Whether a command run while a macro records is a step of it. Modelling, inspection and
/// export count (a flatten-and-export macro is those commands). The macro's own commands,
/// file and document lifecycle, undo and view moves do not: replaying them would replace the
/// design or fight the recording.
pub fn recordable(id: &str) -> bool {
    !(id.starts_with("macro.")
        || id.starts_with("file.")
        || id.starts_with("doc.")
        || id.starts_with("view.")
        || id.starts_with("ui.")
        || matches!(id, "edit.undo" | "edit.redo"))
}

impl Recorder {
    /// [`Session::execute`] calls this after a successful top-level command. Paused and playing
    /// sessions capture nothing.
    pub(crate) fn note(&mut self, id: &str, params: &Value) {
        if !self.recording || self.paused || self.playing || !recordable(id) {
            return;
        }
        if self.captured.commands.len() >= MAX_STEPS {
            self.recording = false;
            self.paused = false;
            if !self.captured.commands.is_empty() {
                self.loaded = Some(self.captured.clone());
            }
            return;
        }
        let mut step = serde_json::Map::new();
        step.insert("command".into(), json!(id));
        if let Some(obj) = params.as_object() {
            let mut p = obj.clone();
            p.remove("live");
            if !p.is_empty() {
                step.insert("params".into(), Value::Object(p));
            }
        }
        self.captured.commands.push(Value::Object(step));
    }

    pub fn steps(&self) -> usize {
        self.captured.commands.len()
    }
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("macro.record", "Record Macro", record)
        .at("MACRO", "RECORD")
        .icon("record")
        .noundo()
        .doc_only()
        .enabled(idle)
        .params("name?: the macro's name (saved with it; defaults to the design's name) → {recording, paused, steps}: a new recording starts (what was captured before is dropped); every recordable command is captured until macro.pause or macro.stop"),
    CommandSpec::new("macro.pause", "Pause Recording", pause)
        .at("MACRO", "RECORD")
        .icon("pause")
        .noundo()
        .doc_only()
        .enabled(capturing)
        .params("→ {recording, paused: true, steps}: stop capturing without ending the recording; macro.resume continues it"),
    CommandSpec::new("macro.resume", "Resume Recording", resume)
        .at("MACRO", "RECORD")
        .icon("record")
        .noundo()
        .doc_only()
        .enabled(paused)
        .params("→ {recording, paused: false, steps}: capture again, into the same recording"),
    CommandSpec::new("macro.stop", "Stop Recording", stop)
        .at("MACRO", "RECORD")
        .icon("stop")
        .noundo()
        .doc_only()
        .enabled(recording)
        .params("→ {recording: false, steps, commands}: end the recording; the steps stay loaded until saved, opened over, or a new recording starts"),
    CommandSpec::new("macro.play", "Play Macro", play)
        .at("MACRO", "PLAYBACK")
        .icon("play")
        .enabled(ready)
        .params("commands?: [{command, params?, let?}] | path?: file | name?: file in the macros folder | params?: {name: value} (default: the loaded macro) → {ran, steps}. A step reads a parameter as {{name}} and an earlier step's result as {{let_name.field}}. A failed step rolls the whole playback back"),
    CommandSpec::new("macro.open", "Open Macro", open)
        .at("MACRO", "PLAYBACK")
        .icon("open")
        .noundo()
        .doc_only()
        .enabled(idle)
        .params("path: a .macro.json (or plain script) file → name, steps, params: [{name, label?, default?}]"),
    CommandSpec::new("macro.save", "Save Macro", save)
        .at("MACRO", "PLAYBACK")
        .icon("export")
        .noundo()
        .doc_only()
        .enabled(has_steps)
        .params("name?: a file in the macros folder, path?: any file (default: the macro's name with .macro.json); a .rs path writes a Cargo crate → {path, steps, crate?}"),
    CommandSpec::new("macro.clear", "Clear Macro", clear_macro)
        .at("MACRO", "PLAYBACK")
        .icon("delete")
        .noundo()
        .doc_only()
        .enabled(has_steps)
        .params("→ {cleared: true}: the loaded macro is dropped; a recording in progress keeps recording, empty"),
    CommandSpec::new("macro.list", "List Macros", list).icon("folder").noundo().doc_only().params(
        "→ {recording, paused, steps, dir, macros: [{name, bytes}]}",
    ),
    CommandSpec::new("macro.delete", "Delete Macro", delete)
        .icon("delete")
        .noundo()
        .doc_only()
        .params("name: a saved macro's name (see macro.list)"),
];

fn idle(s: &Session) -> std::result::Result<(), String> {
    if s.macros.recording {
        Err("already recording; stop the recording first".into())
    } else if s.macros.playing {
        Err("a macro is playing".into())
    } else {
        Ok(())
    }
}

fn recording(s: &Session) -> std::result::Result<(), String> {
    if s.macros.recording { Ok(()) } else { Err("nothing is being recorded".into()) }
}

fn capturing(s: &Session) -> std::result::Result<(), String> {
    if s.macros.recording && !s.macros.paused {
        Ok(())
    } else if s.macros.paused {
        Err("recording is paused; resume it first".into())
    } else {
        Err("nothing is being recorded".into())
    }
}

fn paused(s: &Session) -> std::result::Result<(), String> {
    if s.macros.paused { Ok(()) } else { Err("recording is not paused".into()) }
}

fn ready(s: &Session) -> std::result::Result<(), String> {
    idle(s).map_err(|why| match why.as_str() {
        "already recording; stop the recording first" => "stop the recording before playing a macro".into(),
        other => other.into(),
    })
}

fn has_steps(s: &Session) -> std::result::Result<(), String> {
    let loaded = s.macros.loaded.as_ref().is_some_and(|m| !m.commands.is_empty());
    if s.macros.captured.commands.is_empty() && !loaded { Err("nothing has been recorded yet".into()) } else { Ok(()) }
}

fn record(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.record";
    if let Some(v) = p.get("name")
        && !v.is_null()
        && str_(p, "name").is_none()
    {
        return Err(bad(cmd, "`name` must be text"));
    }
    let name = str_(p, "name").map(str::to_string).or_else(|| Some(s.doc.name.clone()));
    s.macros.captured = Macro { name, ..Default::default() };
    s.macros.recording = true;
    s.macros.paused = false;
    Ok(json!({"recording": true, "paused": false, "steps": 0}))
}

fn pause(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.paused = true;
    Ok(json!({"recording": true, "paused": true, "steps": s.macros.steps()}))
}

fn resume(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.paused = false;
    Ok(json!({"recording": true, "paused": false, "steps": s.macros.steps()}))
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.recording = false;
    s.macros.paused = false;
    let steps = s.macros.steps();
    if steps > 0 {
        s.macros.loaded = Some(s.macros.captured.clone());
    }
    Ok(json!({"recording": false, "paused": false, "steps": steps, "commands": s.macros.captured.commands}))
}

fn clear_macro(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.captured.commands.clear();
    s.macros.loaded = None;
    Ok(json!({"cleared": true}))
}

/// The macros folder: `macros` under the per-user data folder. `None` where there is no per-user folder.
pub fn macros_dir() -> Option<PathBuf> {
    crate::recovery::data_dir().map(|d| d.join("macros"))
}

fn named(cmd: &str, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name.len() > 128
        || !name.ends_with(".json")
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
    {
        return Err(bad(cmd, "`name` must be one plain file name ending in .json (letters, numbers, spaces, -_.)"));
    }
    let dir = macros_dir().ok_or_else(|| bad(cmd, "no macros folder on this system; give a path instead"))?;
    Ok(dir.join(name))
}

fn is_param_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// A macro file, or a plain command script. Every step's command must exist.
pub(crate) fn parse_macro(cmd: &str, raw: &Value) -> Result<Macro> {
    let obj = match raw {
        Value::Object(o) => o.clone(),
        Value::Array(steps) => {
            let mut m = serde_json::Map::new();
            m.insert("commands".into(), Value::Array(steps.clone()));
            m
        }
        _ => return Err(bad(cmd, "a macro is a JSON object with a `commands` list")),
    };
    let steps = obj.get("commands").and_then(Value::as_array).ok_or_else(|| bad(cmd, "missing `commands` (a list of {command, params?})"))?;
    if steps.is_empty() {
        return Err(bad(cmd, "the macro holds no steps"));
    }
    if steps.len() > MAX_STEPS {
        return Err(bad(cmd, format!("a macro holds at most {MAX_STEPS} commands")));
    }
    let meta = obj.get("macro").cloned().unwrap_or(Value::Null);
    let mut params = Vec::new();
    if let Some(list) = obj.get("params") {
        let list = list.as_array().ok_or_else(|| bad(cmd, "`params` must be a list of {name, label?, default?}"))?;
        if list.len() > 100 {
            return Err(bad(cmd, "a macro asks for at most 100 parameters"));
        }
        for p in list {
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| is_param_name(n))
                .ok_or_else(|| bad(cmd, "every parameter needs a name (letters, digits, `_` and `-`)"))?;
            if params.iter().any(|q: &MacroParam| q.name == name) {
                return Err(bad(cmd, format!("parameter `{name}` is given twice")));
            }
            params.push(MacroParam {
                name: name.to_string(),
                label: p.get("label").and_then(Value::as_str).map(str::to_string),
                default: p.get("default").filter(|v| !v.is_null()).cloned(),
            });
        }
    }
    let mut commands = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let id = step.get("command").and_then(Value::as_str).ok_or_else(|| bad(cmd, format!("step {}: missing `command`", i + 1)))?;
        if id.starts_with("macro.") {
            return Err(bad(cmd, format!("step {}: a macro cannot run `{id}` itself", i + 1)));
        }
        if crate::cmd::find_command(id).is_none() {
            return Err(bad(cmd, format!("step {}: unknown command `{id}`", i + 1)));
        }
        if let Some(l) = step.get("let")
            && !l.as_str().is_some_and(is_param_name)
        {
            return Err(bad(cmd, format!("step {}: `let` must be one word (letters, digits, `_` and `-`)", i + 1)));
        }
        let mut out = serde_json::Map::new();
        out.insert("command".into(), json!(id));
        if let Some(p) = step.get("params") {
            out.insert("params".into(), p.clone());
        }
        if let Some(l) = step.get("let") {
            out.insert("let".into(), l.clone());
        }
        commands.push(Value::Object(out));
    }
    Ok(Macro {
        name: meta.get("name").and_then(Value::as_str).map(str::to_string).or_else(|| obj.get("name").and_then(Value::as_str).map(str::to_string)),
        description: meta.get("description").and_then(Value::as_str).map(str::to_string),
        params,
        commands,
    })
}

fn read_macro_file(cmd: &str, path: &std::path::Path) -> Result<String> {
    let text =
        solidcraft_io::vfs::read(&path.display().to_string(), MAX_MACRO_BYTES).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    String::from_utf8(text).map_err(|_| bad(cmd, format!("{} is not text", path.display())))
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.open";
    let path = str_(p, "path").ok_or_else(|| bad(cmd, "`path` must be a .macro.json file"))?;
    let text = read_macro_file(cmd, std::path::Path::new(path))?;
    let raw: Value = serde_json::from_str(&text).map_err(|e| bad(cmd, format!("not a command script: {e}")))?;
    let m = parse_macro(cmd, &raw)?;
    let out = json!({"name": m.name.clone().unwrap_or_default(), "steps": m.commands.len(), "params": json!(m.params)});
    s.macros.loaded = Some(m);
    Ok(out)
}

fn play(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.play";
    if s.macros.playing {
        return Err(bad(cmd, "a macro is already playing"));
    }
    let from_arg = match (p.get("commands"), str_(p, "name"), str_(p, "path")) {
        (Some(Value::Array(_)), None, None) => Some(parse_macro(cmd, &json!({"commands": p["commands"].clone()}))?),
        (None, Some(name), None) => {
            let path = named(cmd, name)?;
            let text = read_macro_file(cmd, &path)?;
            let raw: Value = serde_json::from_str(&text).map_err(|e| bad(cmd, format!("not a command script: {e}")))?;
            Some(parse_macro(cmd, &raw)?)
        }
        (None, None, Some(path)) => {
            let text = read_macro_file(cmd, std::path::Path::new(path))?;
            let raw: Value = serde_json::from_str(&text).map_err(|e| bad(cmd, format!("not a command script: {e}")))?;
            Some(parse_macro(cmd, &raw)?)
        }
        (None, None, None) => None,
        _ => return Err(bad(cmd, "give commands, name or path — one of them")),
    };
    let macro_ = match from_arg {
        Some(m) => m,
        None => s
            .macros
            .loaded
            .clone()
            .filter(|m| !m.commands.is_empty())
            .ok_or_else(|| bad(cmd, "nothing has been recorded; give commands, a path or a saved macro's name"))?,
    };
    let given = match p.get("params") {
        None | Some(Value::Null) => &Value::Null,
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err(bad(cmd, "`params` is an object of {name: value}")),
    };
    let mut vars: BTreeMap<String, Value> = BTreeMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for mp in &macro_.params {
        match given.get(&mp.name).or(mp.default.as_ref()) {
            Some(v) => {
                vars.insert(mp.name.clone(), v.clone());
            }
            None => missing.push(mp.name.as_str()),
        }
    }
    if !missing.is_empty() {
        return Err(bad(cmd, format!("macro `{}` needs values for: {}", macro_.name.as_deref().unwrap_or("macro"), missing.join(", "))));
    }
    let steps = macro_.commands.clone();
    let mark = (s.doc.clone(), s.active_sketch, s.pending_moves.clone(), s.redo.clone(), s.undo.len());
    s.macros.playing = true;
    let outcome = (|| {
        let _top = TopLevel::enter();
        for (i, step) in steps.iter().enumerate() {
            let id = step.get("command").and_then(Value::as_str).unwrap_or_default();
            let params = match step.get("params") {
                None | Some(Value::Null) => Value::Null,
                Some(p) => bind(cmd, i, p, &vars)?,
            };
            let r = s.execute(id, &params).map_err(|e| EngineError::Other(format!("macro step {} ({id}): {e}", i + 1)))?;
            if let Some(name) = step.get("let").and_then(Value::as_str) {
                vars.insert(name.to_string(), r);
            }
        }
        Ok(steps.len())
    })();
    s.macros.playing = false;
    match outcome {
        Ok(ran) => {
            s.undo.truncate(mark.4);
            Ok(json!({"ran": ran, "steps": steps.len()}))
        }
        Err(e) => {
            s.doc = mark.0;
            s.active_sketch = mark.1;
            s.pending_moves = mark.2;
            s.redo = mark.3;
            s.undo.truncate(mark.4);
            s.refresh();
            Err(e)
        }
    }
}

/// Replace `{{name}}` and `{{name.field.0}}` in a step's parameters. A string that is one whole
/// reference keeps the value's type; a reference inside a longer string joins it as text.
pub fn bind(cmd: &str, step: usize, v: &Value, vars: &BTreeMap<String, Value>) -> Result<Value> {
    match v {
        Value::Array(a) => Ok(Value::Array(a.iter().map(|x| bind(cmd, step, x, vars)).collect::<Result<Vec<_>>>()?)),
        Value::Object(o) => Ok(Value::Object(o.iter().map(|(k, x)| bind(cmd, step, x, vars).map(|x| (k.clone(), x))).collect::<Result<_>>()?)),
        Value::String(text) => substitute(cmd, step, text, vars),
        other => Ok(other.clone()),
    }
}

fn substitute(cmd: &str, step: usize, text: &str, vars: &BTreeMap<String, Value>) -> Result<Value> {
    if !text.contains("{{") {
        return Ok(json!(text));
    }
    if text.starts_with("{{") && text.ends_with("}}") && text.matches("{{").count() == 1 {
        let expr = &text[2..text.len() - 2];
        return resolve(cmd, step, expr, vars).cloned();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("{{") {
        out.push_str(&rest[..a]);
        let tail = &rest[a + 2..];
        let Some(b) = tail.find("}}") else {
            return Err(bad(cmd, format!("step {}: `{{{{` is not closed in `{text}`", step + 1)));
        };
        let v = resolve(cmd, step, &tail[..b], vars)?;
        let piece = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.push_str(&piece);
        rest = &tail[b + 2..];
    }
    out.push_str(rest);
    Ok(json!(out))
}

fn resolve<'v>(cmd: &str, step: usize, expr: &str, vars: &'v BTreeMap<String, Value>) -> Result<&'v Value> {
    let mut parts = expr.trim().split('.');
    let name = parts.next().unwrap_or_default();
    let mut cur =
        vars.get(name).ok_or_else(|| bad(cmd, format!("step {}: `{{{{{name}}}}}` is not a macro parameter or an earlier step's `let`", step + 1)))?;
    for part in parts {
        cur = match (cur, part.parse::<usize>()) {
            (Value::Object(o), _) => o.get(part).ok_or_else(|| bad(cmd, format!("step {}: `{{{{{expr}}}}}` reaches no value", step + 1)))?,
            (Value::Array(a), Ok(i)) => a.get(i).ok_or_else(|| bad(cmd, format!("step {}: `{{{{{expr}}}}}` is past the end of a list", step + 1)))?,
            _ => return Err(bad(cmd, format!("step {}: `{{{{{expr}}}}}` reaches no value", step + 1))),
        };
    }
    Ok(cur)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.save";
    let name = p.get("name").filter(|v| !v.is_null());
    let path = p.get("path").filter(|v| !v.is_null());
    if name.is_some() && path.is_some() {
        return Err(bad(cmd, "give a name or a path, not both"));
    }
    let m = if !s.macros.captured.commands.is_empty() {
        s.macros.captured.clone()
    } else if let Some(m) = s.macros.loaded.clone().filter(|m| !m.commands.is_empty()) {
        m
    } else {
        return Err(bad(cmd, "nothing has been recorded yet"));
    };
    let path = match (name, path) {
        (Some(name), None) => {
            let name = name.as_str().ok_or_else(|| bad(cmd, "`name` must be text"))?;
            named(cmd, name)?.display().to_string()
        }
        (None, Some(path)) => path.as_str().ok_or_else(|| bad(cmd, "`path` must be a file path"))?.to_string(),
        _ => {
            let stem = m.name.as_deref().filter(|n| !n.is_empty()).unwrap_or("macro");
            let name = format!("{stem}.macro.json");
            named(cmd, &name)?.display().to_string()
        }
    };
    if path.ends_with(".rs") {
        return save_rust_crate(&path, &m, p);
    }
    let script = json!({
        "macro": {"name": m.name, "description": m.description},
        "params": m.params,
        "commands": m.commands,
        "about": {"replay": "macro.play {path}, or a Rust macro via solidcraft-macro-client"},
    });
    let text = serde_json::to_string_pretty(&script).map_err(|e| EngineError::Other(e.to_string()))?;
    if let Some(parent) = Path::new(&path).parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = solidcraft_io::vfs::create_dir_all(&parent.display().to_string())
    {
        return Err(EngineError::Other(format!("{}: {e}", parent.display())));
    }
    solidcraft_io::vfs::write(&path, text.as_bytes()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "steps": m.commands.len()}))
}

fn save_rust_crate(path: &str, m: &Macro, p: &Value) -> Result<Value> {
    let crate_dir = Path::new(path).with_extension("");
    if crate_dir.as_os_str().is_empty() {
        return Err(bad("macro.save", "path has no crate name"));
    }
    let stem = crate_dir.file_name().and_then(|s| s.to_str()).unwrap_or("macro");
    let pkg = crate_pkg_name(stem);
    let client = match str_(p, "client_path") {
        Some(s) => PathBuf::from(s),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../macro-client"),
    };
    let client = std::fs::canonicalize(&client).unwrap_or(client);
    let src_dir = crate_dir.join("src");
    let src_dir_s = src_dir.display().to_string();
    solidcraft_io::vfs::create_dir_all(&src_dir_s).map_err(|e| EngineError::Other(format!("{src_dir_s}: {e}")))?;
    let main = crate_dir.join("src").join("main.rs");
    let main_s = main.display().to_string();
    solidcraft_io::vfs::write(&main_s, rust_source(m).as_bytes()).map_err(|e| EngineError::Other(format!("{main_s}: {e}")))?;
    let client_toml = client.display().to_string().replace('\\', "/").replace('"', "");
    let cargo = format!(
        "[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nserde_json = \"1\"\nsolidcraft-macro-client = {{ path = \"{client_toml}\" }}\n"
    );
    let cargo_s = crate_dir.join("Cargo.toml").display().to_string();
    solidcraft_io::vfs::write(&cargo_s, cargo.as_bytes()).map_err(|e| EngineError::Other(format!("{cargo_s}: {e}")))?;
    Ok(json!({"path": crate_dir.display().to_string(), "steps": m.commands.len(), "crate": true}))
}

fn crate_pkg_name(stem: &str) -> String {
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return "solidcraft-macro".into();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) { format!("macro-{out}") } else { out }
}

fn saved_macros() -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Some(dir) = macros_dir() else { return out };
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".json")
            && entry.path().is_file()
            && let Ok(len) = entry.path().metadata().map(|m| m.len())
        {
            out.push((name, len));
        }
    }
    out.sort();
    out.truncate(MAX_LISTED);
    out
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let dir = macros_dir().map(|d| d.display().to_string());
    let macros: Vec<Value> = saved_macros().into_iter().map(|(name, bytes)| json!({"name": name, "bytes": bytes})).collect();
    Ok(json!({"recording": s.macros.recording, "paused": s.macros.paused, "steps": s.macros.steps(), "dir": dir, "macros": macros}))
}

fn delete(_: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.delete";
    let name = str_(p, "name").ok_or_else(|| bad(cmd, "`name` is required (see macro.list)"))?;
    let path = named(cmd, name)?;
    solidcraft_io::vfs::remove(&path.display().to_string()).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    Ok(json!({"deleted": name}))
}

/// A recorded macro as a Rust program that talks to the live app (`solidcraft-macro-client`).
pub fn rust_source(m: &Macro) -> String {
    let mut out = String::from(
        "//! Generated SolidCraft macro. `cargo run` in this crate talks to a live app\n\
         //! (`SOLIDCRAFT_CONTROL_PORT`) or use `App::headless()` for `solidcraft-cli mcp`.\n\
         use serde_json::Value;\n\
         use solidcraft_macro_client::App;\n\n\
         fn main() -> Result<(), Box<dyn std::error::Error>> {\n",
    );
    out.push_str("    let mut app = App::connect()?;\n");
    if !m.params.is_empty() {
        out.push_str("    // This recording asked for: ");
        out.push_str(&m.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
        out.push_str(".\n");
    }
    for step in &m.commands {
        let id = step.get("command").and_then(Value::as_str).unwrap_or("");
        let params = step.get("params").cloned().unwrap_or(Value::Null);
        let raw = serde_json::to_string(&params).unwrap_or_else(|_| "null".into());
        let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str("    app.run(\"");
        out.push_str(id);
        out.push_str("\", serde_json::from_str::<Value>(\"");
        out.push_str(&escaped);
        out.push_str("\")?)?;\n");
    }
    out.push_str("    Ok(())\n}\n");
    out
}

impl Session {
    /// The recorder's current state (the UI shows it while recording or paused).
    pub fn macro_state(&self) -> Value {
        json!({"recording": self.macros.recording, "paused": self.macros.paused, "steps": self.macros.steps()})
    }
}
