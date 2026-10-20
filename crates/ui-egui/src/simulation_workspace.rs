//! The Simulation workspace: flow studies on the design's bodies. The view stays the modelling
//! viewport, so a study's faces are picked as in Design. Each study is set up in its dialog
//! (`dialogs_sim`); the results panel on the right follows running studies, reports a finished
//! one's flows and pressures, and chooses what the view draws of it: a section plane coloured by
//! speed or pressure and streamlines coloured by speed, with the bodies drawn see-through.

use std::sync::{Arc, Mutex};

use egui::{Color32, RichText};
use serde_json::json;
use solidcraft_engine::flow::FlowResult;
use solidcraft_engine::geom::Vec3;
use solidcraft_engine::sim::{Fields, Quantity};

use crate::SolveApp;
use crate::dialogs::Kind;
use crate::gpu::GpuScene;
use crate::theme::Tokens;
use crate::workspace_switch::Workspace;

/// What the view draws of a study's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub quantity: Quantity,
    /// The section plane's axis (0 = x, 1 = y, 2 = z), or none.
    pub section: Option<u8>,
    /// Where along its axis, 0..1 of the grid.
    pub offset: f32,
    pub streamlines: bool,
    pub lines: usize,
    pub see_through: bool,
}

impl Default for Display {
    fn default() -> Self {
        Display { quantity: Quantity::Speed, section: Some(0), offset: 0.5, streamlines: true, lines: 48, see_through: true }
    }
}

#[derive(Default)]
pub struct State {
    /// The study whose result the view draws.
    pub shown: Option<String>,
    pub display: Display,
    /// The geometry drawn for the shown result, built when the result or the display changes.
    overlay: Mutex<Option<(OverlayKey, Arc<Overlay>)>>,
    /// Each study's Goal Seek form, as typed.
    pub seek_forms: std::collections::HashMap<String, SeekForm>,
}

#[derive(Clone, PartialEq)]
struct OverlayKey {
    study: String,
    revision: u64,
    display: Display,
}

/// Coloured triangles (three corners each) and line segments, in mm.
#[derive(Default)]
pub struct Overlay {
    tris: Vec<([f32; 3], [u8; 4])>,
    lines: Vec<([f32; 3], [f32; 3], [u8; 4])>,
    /// The colour scale's ends, in the quantity's unit.
    range: (f64, f64),
}

pub fn active(app: &SolveApp) -> bool {
    crate::workspace_switch::current(app) == Workspace::Simulation
}

/// Leaving Simulation closes a study dialog left open; a Design dialog is left alone.
pub(crate) fn close(app: &mut SolveApp) {
    if app.dialog.as_ref().is_some_and(|d| matches!(d.kind, Kind::Sim(_))) {
        crate::dialogs::cancel(app);
    }
}

/// Another document in the window: it opens in Design.
pub fn reset_document(app: &mut SolveApp) {
    app.simulation.shown = None;
    if active(app) {
        close(app);
        app.ui.tab = Workspace::Design.tab().into();
    }
}

fn running(app: &SolveApp) -> bool {
    let studies = app.session.flow_studies();
    studies.list.iter().any(|st| st.job.as_ref().is_some_and(|j| j.progress.lock().is_ok_and(|p| p.done.is_none())))
        || studies.seeks.iter().any(|k| k.state == solidcraft_engine::seek::SeekState::Running)
}

