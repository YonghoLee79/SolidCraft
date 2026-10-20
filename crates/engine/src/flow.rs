//! Airflow studies in a session: their setups (kept beside the design, never in it), their runs
//! on a background thread, and their results. The solver comes from the session's flow backend
//! (the app installs the GPU one; without it the CPU reference runs, for small grids).
//!
//! Openings are picked as faces (a point on the face, as other commands take faces):
//! - a duct's open end or slot is picked by its rim face (a face with a hole in it): the opening is
//!   the hole, and an inlet's air goes into the duct, opposite the rim's outward normal;
//! - a face without holes (a fan's outlet face) is the opening itself, its air leaving along the
//!   face's outward normal.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use solidcraft_doc::ModelState;
use solidcraft_geom::{Mesh, Vec2, Vec3};
use solidcraft_sim as sim;

/// The solver studies run on.
pub type FlowBackend = Arc<dyn sim::Backend>;

/// Most cells a study may have on the CPU reference (it is slow), and on any backend.
pub const CPU_MAX_CELLS: u64 = 2_000_000;
pub const MAX_CELLS: u64 = 40_000_000;

#[derive(Default)]
pub struct Studies {
    pub(crate) backend: Option<FlowBackend>,
    pub list: Vec<Study>,
}

impl Studies {
    /// For a scratch session: the same backend, no studies.
    pub(crate) fn scratch(&self) -> Studies {
        Studies { backend: self.backend.clone(), list: Vec::new() }
    }

    pub fn get(&self, name: &str) -> Option<&Study> {
        self.list.iter().find(|s| s.name == name)
    }

    pub(crate) fn get_mut(&mut self, name: &str) -> Option<&mut Study> {
        self.list.iter_mut().find(|s| s.name == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.list.iter().map(|s| s.name.clone()).collect()
    }
}

/// How an opening's region is taken from its face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The hole(s) in the face (a duct's rim).
    Hole,
    /// The face itself (a fan's outlet face).
    Face,
}

/// An inlet or outlet as picked.
#[derive(Clone, Debug, PartialEq)]
pub struct OpeningSpec {
    pub name: String,
    pub body: String,
    /// A point on the face, mm.
    pub point: Vec3,
    pub region: Option<Region>,
    /// Inlets only.
    pub source: Option<sim::Source>,
}

/// A study as set up (all lengths mm).
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub body: String,
    /// The bodies in the air's way (the duct's body among them).
    pub bodies: Vec<String>,
    pub inlets: Vec<OpeningSpec>,
    pub outlets: Vec<OpeningSpec>,
    /// None: chosen from the openings' sizes.
    pub cell: Option<f64>,
    pub pad: f64,
    pub turbulence: bool,
    pub max_steps: u64,
}

pub struct Study {
    pub name: String,
    pub spec: Spec,
    pub job: Option<Job>,
    pub result: Option<Arc<FlowResult>>,
    /// Bumped whenever the result changes, so views know to redraw.
    pub revision: u64,
}

/// A finished run.
pub struct FlowResult {
    pub fields: sim::Fields,
    /// The measurements averaged over the end of the run.
    pub average: sim::Sample,
    pub history: Vec<sim::Sample>,
    pub state: sim::State,
    pub steps: u64,
    pub backend: String,
    pub warnings: Vec<sim::Warning>,
    pub inlets: Vec<String>,
    pub outlets: Vec<String>,
    /// Per inlet: its final speed, m/s (a fan's where it settled on its curve).
    pub speeds: Vec<f64>,
    pub units: sim::Units,
    pub cell: f64,
    pub seconds: f64,
    /// Where each inlet's air starts, mm: its cells' centres (streamline seeds).
    pub seeds: Vec<Vec<Vec3>>,
}

/// What a running study shares with the session.
#[derive(Default)]
pub struct Progress {
    pub steps: u64,
    pub max_steps: u64,
    pub last: Option<sim::Sample>,
    pub backend: String,
    pub cells: usize,
    pub done: Option<std::result::Result<Arc<FlowResult>, String>>,
}

