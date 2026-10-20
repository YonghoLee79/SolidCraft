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
    app.session.flow_studies().list.iter().any(|st| st.job.as_ref().is_some_and(|j| j.progress.lock().is_ok_and(|p| p.done.is_none())))
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
        let r = egui::Rect::from_min_max(egui::pos2(rect.left() + rect.width() * t0, rect.top()), egui::pos2(rect.left() + rect.width() * t1, rect.bottom()));
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

/// The results panel on the right: one block per study.
pub fn panel(app: &mut SolveApp, ui: &mut egui::Ui) {
    if app.session.flow_studies().list.is_empty() {
        return;
    }
    let tr = crate::i18n::tr;
    let t = Tokens::get();
    let mut run: Vec<(&'static str, serde_json::Value)> = Vec::new();
    egui::Panel::right("sc_sim_results")
        .exact_size(300.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10))
        .show(ui, |ui| {
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
                    let progress = st.job.as_ref().and_then(|j| j.progress.lock().ok().map(|p| (p.steps, p.max_steps, p.last.clone(), p.backend.clone(), p.cells, p.done.as_ref().map(|d| d.as_ref().err().cloned()))));
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
                            RichText::new(format!("{} · {} steps · {:.0} s · {} · {:.2} mm", if r.state == solidcraft_engine::sim::State::Settled { tr("sim.settled") } else { tr("sim.step_limit") }, r.steps, r.seconds, r.backend, r.cell))
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
        if let Err(e) = app.run(id, params) {
            log::warn!("{id}: {e}");
        }
    }
    if app.simulation.shown.as_ref().is_some_and(|n| app.session.flow_studies().get(n).is_none()) {
        app.simulation.shown = None;
    }
}