/// Per frame: take in finished runs (a new result is shown at once) and keep the panel moving
/// while a study runs.
pub fn tick(app: &mut SolveApp, ctx: &egui::Context) {
    if app.session.poll_flow() {
        // Show the study that just finished, unless another is shown and still there.
        let finished = app.session.flow_studies().list.iter().rev().find(|st| st.result.is_some()).map(|st| st.name.clone());
        let keep = app.simulation.shown.as_ref().is_some_and(|n| app.session.flow_studies().get(n).is_some_and(|st| st.result.is_some()));
        if !keep {
            app.simulation.shown = finished;
        }
        ctx.request_repaint();
    }
    if running(app) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// The shown study's result, when the Simulation workspace is up.
fn shown(app: &SolveApp) -> Option<(String, u64, Arc<FlowResult>)> {
    if !active(app) {
        return None;
    }
    let name = app.simulation.shown.as_ref()?;
    let st = app.session.flow_studies().get(name)?;
    Some((name.clone(), st.revision, st.result.clone()?))
}

/// What the view's scene depends on here: the shown result and how it is drawn (None: nothing).
pub fn scene_key(app: &SolveApp) -> Option<(String, u64, [u64; 6])> {
    let (name, revision, _) = shown(app)?;
    let d = app.simulation.display;
    let q = match d.quantity {
        Quantity::Speed => 0,
        Quantity::Pressure => 1,
        Quantity::Component(c) => 2 + u64::from(c),
    };
    Some((
        name,
        revision,
        [q, d.section.map_or(9, u64::from), u64::from(d.offset.to_bits()), u64::from(d.streamlines), d.lines as u64, u64::from(d.see_through)],
    ))
}

/// The view draws the bodies see-through while a result is shown with that choice.
pub fn see_through(app: &SolveApp) -> bool {
    app.simulation.display.see_through && shown(app).is_some()
}

/// A colour for `t` in 0..1: dark blue through cyan, green and yellow to dark red.
pub fn colour(t: f64) -> [u8; 4] {
    const STOPS: [(f64, [f64; 3]); 9] = [
        (0.0, [48.0, 18.0, 59.0]),
        (0.13, [70.0, 107.0, 227.0]),
        (0.25, [40.0, 168.0, 248.0]),
        (0.38, [26.0, 228.0, 182.0]),
        (0.5, [106.0, 254.0, 93.0]),
        (0.63, [196.0, 241.0, 52.0]),
        (0.75, [251.0, 185.0, 56.0]),
        (0.88, [234.0, 92.0, 13.0]),
        (1.0, [122.0, 4.0, 3.0]),
    ];
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    for w in STOPS.windows(2) {
        if let [(a, ca), (b, cb)] = w
            && t <= *b
        {
            let f = (t - a) / (b - a);
            let c = |i: usize| (ca[i] + (cb[i] - ca[i]) * f).round().clamp(0.0, 255.0) as u8;
            return [c(0), c(1), c(2), 255];
        }
    }
    [122, 4, 3, 255]
}

fn scale(range: (f64, f64), v: f64) -> f64 {
    if range.1 > range.0 { (v - range.0) / (range.1 - range.0) } else { 0.5 }
}

fn f32s(p: Vec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}

/// The section plane's quads: each air cell on the plane as a square coloured by its value.
fn section(f: &Fields, axis: usize, offset: f32, q: Quantity, range: (f64, f64), out: &mut Overlay) {
    let dims = f.dims;
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let n = dims.get(axis).copied().unwrap_or(1);
    let at = ((f64::from(offset.clamp(0.0, 1.0)) * (n as f64 - 1.0)).round() as usize).min(n.saturating_sub(1));
    let s = f.cell;
    let o = [f.origin.x, f.origin.y, f.origin.z];
    for j in 0..dims.get(v).copied().unwrap_or(0) {
        for i in 0..dims.get(u).copied().unwrap_or(0) {
            let mut c = [0usize; 3];
            c[axis] = at;
            c[u] = i;
            c[v] = j;
            let idx = c[0] + dims[0] * (c[1] + dims[1] * c[2]);
            let Some(value) = f.value(idx, q) else { continue };
            let col = colour(scale(range, value));
            let corner = |du: f64, dv: f64| {
                let mut p = [0.0; 3];
                p[axis] = o[axis] + (at as f64 + 0.5) * s;
                p[u] = o[u] + (i as f64 + du) * s;
                p[v] = o[v] + (j as f64 + dv) * s;
                [p[0] as f32, p[1] as f32, p[2] as f32]
            };
            let (a, b, cc, d) = (corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0));
            out.tris.extend([(a, col), (b, col), (cc, col), (a, col), (cc, col), (d, col)]);
        }
    }
}

/// Streamlines from the inlets, coloured by speed, with a chevron every so often pointing the
/// way the air goes.
fn streamlines(r: &FlowResult, lines: usize, out: &mut Overlay) {
    let f = &r.fields;
    let speed = f.range(Quantity::Speed).unwrap_or((0.0, 1.0));
    let inlets = r.seeds.iter().filter(|s| !s.is_empty()).count().max(1);
    let per = (lines / inlets).max(1);
    let step = 8.0 * f.cell;
    for seeds in &r.seeds {
        if seeds.is_empty() {
            continue;
        }
        let stride = (seeds.len() as f64 / per as f64).max(1.0);
        let mut k = 0.5 * stride;
        while (k as usize) < seeds.len() {
            let Some(seed) = seeds.get(k as usize) else { break };
            k += stride;
            let line = f.streamline(*seed, 4000);
            let mut since = 0.0;
            for w in line.windows(2) {
                let [(a, va), (b, vb)] = w else { continue };
                let col = colour(scale(speed, 0.5 * (va + vb)));
                out.lines.push((f32s(*a), f32s(*b), col));
                since += a.dist(*b);
                if since >= step
                    && let Some(d) = (*b - *a).normalized()
                {
                    since = 0.0;
                    let side = d.any_perp() * (0.8 * f.cell);
                    let back = *b - d * (1.6 * f.cell);
                    out.lines.push((f32s(*b), f32s(back + side), col));
                    out.lines.push((f32s(*b), f32s(back - side), col));
                }
            }
        }
    }
}