pub struct Job {
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The face of `body` nearest `point`, its triangles' outward normal (area weighted) and how flat
/// it is.
fn face_of(st: &ModelState, body: &str, point: Vec3) -> std::result::Result<(Arc<Mesh>, u32), String> {
    let mb = st.body(body).ok_or_else(|| format!("no body `{body}`"))?;
    let face = solidcraft_doc::appearance::face_index_at(mb, point).ok_or_else(|| format!("no face of `{body}` at {point:?}"))?;
    Ok((mb.mesh(), u32::try_from(face).map_err(|_| "face index out of range".to_string())?))
}

/// Polylines (the face's edges) chained into closed loops.
fn loops(edges: Vec<&Vec<Vec3>>, tol: f64) -> Vec<Vec<Vec3>> {
    let mut left: Vec<Vec<Vec3>> = edges.into_iter().filter(|e| e.len() >= 2).cloned().collect();
    let mut out = Vec::new();
    while let Some(mut chain) = left.pop() {
        // A closed edge on its own (a circle).
        let closed = |c: &Vec<Vec3>| matches!((c.first(), c.last()), (Some(a), Some(b)) if a.dist(*b) <= tol);
        let mut guard = 0;
        while !closed(&chain) && guard < 10_000 {
            guard += 1;
            let Some(end) = chain.last().copied() else { break };
            let Some(i) = left.iter().position(|e| e.first().is_some_and(|p| p.dist(end) <= tol) || e.last().is_some_and(|p| p.dist(end) <= tol))
            else {
                break;
            };
            let mut next = left.swap_remove(i);
            if next.first().is_some_and(|p| p.dist(end) > tol) {
                next.reverse();
            }
            chain.extend(next.into_iter().skip(1));
        }
        if closed(&chain) {
            chain.pop();
            if chain.len() >= 3 {
                out.push(chain);
            }
        }
    }
    out
}

/// The patch an opening covers: its triangles, the normal its air goes along, and whether the cells
/// behind it are to be closed (an inlet at a duct's open end).
pub fn patch(st: &ModelState, o: &OpeningSpec, inlet: bool) -> std::result::Result<(sim::Patch, bool), String> {
    let (mesh, face) = face_of(st, &o.body, o.point)?;
    let mut tris = Vec::new();
    let mut normal = Vec3::ZERO;
    for (t, f) in mesh.triangles.iter().zip(&mesh.tri_face) {
        if *f != face {
            continue;
        }
        let Some([a, b, c]) = mesh.tri(t) else { continue };
        normal += (b - a).cross(c - a);
        tris.push([a, b, c]);
    }
    let n = normal.normalized().ok_or_else(|| format!("the face `{}` picked is empty", o.name))?;
    // Flat: every triangle faces within about 3° of the mean.
    let flat = tris.iter().all(|[a, b, c]| (*b - *a).cross(*c - *a).normalized().is_none_or(|t| t.dot(n) > 0.9986));
    if !flat {
        return Err(format!("the opening `{}` must be a flat face", o.name));
    }
    let size = mesh.bounds().max.dist(mesh.bounds().min).max(1.0);
    let edges: Vec<&Vec<Vec3>> = mesh.edges.iter().zip(&mesh.edge_faces).filter(|(_, fs)| fs.contains(&face)).map(|(e, _)| e).collect();
    let rings = loops(edges, 1e-6 * size + 1e-6);
    let region = o.region.unwrap_or(if rings.len() >= 2 { Region::Hole } else { Region::Face });
    let origin = tris.first().map_or(Vec3::ZERO, |t| t[0]);
    let u = n.any_perp();
    let v = n.cross(u);
    let flat2 = |p: Vec3| Vec2::new((p - origin).dot(u), (p - origin).dot(v));
    let lift = |q: Vec2| origin + u * q.x + v * q.y;
    let area = |l: &[Vec3]| {
        let pts: Vec<Vec2> = l.iter().map(|p| flat2(*p)).collect();
        let mut a = 0.0;
        for (i, p) in pts.iter().enumerate() {
            let q = pts.get((i + 1) % pts.len()).copied().unwrap_or(*p);
            a += p.x * q.y - q.x * p.y;
        }
        0.5 * a
    };
    let triangles = match region {
        Region::Face => tris,
        Region::Hole => {
            if rings.len() < 2 {
                return Err(format!("the face picked for `{}` has no hole in it; pick the rim around the opening", o.name));
            }
            // The outer loop is the largest; every other loop is a hole.
            let outer = rings.iter().enumerate().map(|(i, l)| (i, area(l).abs())).fold((0, 0.0), |best, x| if x.1 > best.1 { x } else { best }).0;
            let mut out = Vec::new();
            for (i, ring) in rings.iter().enumerate() {
                if i == outer {
                    continue;
                }
                let pts: Vec<Vec2> = ring.iter().map(|p| flat2(*p)).collect();
                let mut l = solidcraft_geom::Loop2::polygon(&pts);
                if area(ring) < 0.0 {
                    l = l.reversed();
                }
                let r = solidcraft_geom::Region2 { outer: l, holes: Vec::new() };
                out.extend(r.triangulate(1e-3 * size).into_iter().map(|t| t.map(lift)));
            }
            out
        }
    };
    if triangles.is_empty() {
        return Err(format!("the opening `{}` covers nothing", o.name));
    }
    // An inlet's air goes into the duct through a hole (against the rim's outward normal) or out
    // of a fan's face (along it); an outlet's air leaves along the rim's or face's normal.
    let along = if inlet && region == Region::Hole { n * -1.0 } else { n };
    Ok((sim::Patch { triangles, normal: along }, inlet && region == Region::Hole))
}

/// Hydraulic diameter of a patch (4 A / P), mm, from its triangles' area and outline.
fn width(p: &sim::Patch) -> f64 {
    let area = p.area();
    // The outline: edges used by one triangle only.
    let key = |a: Vec3, b: Vec3| {
        let r = |v: Vec3| [(v.x * 1e4).round() as i64, (v.y * 1e4).round() as i64, (v.z * 1e4).round() as i64];
        let (x, y) = (r(a), r(b));
        if x <= y { (x, y) } else { (y, x) }
    };
    let mut count: std::collections::HashMap<_, (usize, f64)> = std::collections::HashMap::new();
    for [a, b, c] in &p.triangles {
        for (x, y) in [(*a, *b), (*b, *c), (*c, *a)] {
            let e = count.entry(key(x, y)).or_insert((0, x.dist(y)));
            e.0 += 1;
        }
    }
    let perimeter: f64 = count.values().filter(|(n, _)| *n == 1).map(|(_, l)| l).sum();
    if perimeter > 0.0 { 4.0 * area / perimeter } else { area.sqrt() }
}

/// The study's setup for the solver: bodies and patches from the design as it is now.
pub fn setup(st: &ModelState, spec: &Spec, max_cells: u64) -> std::result::Result<(sim::FlowSetup, Vec<Arc<Mesh>>), String> {
    let mut meshes = Vec::new();
    for b in &spec.bodies {
        meshes.push(st.body(b).ok_or_else(|| format!("no body `{b}`"))?.mesh());
    }
    let mut inlets = Vec::new();
    for o in &spec.inlets {
        let (patch, cap) = patch(st, o, true)?;
        let source = o.source.clone().ok_or_else(|| format!("the inlet `{}` needs a speed or a fan", o.name))?;
        inlets.push(sim::Inlet { name: o.name.clone(), patch, source, cap });
    }
    let mut outlets = Vec::new();
    for o in &spec.outlets {
        let (patch, _) = patch(st, o, false)?;
        outlets.push(sim::Outlet { name: o.name.clone(), patch });
    }
    let cell = match spec.cell {
        Some(c) => c,
        None => {
            // Eight cells across the narrowest opening, then larger if the grid would be too big.
            let narrow = inlets.iter().map(|i| width(&i.patch)).chain(outlets.iter().map(|o| width(&o.patch))).fold(f64::INFINITY, f64::min);
            let mut c = (narrow / 8.0).clamp(0.2, 5.0);
            let b = meshes.iter().map(|m| m.bounds()).reduce(|a, b| solidcraft_geom::Aabb3 { min: a.min.min(b.min), max: a.max.max(b.max) });
            if let Some(b) = b {
                let size = b.max - b.min + Vec3::new(2.0, 2.0, 2.0) * spec.pad;
                let volume = size.x.max(c) * size.y.max(c) * size.z.max(c);
                let cells = volume / (c * c * c);
                if cells > max_cells as f64 * 0.9 {
                    c = (volume / (max_cells as f64 * 0.9)).cbrt();
                }
            }
            // A tidy number: two significant figures.
            let scale = 10f64.powf(c.log10().floor() - 1.0);
            (c / scale).ceil() * scale
        }
    };
    let setup =
        sim::FlowSetup { cell, pad: spec.pad, air: sim::AIR, inlets, outlets, turbulence: if spec.turbulence { 0.14 } else { 0.0 }, max_cells };
    Ok((setup, meshes))
}

/// Run `prepared` to the end on `backend`, reporting into `progress`.
fn run_to_end(
    prepared: sim::Prepared,
    backend: &dyn sim::Backend,
    limits: sim::Limits,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
    names: (Vec<String>, Vec<String>),
) -> std::result::Result<Arc<FlowResult>, String> {
    let clock = web_time::Instant::now();
    let seeds: Vec<Vec<Vec3>> = prepared
        .inlets
        .iter()
        .map(|l| {
            l.cells
                .iter()
                .map(|c| {
                    let [i, j, k] = prepared.grid.coords(*c);
                    prepared.grid.center(i, j, k)
                })
                .collect()
        })
        .collect();
    let warnings = prepared.warnings.clone();
    let mut run = sim::Run::new(prepared, backend, limits).map_err(|e| e.to_string())?;
    if let Ok(mut p) = progress.lock() {
        p.backend = run.backend.clone();
        p.max_steps = limits.max_steps;
        p.cells = run.prepared.grid.len();
    }
    let state = run
        .finish(|r| {
            if let Ok(mut p) = progress.lock() {
                p.steps = r.steps;
                p.last = r.last().cloned();
            }
            !cancel.load(Ordering::Relaxed)
        })
        .map_err(|e| e.to_string())?;
    let average = run.average().ok_or("the run measured nothing")?;
    Ok(Arc::new(FlowResult {
        fields: run.fields(),
        average,
        history: run.history.clone(),
        state,
        steps: run.steps,
        backend: run.backend.clone(),
        warnings,
        inlets: names.0,
        outlets: names.1,
        speeds: run.prepared.speeds.clone(),
        units: run.prepared.units,
        cell: run.prepared.grid.spacing,
        seconds: clock.elapsed().as_secs_f64(),
        seeds,
    }))
}

/// Start `prepared` on a thread of its own (or here, where no thread can start or `wait`).
pub fn start(prepared: sim::Prepared, backend: FlowBackend, limits: sim::Limits, names: (Vec<String>, Vec<String>), wait: bool) -> Job {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let cancel = Arc::new(AtomicBool::new(false));
    let job = Job { progress: progress.clone(), cancel: cancel.clone() };
    let work = move |prepared: sim::Prepared| {
        let done =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_to_end(prepared, backend.as_ref(), limits, &progress, &cancel, names)))
                .unwrap_or_else(|_| Err("internal error in the flow solver".into()));
        if let Ok(mut p) = progress.lock() {
            p.done = Some(done);
        }
    };
    // The prepared lattice moves into the thread; if no thread starts it comes back to run here.
    let cell = Arc::new(Mutex::new(Some(prepared)));
    let spawned = if wait {
        None
    } else {
        let (cell, work) = (cell.clone(), work.clone());
        std::thread::Builder::new()
            .name("solidcraft-flow".into())
            .spawn(move || {
                if let Some(p) = cell.lock().ok().and_then(|mut c| c.take()) {
                    work(p);
                }
            })
            .ok()
    };
    if spawned.is_none()
        && let Some(p) = cell.lock().ok().and_then(|mut c| c.take())
    {
        work(p);
    }
    job
}