fn build(r: &FlowResult, d: Display) -> Overlay {
    let mut out = Overlay::default();
    let range = r.fields.range(d.quantity).unwrap_or((0.0, 1.0));
    out.range = range;
    if let Some(axis) = d.section {
        section(&r.fields, usize::from(axis), d.offset, d.quantity, range, &mut out);
    }
    if d.streamlines {
        streamlines(r, d.lines, &mut out);
    }
    out
}

fn overlay(app: &SolveApp) -> Option<Arc<Overlay>> {
    let (study, revision, result) = shown(app)?;
    let key = OverlayKey { study, revision, display: app.simulation.display };
    let mut cache = app.simulation.overlay.lock().ok()?;
    if let Some((k, o)) = cache.as_ref()
        && *k == key
    {
        return Some(o.clone());
    }
    let built = Arc::new(build(&result, key.display));
    *cache = Some((key, built.clone()));
    Some(built)
}

/// Add the shown result to the view's scene.
pub fn draw(app: &SolveApp, sc: &mut GpuScene) {
    let Some(o) = overlay(app) else { return };
    for (p, c) in &o.tris {
        sc.tri(*p, [0.0; 3], *c);
    }
    for (a, b, c) in &o.lines {
        sc.line(*a, *b, *c, 1.6, false);
    }
}

fn legend(ui: &mut egui::Ui, range: (f64, f64), unit: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(240.0), 14.0), egui::Sense::hover());
    let steps = 48;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let c = colour(f64::from(t0));
        let r = egui::Rect::from_min_max(
            egui::pos2(rect.left() + rect.width() * t0, rect.top()),
            egui::pos2(rect.left() + rect.width() * t1, rect.bottom()),
        );
        ui.painter().rect_filled(r, 0.0, Color32::from_rgb(c[0], c[1], c[2]));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{:.3} {unit}", range.0)).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(format!("{:.3} {unit}", range.1)).small()));
    });
}

const CFM: f64 = solidcraft_engine::sim::study::CFM_PER_M3S;

fn results_table(ui: &mut egui::Ui, r: &FlowResult) {
    let tr = crate::i18n::tr;
    let a = &r.average;
    let total: f64 = a.outlets.iter().map(|t| t.flow).sum();
    egui::Grid::new("sim_results").striped(true).num_columns(5).spacing([10.0, 3.0]).show(ui, |ui| {
        for h in [tr("sim.opening"), "CFM", "Pa", "m/s", tr("sim.share")] {
            ui.label(RichText::new(h).strong().small());
        }
        ui.end_row();
        let row = |ui: &mut egui::Ui, name: &str, t: &solidcraft_engine::sim::Through, share: Option<f64>| {
            ui.label(RichText::new(name).small());
            ui.label(RichText::new(format!("{:.2}", t.flow * CFM)).small());
            ui.label(RichText::new(format!("{:.1}", t.pressure)).small());
            ui.label(RichText::new(format!("{:.2} / {:.2}", t.speed, t.max_speed)).small());
            ui.label(RichText::new(share.map_or(String::new(), |s| format!("{:.0}%", 100.0 * s))).small());
            ui.end_row();
        };
        for (t, n) in a.inlets.iter().zip(&r.inlets) {
            row(ui, n, t, None);
        }
        for (t, n) in a.outlets.iter().zip(&r.outlets) {
            row(ui, n, t, (total.abs() > 0.0).then(|| t.flow / total));
        }
    });
}

/// `ui.simulation`: which study's result the view shows and how (`shown`, `quantity`: speed |
/// pressure, `section`: x | y | z | off, `offset` 0..1, `streamlines`, `lines`, `see_through`);
/// returns them all.
pub fn control(app: &mut SolveApp, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    if let Some(name) = p.get("shown").and_then(serde_json::Value::as_str) {
        if app.session.flow_studies().get(name).is_none() {
            return Err(format!("no study `{name}`"));
        }
        app.simulation.shown = Some(name.to_string());
    }
    let d = &mut app.simulation.display;
    match p.get("quantity").and_then(serde_json::Value::as_str) {
        None => {}
        Some("speed") => d.quantity = Quantity::Speed,
        Some("pressure") => d.quantity = Quantity::Pressure,
        Some(q) => return Err(format!("quantity `{q}`: use speed or pressure")),
    }
    match p.get("section").and_then(serde_json::Value::as_str) {
        None => {}
        Some("off") => d.section = None,
        Some("x") => d.section = Some(0),
        Some("y") => d.section = Some(1),
        Some("z") => d.section = Some(2),
        Some(a) => return Err(format!("section `{a}`: use x, y, z or off")),
    }
    if let Some(o) = p.get("offset").and_then(serde_json::Value::as_f64) {
        d.offset = o.clamp(0.0, 1.0) as f32;
    }
    if let Some(b) = p.get("streamlines").and_then(serde_json::Value::as_bool) {
        d.streamlines = b;
    }
    if let Some(n) = p.get("lines").and_then(serde_json::Value::as_u64) {
        d.lines = (n as usize).clamp(1, 400);
    }
    if let Some(b) = p.get("see_through").and_then(serde_json::Value::as_bool) {
        d.see_through = b;
    }
    let d = app.simulation.display;
    Ok(json!({
        "shown": app.simulation.shown,
        "quantity": if d.quantity == Quantity::Pressure { "pressure" } else { "speed" },
        "section": match d.section { Some(0) => "x", Some(1) => "y", Some(2) => "z", _ => "off" },
        "offset": d.offset,
        "streamlines": d.streamlines,
        "lines": d.lines,
        "see_through": d.see_through,
    }))
}