/// A study's state as `sim.flow.status` reports it.
pub fn status(study: &Study) -> Value {
    let Some(job) = &study.job else {
        return json!({"study": study.name, "state": if study.result.is_some() { "done" } else { "ready" }});
    };
    let Ok(p) = job.progress.lock() else { return json!({"study": study.name, "state": "unknown"}) };
    let state = match &p.done {
        None => "running",
        Some(Ok(_)) => "done",
        Some(Err(_)) => "failed",
    };
    let mut out = json!({"study": study.name, "state": state, "steps": p.steps, "max_steps": p.max_steps, "backend": p.backend, "cells": p.cells});
    if let Some(Err(e)) = &p.done {
        out["error"] = json!(e);
    }
    if let Some(last) = &p.last {
        out["time_s"] = json!(last.time);
        out["inlet_flow_cfm"] = json!(last.inlets.iter().map(|t| t.flow * sim::study::CFM_PER_M3S).sum::<f64>());
    }
    out
}

fn through_json(name: &str, t: &sim::Through) -> Value {
    json!({
        "name": name,
        "flow_m3s": t.flow,
        "flow_cfm": t.flow * sim::study::CFM_PER_M3S,
        "pressure_pa": t.pressure,
        "speed_ms": t.speed,
        "max_speed_ms": t.max_speed,
        "area_mm2": t.area * 1e6,
    })
}