/// The results panel on the right: one block per study.
pub fn panel(app: &mut SolveApp, ui: &mut egui::Ui) {
    if app.session.flow_studies().list.is_empty() {
        return;
    }
    let tr = crate::i18n::tr;
    let t = Tokens::get();
    let mut run: Vec<(&'static str, serde_json::Value)> = Vec::new();
    egui::Panel::right("sc_sim_results").exact_size(300.0).resizable(false).frame(egui::Frame::NONE.fill(t.panel).inner_margin(10)).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(tr("sim.results"));
            for st in &app.session.flow_studies().list {
                ui.separator();
                let shown = app.simulation.shown.as_deref() == Some(st.name.as_str());
                ui.horizontal(|ui| {
                    if ui.radio(shown, RichText::new(&st.name).strong()).clicked() && st.result.is_some() {
                        app.simulation.shown = Some(st.name.clone());
                    }
                });
                let progress = st.job.as_ref().and_then(|j| {
                    j.progress.lock().ok().map(|p| {
                        (p.steps, p.max_steps, p.last.clone(), p.backend.clone(), p.cells, p.done.as_ref().map(|d| d.as_ref().err().cloned()))
                    })
                });
                match &progress {
                    Some((steps, max, last, backend, cells, None)) => {
                        let frac = if *max > 0 { *steps as f32 / *max as f32 } else { 0.0 };
                        ui.add(egui::ProgressBar::new(frac).text(crate::i18n::format("sim.running", &[("steps", &steps.to_string())])));
                        if let Some(s) = last {
                            let q: f64 = s.inlets.iter().map(|t| t.flow).sum::<f64>() * CFM;
                            ui.label(RichText::new(format!("{q:.2} CFM · {:.3} s · {backend} · {cells} cells", s.time)).small().color(t.text_dim));
                        }
                        if ui.button(tr("sim.cancel")).clicked() {
                            run.push(("sim.flow.cancel", json!({"study": st.name})));
                        }
                    }
                    Some((.., Some(Some(e)))) => {
                        ui.label(RichText::new(e).color(t.warning));
                    }
                    _ => {}
                }
                if let Some(r) = &st.result {
                    results_table(ui, r);
                    let a = &r.average;
                    let mean = |ts: &[solidcraft_engine::sim::Through]| {
                        let area: f64 = ts.iter().map(|t| t.area).sum();
                        if area > 0.0 { ts.iter().map(|t| t.pressure * t.area).sum::<f64>() / area } else { 0.0 }
                    };
                    ui.label(crate::i18n::format("sim.pressure_drop", &[("pa", &format!("{:.2}", mean(&a.inlets) - mean(&a.outlets)))]));
                    ui.label(
                        RichText::new(format!(
                            "{} · {} steps · {:.0} s · {} · {:.2} mm",
                            if r.state == solidcraft_engine::sim::State::Settled { tr("sim.settled") } else { tr("sim.step_limit") },
                            r.steps,
                            r.seconds,
                            r.backend,
                            r.cell
                        ))
                        .small()
                        .color(t.text_dim),
                    );
                    for w in &r.warnings {
                        if !matches!(w, solidcraft_engine::sim::Warning::LowViscosity { .. }) {
                            ui.label(RichText::new(solidcraft_engine::flow::warning_text(w)).small().color(t.warning));
                        }
                    }
                }
                ui.horizontal(|ui| {
                    let busy = matches!(progress, Some((.., None)));
                    if ui.add_enabled(!busy, egui::Button::new(tr("sim.run_again"))).clicked() {
                        run.push(("sim.flow.run", json!({"study": st.name})));
                    }
                    if ui.button(tr("sim.delete")).clicked() {
                        run.push(("sim.flow.delete", json!({"study": st.name})));
                    }
                });
            }
            let names: Vec<String> = app.session.flow_studies().list.iter().filter(|st| st.result.is_some()).map(|st| st.name.clone()).collect();
            for name in names {
                ui.separator();
                seek_block(app, ui, &name, &mut run);
            }
            if let Some((_, _, r)) = shown(app) {
                ui.separator();
                ui.label(RichText::new(tr("sim.display")).strong());
                let d = &mut app.simulation.display;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut d.quantity, Quantity::Speed, tr("sim.speed_map"));
                    ui.selectable_value(&mut d.quantity, Quantity::Pressure, tr("sim.pressure_map"));
                });
                let unit = if d.quantity == Quantity::Pressure { "Pa" } else { "m/s" };
                legend(ui, r.fields.range(d.quantity).unwrap_or((0.0, 1.0)), unit);
                ui.horizontal(|ui| {
                    ui.label(tr("sim.section"));
                    ui.selectable_value(&mut d.section, None, tr("sim.off"));
                    for (axis, name) in [(0u8, "X"), (1, "Y"), (2, "Z")] {
                        ui.selectable_value(&mut d.section, Some(axis), name);
                    }
                });
                if d.section.is_some() {
                    ui.add(egui::Slider::new(&mut d.offset, 0.0..=1.0).show_value(false));
                }
                ui.checkbox(&mut d.streamlines, tr("sim.streamlines"));
                if d.streamlines {
                    ui.add(egui::Slider::new(&mut d.lines, 4..=400).logarithmic(true));
                }
                ui.checkbox(&mut d.see_through, tr("sim.see_through"));
            }
        });
    });
    for (id, params) in run {
        match app.run(id, params) {
            // A goal seek set up from the form starts at once.
            Ok(v) if id == "sim.seek.new" => {
                if let Some(seek) = v.get("seek").and_then(serde_json::Value::as_str)
                    && let Err(e) = app.run("sim.seek.run", json!({"seek": seek}))
                {
                    log::warn!("sim.seek.run: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => {
                if id == "sim.seek.new"
                    && let Some(f) = app.simulation.seek_forms.values_mut().find(|f| f.open || f.error.is_none())
                {
                    f.error = Some(e.clone());
                }
                log::warn!("{id}: {e}");
            }
        }
    }
    if app.simulation.shown.as_ref().is_some_and(|n| app.session.flow_studies().get(n).is_none()) {
        app.simulation.shown = None;
    }
}

// ---------------------------------------------------------------------------------------------
// Goal Seek

/// One varied parameter as typed.
#[derive(Clone, Debug, PartialEq)]
pub struct VaryRow {
    pub parameter: String,
    pub min: String,
    pub max: String,
    pub count: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalKind {
    Maximize,
    Minimize,
    Target,
    /// Even out the outlets' flows.
    Equalize,
}

/// The Goal Seek form under a study's results.
#[derive(Clone, Debug, PartialEq)]
pub struct SeekForm {
    pub open: bool,
    pub vary: Vec<VaryRow>,
    pub optimize: bool,
    pub budget: String,
    pub goal: GoalKind,
    pub measure: String,
    pub quantity: &'static str,
    pub target: String,
    pub error: Option<String>,
}

impl Default for SeekForm {
    fn default() -> Self {
        SeekForm {
            open: false,
            vary: vec![VaryRow { parameter: String::new(), min: String::new(), max: String::new(), count: "5".into() }],
            optimize: false,
            budget: "20".into(),
            goal: GoalKind::Maximize,
            measure: String::new(),
            quantity: "flow",
            target: String::new(),
            error: None,
        }
    }
}

fn parse(text: &str, what: &str) -> Result<f64, String> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("{what}: enter a number"))
}

/// The cycles a form asks for.
fn form_cycles(f: &SeekForm) -> Option<usize> {
    if f.optimize {
        return f.budget.trim().parse().ok();
    }
    f.vary.iter().map(|v| v.count.trim().parse::<usize>().ok()).try_fold(1usize, |n, c| c.map(|c| n.saturating_mul(c)))
}

/// The `sim.seek.new` request the form makes for `study`.
pub fn seek_request(f: &SeekForm, study: &str, outlets: &[String]) -> Result<serde_json::Value, String> {
    let mut vary = Vec::new();
    for v in &f.vary {
        if v.parameter.is_empty() {
            return Err("pick the parameter to vary".into());
        }
        let mut row = json!({"parameter": v.parameter, "min": parse(&v.min, "min")?, "max": parse(&v.max, "max")?});
        if !f.optimize {
            row["count"] = json!(parse(&v.count, "values")?);
        }
        vary.push(row);
    }
    let goal = match f.goal {
        GoalKind::Equalize => json!({"goal": "equalize", "measures": outlets, "quantity": f.quantity}),
        GoalKind::Target => {
            json!({"goal": "target", "measure": f.measure, "quantity": f.quantity, "value": parse(&f.target, "target")? / if f.quantity == "flow" { CFM } else { 1.0 }})
        }
        GoalKind::Maximize => json!({"goal": "maximize", "measure": f.measure, "quantity": f.quantity}),
        GoalKind::Minimize => json!({"goal": "minimize", "measure": f.measure, "quantity": f.quantity}),
    };
    let mut p = json!({"study": study, "vary": vary, "goals": [goal]});
    if f.optimize {
        p["mode"] = json!("optimize");
        p["budget"] = json!(parse(&f.budget, "cycles")?);
    }
    Ok(p)
}

/// A measure of a cycle in the unit shown (CFM for flows).
fn shown_measure(c: &solidcraft_engine::seek::Cycle, measure: &str, quantity: solidcraft_engine::seek::Quantity) -> Option<f64> {
    let v = c.measures.get(measure)?.get(quantity);
    Some(if quantity == solidcraft_engine::seek::Quantity::Flow { v * CFM } else { v })
}

/// The goal a seek is judged on first, as (measure, quantity), for its table and plot.
fn main_measure(k: &solidcraft_engine::seek::Seek) -> Option<(String, solidcraft_engine::seek::Quantity)> {
    use solidcraft_engine::seek::Goal;
    k.spec.goals.iter().find_map(|g| match g {
        Goal::Maximize { measure, quantity, .. }
        | Goal::Minimize { measure, quantity, .. }
        | Goal::Target { measure, quantity, .. }
        | Goal::Limit { measure, quantity, .. } => Some((measure.clone(), *quantity)),
        Goal::Equalize { measures, quantity, .. } => measures.first().map(|m| (m.clone(), *quantity)),
    })
}