pub fn warning_text(w: &sim::Warning) -> String {
    match w {
        sim::Warning::Leak => "the walls leak: with the outlets closed the air still gets out (a gap, or a wall thinner than a cell)".into(),
        sim::Warning::ThinWalls(at) => format!(
            "{} place(s) where a wall is thinner than a cell, e.g. at {:.1?}; use smaller cells",
            at.len(),
            at.first().map(|p| [p.x, p.y, p.z]).unwrap_or_default()
        ),
        sim::Warning::DroppedCells { opening, cells } => format!("{cells} cell(s) of `{opening}` lie in a wall and are left out"),
        sim::Warning::LowViscosity { tau } => {
            format!("relaxation time {tau:.5}: the air's real viscosity is resolved by the turbulence model (expected at these speeds)")
        }
    }
}

/// A finished study's results as `sim.flow.results` reports them.
pub fn results(name: &str, r: &FlowResult) -> Value {
    let a = &r.average;
    let inlets: Vec<Value> = a.inlets.iter().zip(&r.inlets).map(|(t, n)| through_json(n, t)).collect();
    let outlets: Vec<Value> = a.outlets.iter().zip(&r.outlets).map(|(t, n)| through_json(n, t)).collect();
    let total_out: f64 = a.outlets.iter().map(|t| t.flow).sum();
    let shares: Vec<Value> = a
        .outlets
        .iter()
        .zip(&r.outlets)
        .map(|(t, n)| json!({"name": n, "share": if total_out.abs() > 0.0 { t.flow / total_out } else { 0.0 }}))
        .collect();
    let area_mean = |ts: &[sim::Through]| {
        let area: f64 = ts.iter().map(|t| t.area).sum();
        if area > 0.0 { ts.iter().map(|t| t.pressure * t.area).sum::<f64>() / area } else { 0.0 }
    };
    let speed = r.fields.range(sim::Quantity::Speed).unwrap_or((0.0, 0.0));
    let pressure = r.fields.range(sim::Quantity::Pressure).unwrap_or((0.0, 0.0));
    json!({
        "study": name,
        "state": match r.state { sim::State::Settled => "settled", sim::State::StepLimit => "step limit", sim::State::Running => "running" },
        "steps": r.steps,
        "simulated_s": r.units.time(r.steps),
        "seconds": r.seconds,
        "backend": r.backend,
        "cells": r.fields.fluid.len(),
        "air_cells": r.fields.fluid.iter().filter(|f| **f).count(),
        "cell_mm": r.cell,
        "inlets": inlets,
        "outlets": outlets,
        "outlet_shares": shares,
        "pressure_drop_pa": area_mean(&a.inlets) - area_mean(&a.outlets),
        "inlet_speeds_ms": r.speeds,
        "speed_range_ms": [speed.0, speed.1],
        "pressure_range_pa": [pressure.0, pressure.1],
        "warnings": r.warnings.iter().map(warning_text).collect::<Vec<_>>(),
    })
}