/// The measure against the first parameter (one parameter), else the score against the cycle.
fn plot(ui: &mut egui::Ui, k: &solidcraft_engine::seek::Seek) {
    let t = Tokens::get();
    let Some((measure, quantity)) = main_measure(k) else { return };
    let pts: Vec<(f64, f64, bool)> = k
        .cycles
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let x = if k.spec.vary.len() == 1 { *c.values.first()? } else { i as f64 };
            Some((x, shown_measure(c, &measure, quantity)?, c.feasible))
        })
        .collect();
    if pts.len() < 2 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(270.0), 110.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.text_dim.gamma_multiply(0.4)), egui::StrokeKind::Inside);
    let (x0, x1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), q| (a.min(q.0), b.max(q.0)));
    let (y0, y1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), q| (a.min(q.1), b.max(q.1)));
    let inner = rect.shrink(10.0);
    let at = |x: f64, y: f64| {
        let fx = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.5 };
        let fy = if y1 > y0 { (y - y0) / (y1 - y0) } else { 0.5 };
        egui::pos2(inner.left() + inner.width() * fx as f32, inner.bottom() - inner.height() * fy as f32)
    };
    let mut sorted = pts.clone();
    if k.spec.vary.len() == 1 {
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let line: Vec<egui::Pos2> = sorted.iter().map(|q| at(q.0, q.1)).collect();
    p.add(egui::Shape::line(line, egui::Stroke::new(1.5, t.accent)));
    let best = k.best_cycle().map(|(i, _)| i);
    for (i, c) in k.cycles.iter().enumerate() {
        let x = if k.spec.vary.len() == 1 { c.values.first().copied() } else { Some(i as f64) };
        if let (Some(x), Some(y)) = (x, shown_measure(c, &measure, quantity)) {
            let col = if Some(i) == best {
                Color32::from_rgb(250, 190, 60)
            } else if c.feasible {
                t.accent
            } else {
                t.warning
            };
            p.circle_filled(at(x, y), if Some(i) == best { 4.5 } else { 3.0 }, col);
        }
    }
    let unit = if quantity == solidcraft_engine::seek::Quantity::Flow {
        "CFM"
    } else if quantity == solidcraft_engine::seek::Quantity::Pressure {
        "Pa"
    } else {
        "m/s"
    };
    p.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        format!("{measure} {} ({unit})", quantity.id()),
        egui::FontId::proportional(10.0),
        t.text_dim,
    );
    let xl = if k.spec.vary.len() == 1 {
        k.spec.vary.first().map_or(String::new(), |v| v.parameter.clone())
    } else {
        crate::i18n::tr("sim.seek_cycle").to_string()
    };
    p.text(rect.right_bottom() - egui::vec2(4.0, 2.0), egui::Align2::RIGHT_BOTTOM, xl, egui::FontId::proportional(10.0), t.text_dim);
}

/// The Goal Seek block under study `name`: the form, then each of its seeks with its cycles.
fn seek_block(app: &mut SolveApp, ui: &mut egui::Ui, name: &str, run: &mut Vec<(&'static str, serde_json::Value)>) {
    let tr = crate::i18n::tr;
    let t = Tokens::get();
    let Some(st) = app.session.flow_studies().get(name) else { return };
    let openings: Vec<String> =
        st.spec.inlets.iter().chain(&st.spec.outlets).map(|o| o.name.clone()).chain(std::iter::once("pressure_drop".to_string())).collect();
    let outlets: Vec<String> = st.spec.outlets.iter().map(|o| o.name.clone()).collect();
    let last_seconds = st.result.as_ref().map(|r| r.seconds);
    let params: Vec<(String, String)> = app.session.doc.param_rows().into_iter().map(|r| (r.name, r.unit)).collect();
    let f = app.simulation.seek_forms.entry(name.to_string()).or_default();
    if f.measure.is_empty() {
        f.measure = outlets.first().cloned().or_else(|| openings.first().cloned()).unwrap_or_default();
    }
    egui::CollapsingHeader::new(RichText::new(tr("sim.seek")).strong()).id_salt(("seek", name)).default_open(f.open).show(ui, |ui| {
        if params.is_empty() {
            ui.label(RichText::new(tr("sim.seek_no_params")).small().color(t.text_dim));
            return;
        }
        let mut remove = None;
        for (i, v) in f.vary.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("seek_param", name, i))
                    .width(90.0)
                    .selected_text(if v.parameter.is_empty() { tr("sim.seek_parameter") } else { &v.parameter })
                    .show_ui(ui, |ui| {
                        for (p, unit) in &params {
                            ui.selectable_value(&mut v.parameter, p.clone(), format!("{p} ({unit})"));
                        }
                    });
                ui.add(egui::TextEdit::singleline(&mut v.min).desired_width(38.0).hint_text("min"));
                ui.add(egui::TextEdit::singleline(&mut v.max).desired_width(38.0).hint_text("max"));
                if !f.optimize {
                    ui.add(egui::TextEdit::singleline(&mut v.count).desired_width(24.0)).on_hover_text(tr("sim.seek_values"));
                }
                if i > 0 && ui.small_button("−").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            f.vary.remove(i);
        }
        if f.vary.len() < 3 && ui.small_button(tr("sim.seek_add")).clicked() {
            f.vary.push(VaryRow { parameter: String::new(), min: String::new(), max: String::new(), count: "5".into() });
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut f.optimize, false, tr("sim.seek_sweep"));
            ui.selectable_value(&mut f.optimize, true, tr("sim.seek_optimize"));
            if f.optimize {
                ui.add(egui::TextEdit::singleline(&mut f.budget).desired_width(30.0)).on_hover_text(tr("sim.seek_budget"));
            }
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("seek_goal", name))
                .width(80.0)
                .selected_text(tr(match f.goal {
                    GoalKind::Maximize => "sim.seek_max",
                    GoalKind::Minimize => "sim.seek_min",
                    GoalKind::Target => "sim.seek_target",
                    GoalKind::Equalize => "sim.seek_equal",
                }))
                .show_ui(ui, |ui| {
                    for (g, id) in [
                        (GoalKind::Maximize, "sim.seek_max"),
                        (GoalKind::Minimize, "sim.seek_min"),
                        (GoalKind::Target, "sim.seek_target"),
                        (GoalKind::Equalize, "sim.seek_equal"),
                    ] {
                        ui.selectable_value(&mut f.goal, g, tr(id));
                    }
                });
            if f.goal != GoalKind::Equalize {
                egui::ComboBox::from_id_salt(("seek_measure", name)).width(80.0).selected_text(f.measure.clone()).show_ui(ui, |ui| {
                    for o in &openings {
                        ui.selectable_value(&mut f.measure, o.clone(), o);
                    }
                });
            }
            egui::ComboBox::from_id_salt(("seek_quantity", name)).width(60.0).selected_text(f.quantity).show_ui(ui, |ui| {
                for q in ["flow", "pressure", "speed", "max_speed"] {
                    ui.selectable_value(&mut f.quantity, q, q);
                }
            });
            if f.goal == GoalKind::Target {
                ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(40.0).hint_text(if f.quantity == "flow" { "CFM" } else { "" }));
            }
        });
        let cycles = form_cycles(f);
        let estimate = match (cycles, last_seconds) {
            (Some(n), Some(s)) => crate::i18n::format(
                "sim.seek_estimate",
                &[("cycles", &n.to_string()), ("time", &format!("{:.0}", s * (1.0 + 0.5 * n.saturating_sub(1) as f64)))],
            ),
            (Some(n), None) => crate::i18n::format("sim.seek_cycles", &[("cycles", &n.to_string())]),
            _ => String::new(),
        };
        ui.label(RichText::new(estimate).small().color(t.text_dim));
        if let Some(e) = &f.error {
            ui.label(RichText::new(e).small().color(t.warning));
        }
        if ui.button(tr("sim.seek_run")).clicked() {
            match seek_request(f, name, &outlets) {
                Ok(p) => {
                    f.error = None;
                    run.push(("sim.seek.new", p));
                }
                Err(e) => f.error = Some(e),
            }
        }
    });
    // The seeks over this study.
    for k in app.session.flow_studies().seeks.iter().filter(|k| k.spec.study == name) {
        ui.add_space(4.0);
        let total = k.plan.total();
        ui.label(RichText::new(format!("{} · {}/{}", k.name, k.cycles.len(), total)).strong().small());
        if k.state == solidcraft_engine::seek::SeekState::Running {
            ui.add(egui::ProgressBar::new(k.cycles.len() as f32 / total.max(1) as f32));
            if ui.small_button(tr("sim.cancel")).clicked() {
                run.push(("sim.seek.cancel", json!({"seek": k.name})));
            }
        }
        plot(ui, k);
        let best = k.best_cycle().map(|(i, _)| i);
        let main = main_measure(k);
        egui::Grid::new(("seek_cycles", &k.name)).striped(true).spacing([8.0, 2.0]).show(ui, |ui| {
            ui.label(RichText::new("#").small().strong());
            for v in &k.spec.vary {
                ui.label(RichText::new(&v.parameter).small().strong());
            }
            ui.label(RichText::new(main.as_ref().map_or(String::new(), |(m, q)| format!("{m} {}", q.id()))).small().strong());
            ui.label("");
            ui.end_row();
            for (i, c) in k.cycles.iter().enumerate() {
                let mark = if Some(i) == best {
                    "★"
                } else if !c.feasible {
                    "✗"
                } else {
                    ""
                };
                ui.label(RichText::new(format!("{i}{mark}")).small());
                for x in &c.values {
                    ui.label(RichText::new(format!("{x:.3}")).small());
                }
                let shown = main
                    .as_ref()
                    .and_then(|(m, q)| shown_measure(c, m, *q))
                    .map_or_else(|| c.error.clone().unwrap_or_default(), |v| format!("{v:.3}"));
                ui.label(RichText::new(shown).small());
                if c.error.is_none() && ui.small_button(tr("sim.seek_apply")).clicked() {
                    run.push(("sim.seek.apply", json!({"seek": k.name, "cycle": i})));
                }
                ui.end_row();
            }
        });
    }
}
