//! Simulation studies: Airflow (steady, low-speed flow through ducts, fans and cooling paths),
//! Aerodynamics (external flow around a body: drag and lift forces) and Gas Dynamics (transient
//! compressible flow with shocks: pressure pulses, blast, nozzles).
//!
//! Airflow studies run: `sim.flow.new` sets one up (inlets and outlets picked as faces), `sim.flow.run`
//! starts it on a background thread, `sim.flow.status` follows it, `sim.flow.results` reports the
//! flows and pressures. Studies live beside the design, never in it, so nothing here touches the
//! design or its undo history. Aerodynamics and Gas Dynamics check their inputs and are refused with
//! "not supported yet".

use serde_json::{Value, json};
use solidcraft_sim as sim;

use super::CommandSpec;
use crate::flow::{self, OpeningSpec, Region, Spec};
use crate::params::{bad, bool_, num, str_};
use crate::{EngineError, Result, Session};

const OPENING_DOC: &str = "Openings are faces: `{\"face\": [x, y, z] (a point on it) or its name, \"body\"?: name (default the study's body), \"name\"?: label, \"region\"?: \"hole\" | \"face\"}`. A face with a hole in it (a duct's rim) stands for the hole; a face without one (a fan's outlet face) for itself.";

pub static COMMANDS: &[CommandSpec] = &[
CommandSpec::new("sim.flow.new", "Airflow Study", flow_new).noundo().at("SIMULATION", "STUDY").icon("sim_flow")
    .params("Set up an Airflow study (steady, low-speed air through ducts, fans and cooling paths); `sim.flow.run` runs it. body: the duct's body; inlets: list of openings each with `speed` (m/s) or `fan`: {\"type\": \"axial\" | \"radial\" | \"linear\", \"free_flow_cfm\", \"max_pressure_pa\"} or {\"curve\": [[cfm, pa], ...]}; outlets: list of openings (where the air is expected to leave; measured, and the air flows on past them into open air); bodies?: every body in the air's way (default: all visible bodies); cell?: cell size, mm (default: about 8 cells across the narrowest opening); pad?: open air around the bodies, mm (default 20); turbulence?: bool (default true); max_steps?: (default 200000); name?: the study's name. The design is unchanged. Openings: see sim.flow.edit."),
CommandSpec::new("sim.flow.edit", "Edit Airflow Study", flow_edit).noundo()
    .params("Change a study's setup: study: its name; any of sim.flow.new's keys (inlets/outlets replace the lists). Its results are cleared."),
CommandSpec::new("sim.flow.run", "Run Airflow Study", flow_run).noundo()
    .params("Run a study until the flow settles: study: its name; wait?: bool (default false: it runs in the background and sim.flow.status follows it); max_steps?, tolerance? (settled when the flows and pressures change by less than this fraction, default 0.005). Uses the GPU when the app has one, else the CPU (small grids only)."),
CommandSpec::new("sim.flow.status", "Airflow Study Status", flow_status).noundo()
    .params("study?: its name (default: every study). State (ready, running, done, failed), steps taken, the solver."),
CommandSpec::new("sim.flow.results", "Airflow Study Results", flow_results).noundo()
    .params("study: its name. Flow (m³/s and CFM), mean pressure (Pa, gauge), mean and top speed (m/s) per inlet and outlet, each outlet's share of the flow, the pressure drop, the ranges of speed and pressure, and setup warnings (leaks, thin walls)."),
CommandSpec::new("sim.flow.probe", "Probe Airflow Result", flow_probe).noundo()
    .params("study: its name; point: [x, y, z] mm. The velocity (m/s) and pressure (Pa) there, or null outside the air."),
CommandSpec::new("sim.flow.cancel", "Cancel Airflow Study", flow_cancel).noundo()
    .params("study: its name. Stops its run; the last results stay."),
CommandSpec::new("sim.flow.delete", "Delete Airflow Study", flow_delete).noundo()
    .params("study: its name."),
CommandSpec::new("sim.flow.list", "Airflow Studies", flow_list).noundo()
    .params("The studies set up in this session with their setups."),
CommandSpec::new("sim.aero.new", "Aerodynamics Study", aero_new).noundo().at("SIMULATION", "STUDY").icon("sim_aero")
    .params("body: the solid body the air flows around. External flow at a wind speed, reporting drag and lift forces. Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
CommandSpec::new("sim.gas.new", "Gas Dynamics Study", gas_new).noundo().at("SIMULATION", "STUDY").icon("sim_gas")
    .params("body: the solid body the gas flows through or around. Transient compressible flow with shocks (pressure pulses, blast, nozzles). Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
];

fn aero_new(s: &mut Session, params: &Value) -> Result<Value> {
    refused(s, params, "sim.aero.new", "aerodynamics")
}

fn gas_new(s: &mut Session, params: &Value) -> Result<Value> {
    refused(s, params, "sim.gas.new", "gas dynamics")
}

fn body_param<'a>(s: &Session, params: &'a Value, cmd: &str) -> Result<&'a str> {
    let body = params.get("body").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`body` is required: the name of the solid body to study"))?;
    if s.world_state().body(body).is_none() {
        return Err(bad(cmd, format!("no body `{body}`")));
    }
    Ok(body)
}

/// Check the request as the finished study will, then refuse it: there is no solver to run.
fn refused(s: &mut Session, params: &Value, cmd: &str, solver: &str) -> Result<Value> {
    body_param(s, params, cmd)?;
    Err(EngineError::Other(format!("not supported yet: the {solver} solver is not built")))
}

const CFM: f64 = sim::study::CFM_PER_M3S;

fn source(cmd: &str, o: &Value, name: &str) -> Result<sim::Source> {
    if let Some(v) = o.get("speed") {
        let v = v
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 100.0)
            .ok_or_else(|| bad(cmd, format!("`{name}`: `speed` is m/s, above 0 and at most 100")))?;
        return Ok(sim::Source::Speed(v));
    }
    let fan = o.get("fan").ok_or_else(|| bad(cmd, format!("the inlet `{name}` needs a `speed` (m/s) or a `fan`")))?;
    let fan_err = |e: sim::SimError| bad(cmd, format!("`{name}`: {e}"));
    if let Some(points) = fan.get("curve").and_then(Value::as_array) {
        let mut pts = Vec::new();
        for p in points.iter().take(65) {
            let (Some(q), Some(pa)) = (p.get(0).and_then(Value::as_f64), p.get(1).and_then(Value::as_f64)) else {
                return Err(bad(cmd, format!("`{name}`: a fan curve is a list of [cfm, pa] points")));
            };
            pts.push((q / CFM, pa));
        }
        return sim::FanCurve::new(pts).map(sim::Source::Fan).map_err(fan_err);
    }
    let free = num(fan, "free_flow_cfm")
        .ok_or_else(|| bad(cmd, format!("`{name}`: a fan needs `free_flow_cfm` and `max_pressure_pa`, or a `curve`")))?
        / CFM;
    let pmax = num(fan, "max_pressure_pa").ok_or_else(|| bad(cmd, format!("`{name}`: a fan needs `max_pressure_pa`")))?;
    let curve = match str_(fan, "type").unwrap_or("axial") {
        "axial" => sim::FanCurve::axial(free, pmax),
        "radial" | "blower" => sim::FanCurve::radial(free, pmax),
        "linear" => sim::FanCurve::linear(free, pmax),
        other => return Err(bad(cmd, format!("`{name}`: fan type `{other}`: use axial, radial or linear"))),
    };
    curve.map(sim::Source::Fan).map_err(fan_err)
}

fn openings(s: &Session, cmd: &str, list: &Value, body: &str, inlet: bool) -> Result<Vec<OpeningSpec>> {
    let what = if inlet { "inlets" } else { "outlets" };
    let a = list.as_array().ok_or_else(|| bad(cmd, format!("`{what}` is a list of openings. {OPENING_DOC}")))?;
    if a.len() > 64 {
        return Err(bad(cmd, format!("at most 64 {what}")));
    }
    let mut out = Vec::new();
    for (i, o) in a.iter().enumerate() {
        let name = str_(o, "name").map(str::to_string).unwrap_or_else(|| format!("{} {}", if inlet { "Inlet" } else { "Outlet" }, i + 1));
        if name.is_empty() || name.len() > 128 {
            return Err(bad(cmd, "an opening's name is 1…128 characters"));
        }
        let on = str_(o, "body").unwrap_or(body);
        if s.world_state().body(on).is_none() {
            return Err(bad(cmd, format!("`{name}`: no body `{on}`")));
        }
        let face = o.get("face").ok_or_else(|| bad(cmd, format!("`{name}` needs a `face`. {OPENING_DOC}")))?;
        let point = super::face::face_ref(s, face, Some(on), cmd)?;
        let region = match str_(o, "region") {
            None => None,
            Some("hole") => Some(Region::Hole),
            Some("face") => Some(Region::Face),
            Some(other) => return Err(bad(cmd, format!("`{name}`: region `{other}`: use hole or face"))),
        };
        let source = if inlet { Some(source(cmd, o, &name)?) } else { None };
        out.push(OpeningSpec { name, body: on.to_string(), point, region, source });
    }
    Ok(out)
}

/// The spec from `p`, starting from `base` (an existing study's) where given.
fn spec(s: &Session, p: &Value, cmd: &str, base: Option<Spec>) -> Result<Spec> {
    let body = match (&base, p.get("body")) {
        (Some(b), None) => b.body.clone(),
        _ => body_param(s, p, cmd)?.to_string(),
    };
    let inlets = match (p.get("inlets"), &base) {
        (Some(l), _) => openings(s, cmd, l, &body, true)?,
        (None, Some(b)) => b.inlets.clone(),
        (None, None) => return Err(bad(cmd, format!("`inlets` is required: where the air comes in. {OPENING_DOC}"))),
    };
    if inlets.is_empty() {
        return Err(bad(cmd, "a study needs at least one inlet"));
    }
    let outlets = match (p.get("outlets"), &base) {
        (Some(l), _) => openings(s, cmd, l, &body, false)?,
        (None, Some(b)) => b.outlets.clone(),
        (None, None) => Vec::new(),
    };
    let st = s.world_state();
    let bodies = match (p.get("bodies"), &base) {
        (Some(v), _) => {
            let a = v.as_array().ok_or_else(|| bad(cmd, "`bodies` is a list of body names"))?;
            let mut names = Vec::new();
            for b in a {
                let n = b.as_str().ok_or_else(|| bad(cmd, "`bodies` is a list of body names"))?;
                if st.body(n).is_none() {
                    return Err(bad(cmd, format!("no body `{n}`")));
                }
                names.push(n.to_string());
            }
            names
        }
        (None, Some(b)) if p.get("body").is_none() => b.bodies.clone(),
        _ => st.bodies.iter().map(|b| b.name.clone()).filter(|n| !s.visibility.hidden_bodies.contains(n)).collect(),
    };
    let mut bodies = bodies;
    for b in std::iter::once(&body).chain(inlets.iter().map(|o| &o.body)).chain(outlets.iter().map(|o| &o.body)) {
        if !bodies.contains(b) {
            bodies.push(b.clone());
        }
    }
    let cell = match p.get("cell") {
        Some(Value::Null) | None => base.as_ref().and_then(|b| b.cell),
        Some(v) => {
            Some(v.as_f64().filter(|c| c.is_finite() && *c >= 0.05 && *c <= 50.0).ok_or_else(|| bad(cmd, "`cell` is the cell size in mm, 0.05…50"))?)
        }
    };
    let pad = match num(p, "pad") {
        Some(v) if v.is_finite() && (0.0..=1000.0).contains(&v) => v,
        Some(_) => return Err(bad(cmd, "`pad` is the open air around the bodies in mm, 0…1000")),
        None => base.as_ref().map_or(20.0, |b| b.pad),
    };
    let turbulence = bool_(p, "turbulence").or(base.as_ref().map(|b| b.turbulence)).unwrap_or(true);
    let max_steps = match num(p, "max_steps") {
        Some(v) if v.is_finite() && (1000.0..=10_000_000.0).contains(&v) => v as u64,
        Some(_) => return Err(bad(cmd, "`max_steps` is 1000…10000000")),
        None => base.as_ref().map_or(200_000, |b| b.max_steps),
    };
    Ok(Spec { body, bodies, inlets, outlets, cell, pad, turbulence, max_steps })
}

fn max_cells(s: &Session) -> u64 {
    if s.flow.backend.is_some() { flow::MAX_CELLS } else { flow::CPU_MAX_CELLS }
}

/// Prepare `spec` on the design as it is now: the check `sim.flow.new` makes, and the lattice a
/// run starts from.
fn prepare(s: &Session, spec: &Spec, cmd: &str) -> Result<(sim::Prepared, sim::FlowSetup)> {
    let st = s.world_state();
    let (setup, meshes) = flow::setup(&st, spec, max_cells(s)).map_err(|e| bad(cmd, e))?;
    let refs: Vec<&solidcraft_geom::Mesh> = meshes.iter().map(|m| m.as_ref()).collect();
    let prepared = sim::prepare(&refs, &setup).map_err(|e| bad(cmd, e.to_string()))?;
    Ok((prepared, setup))
}

fn summary(name: &str, prepared: &sim::Prepared, setup: &sim::FlowSetup) -> Value {
    json!({
        "study": name,
        "cell_mm": setup.cell,
        "grid": prepared.grid.dims,
        "cells": prepared.grid.len(),
        "inlets": prepared.inlets.iter().map(|l| json!({"name": l.name, "cells": l.cells.len(), "area_mm2": l.cells.len() as f64 * setup.cell * setup.cell})).collect::<Vec<_>>(),
        "outlets": prepared.outlets.iter().map(|l| json!({"name": l.name, "cells": l.cells.len()})).collect::<Vec<_>>(),
        "warnings": prepared.warnings.iter().map(flow::warning_text).collect::<Vec<_>>(),
    })
}

fn flow_new(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.new";
    let spec = spec(s, p, cmd, None)?;
    let name = match str_(p, "name") {
        Some(n) if !n.is_empty() && n.len() <= 128 => n.to_string(),
        Some(_) => return Err(bad(cmd, "a study's name is 1…128 characters")),
        None => (1..).map(|i| format!("Airflow {i}")).find(|n| s.flow.get(n).is_none()).unwrap_or_else(|| "Airflow".into()),
    };
    let (prepared, setup) = prepare(s, &spec, cmd)?;
    let out = summary(&name, &prepared, &setup);
    s.flow.list.retain(|st| st.name != name);
    s.flow.list.push(flow::Study { name, spec, job: None, result: None, revision: 0 });
    Ok(out)
}

fn study_name<'a>(s: &Session, p: &'a Value, cmd: &str) -> Result<&'a str> {
    let name = str_(p, "study").ok_or_else(|| bad(cmd, "`study` is required: the study's name (sim.flow.list)"))?;
    if s.flow.get(name).is_none() {
        return Err(bad(cmd, format!("no study `{name}`")));
    }
    Ok(name)
}

fn flow_edit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.edit";
    let name = study_name(s, p, cmd)?.to_string();
    let base = s.flow.get(&name).map(|st| st.spec.clone());
    let spec = spec(s, p, cmd, base)?;
    let (prepared, setup) = prepare(s, &spec, cmd)?;
    let out = summary(&name, &prepared, &setup);
    if let Some(st) = s.flow.get_mut(&name) {
        st.spec = spec;
        st.job = None;
        st.result = None;
        st.revision += 1;
    }
    Ok(out)
}

fn flow_run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.run";
    let name = study_name(s, p, cmd)?.to_string();
    s.poll_flow();
    if s.flow.get(&name).and_then(|st| st.job.as_ref()).is_some_and(|j| j.progress.lock().is_ok_and(|p| p.done.is_none())) {
        return Err(bad(cmd, format!("`{name}` is running; sim.flow.cancel stops it")));
    }
    let Some(spec) = s.flow.get(&name).map(|st| st.spec.clone()) else { return Err(bad(cmd, format!("no study `{name}`"))) };
    let tolerance = match num(p, "tolerance") {
        Some(t) if t.is_finite() && (1e-5..=0.2).contains(&t) => t,
        Some(_) => return Err(bad(cmd, "`tolerance` is a fraction, 0.00001…0.2")),
        None => 0.005,
    };
    let max_steps = match num(p, "max_steps") {
        Some(v) if v.is_finite() && (100.0..=10_000_000.0).contains(&v) => v as u64,
        Some(_) => return Err(bad(cmd, "`max_steps` is 100…10000000")),
        None => spec.max_steps,
    };
    let (prepared, _) = prepare(s, &spec, cmd)?;
    // Measure about sixteen times per crossing of the box, at least every 50 steps.
    let longest = prepared.grid.dims.iter().copied().max().unwrap_or(1) as f64;
    let check_every = ((longest / sim::study::LATTICE_SPEED) / 16.0).clamp(50.0, 2000.0) as u32;
    let limits = sim::Limits { max_steps, check_every, tolerance };
    let backend: flow::FlowBackend = s.flow.backend.clone().unwrap_or_else(|| std::sync::Arc::new(sim::Cpu));
    let names = (spec.inlets.iter().map(|o| o.name.clone()).collect(), spec.outlets.iter().map(|o| o.name.clone()).collect());
    let wait = bool_(p, "wait").unwrap_or(false);
    let cells = prepared.grid.len();
    let job = flow::start(prepared, backend, limits, names, wait);
    if let Some(st) = s.flow.get_mut(&name) {
        st.job = Some(job);
    }
    if wait {
        s.poll_flow();
        let st = s.flow.get(&name).ok_or_else(|| bad(cmd, "the study went away"))?;
        if let Some(r) = &st.result {
            return Ok(flow::results(&name, r));
        }
        let status = flow::status(st);
        return Err(EngineError::Other(status.get("error").and_then(Value::as_str).unwrap_or("the run did not finish").to_string()));
    }
    Ok(json!({"study": name, "state": "running", "cells": cells, "check_every": check_every}))
}

fn flow_status(s: &mut Session, p: &Value) -> Result<Value> {
    s.poll_flow();
    if str_(p, "study").is_some() {
        let name = study_name(s, p, "sim.flow.status")?;
        return s.flow.get(name).map(flow::status).ok_or_else(|| bad("sim.flow.status", "no such study"));
    }
    Ok(json!({"studies": s.flow.list.iter().map(flow::status).collect::<Vec<_>>()}))
}

fn flow_results(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.results";
    s.poll_flow();
    let name = study_name(s, p, cmd)?;
    let st = s.flow.get(name).ok_or_else(|| bad(cmd, "no such study"))?;
    let r = st.result.as_ref().ok_or_else(|| {
        bad(cmd, format!("`{name}` has no results yet ({})", flow::status(st).get("state").and_then(Value::as_str).unwrap_or("ready")))
    })?;
    Ok(flow::results(name, r))
}

fn flow_probe(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.probe";
    s.poll_flow();
    let name = study_name(s, p, cmd)?;
    let at = crate::params::req_vec3(cmd, p, "point")?;
    let r = s.flow.get(name).and_then(|st| st.result.clone()).ok_or_else(|| bad(cmd, format!("`{name}` has no results yet")))?;
    Ok(match r.fields.sample(at) {
        Some((u, pressure)) => json!({"velocity_ms": [u.x, u.y, u.z], "speed_ms": u.len(), "pressure_pa": pressure}),
        None => json!(null),
    })
}

fn flow_cancel(s: &mut Session, p: &Value) -> Result<Value> {
    let name = study_name(s, p, "sim.flow.cancel")?.to_string();
    if let Some(job) = s.flow.get(&name).and_then(|st| st.job.as_ref()) {
        job.cancel();
    }
    Ok(json!({"study": name}))
}

fn flow_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = study_name(s, p, "sim.flow.delete")?.to_string();
    s.flow.list.retain(|st| st.name != name);
    Ok(json!({"deleted": name}))
}

fn opening_json(o: &OpeningSpec) -> Value {
    let mut v = json!({"name": o.name, "body": o.body, "face": [o.point.x, o.point.y, o.point.z]});
    if let Some(r) = o.region {
        v["region"] = json!(if r == Region::Hole { "hole" } else { "face" });
    }
    match &o.source {
        Some(sim::Source::Speed(u)) => v["speed"] = json!(u),
        Some(sim::Source::Fan(c)) => v["fan"] = json!({"curve": c.points().iter().map(|(q, pa)| [q * CFM, *pa]).collect::<Vec<_>>()}),
        None => {}
    }
    v
}

fn flow_list(s: &mut Session, _: &Value) -> Result<Value> {
    s.poll_flow();
    let studies: Vec<Value> = s
        .flow
        .list
        .iter()
        .map(|st| {
            json!({
                "study": st.name,
                "body": st.spec.body,
                "bodies": st.spec.bodies,
                "inlets": st.spec.inlets.iter().map(opening_json).collect::<Vec<_>>(),
                "outlets": st.spec.outlets.iter().map(opening_json).collect::<Vec<_>>(),
                "cell": st.spec.cell,
                "pad": st.spec.pad,
                "turbulence": st.spec.turbulence,
                "max_steps": st.spec.max_steps,
                "state": flow::status(st).get("state").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({"studies": studies}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn studies_are_registered_on_the_simulation_tab() {
        for (id, label) in [("sim.flow.new", "Airflow Study"), ("sim.aero.new", "Aerodynamics Study"), ("sim.gas.new", "Gas Dynamics Study")] {
            let spec = crate::find_command(id).unwrap();
            assert_eq!((spec.label, spec.tab, spec.panel), (label, "SIMULATION", "STUDY"));
            assert!(!spec.undoable, "{id}");
            assert!(spec.planned.is_none(), "{id} opens its dialog, so it is not a greyed planned command");
        }
    }

    #[test]
    fn the_other_studies_check_their_body_then_say_the_solver_is_not_built() {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10, "body_name": "Duct"})).unwrap();
        let before = serde_json::to_string(&*s.doc).unwrap();
        let undo = s.undo.len();
        for id in ["sim.aero.new", "sim.gas.new"] {
            let missing = s.execute(id, &json!({})).unwrap_err().to_string();
            assert!(missing.contains("`body` is required"), "{id}: {missing}");
            let wrong = s.execute(id, &json!({"body": "Nope"})).unwrap_err().to_string();
            assert!(wrong.contains("no body `Nope`"), "{id}: {wrong}");
            let refused = s.execute(id, &json!({"body": "Duct"})).unwrap_err().to_string();
            assert!(refused.contains("not supported yet"), "{id}: {refused}");
        }
        let flow = s.execute("sim.flow.new", &json!({"body": "Duct"})).unwrap_err().to_string();
        assert!(flow.contains("`inlets` is required"), "{flow}");
        assert_eq!(serde_json::to_string(&*s.doc).unwrap(), before, "a refused study leaves the design alone");
        assert_eq!(s.undo.len(), undo);
    }

    /// A square duct (8 mm bore, 2 mm walls, 24 mm long along x) cut from a box.
    fn duct(s: &mut Session) {
        s.execute("solid.box", &json!({"length": 24, "width": 12, "height": 12, "body_name": "Duct"})).unwrap();
        s.execute("solid.box", &json!({"corner": [-1, 2, 2], "length": 26, "width": 8, "height": 8, "operation": "cut", "targets": ["Duct"]}))
            .unwrap();
    }

    /// The whole path headless: a duct cut in the modeller, its open ends picked by their rims, run
    /// on the CPU to a settled flow; what goes in comes out, and nothing in the design changed.
    #[test]
    fn an_airflow_study_runs_through_a_modelled_duct() {
        let mut s = Session::default();
        duct(&mut s);
        let before = serde_json::to_string(&*s.doc).unwrap();
        let set = s
            .execute(
                "sim.flow.new",
                &json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": 0.05, "name": "Fan"}], "outlets": [{"face": [24, 1, 6], "name": "Nozzle"}], "cell": 1, "pad": 4, "turbulence": false}),
            )
            .unwrap();
        assert_eq!(set["study"], "Airflow 1");
        assert_eq!(set["inlets"][0]["cells"], 64, "{set}");
        assert!(set["warnings"].as_array().unwrap().is_empty(), "{set}");
        let r = s.execute("sim.flow.run", &json!({"study": "Airflow 1", "wait": true})).unwrap();
        assert_eq!(r["state"], "settled", "{r}");
        let (q_in, q_out) = (r["inlets"][0]["flow_m3s"].as_f64().unwrap(), r["outlets"][0]["flow_m3s"].as_f64().unwrap());
        assert!((q_in - 0.05 * 64e-6).abs() / (0.05 * 64e-6) < 0.08, "{r}");
        assert!((q_out - q_in).abs() / q_in < 0.03, "{r}");
        assert!(r["pressure_drop_pa"].as_f64().unwrap() > 0.0, "{r}");
        assert_eq!(r["outlet_shares"][0]["share"], 1.0);
        let probe = s.execute("sim.flow.probe", &json!({"study": "Airflow 1", "point": [12, 6, 6]})).unwrap();
        assert!(probe["velocity_ms"][0].as_f64().unwrap() > 0.05, "the centre runs faster than the mean: {probe}");
        assert!(s.execute("sim.flow.probe", &json!({"study": "Airflow 1", "point": [12, 0.5, 6]})).unwrap().is_null(), "inside the wall");
        assert_eq!(s.execute("sim.flow.status", &json!({"study": "Airflow 1"})).unwrap()["state"], "done");
        assert_eq!(serde_json::to_string(&*s.doc).unwrap(), before, "studies never touch the design");
        let listed = s.execute("sim.flow.list", &json!({})).unwrap();
        assert_eq!(listed["studies"][0]["inlets"][0]["speed"], 0.05);
        // Editing clears the results; deleting removes the study.
        s.execute("sim.flow.edit", &json!({"study": "Airflow 1", "inlets": [{"face": [0, 1, 6], "fan": {"type": "radial", "free_flow_cfm": 0.01, "max_pressure_pa": 0.05}}]})).unwrap();
        assert!(s.execute("sim.flow.results", &json!({"study": "Airflow 1"})).is_err());
        s.execute("sim.flow.delete", &json!({"study": "Airflow 1"})).unwrap();
        assert!(s.execute("sim.flow.list", &json!({})).unwrap()["studies"].as_array().unwrap().is_empty());
    }

    #[test]
    fn hostile_study_requests_are_errors() {
        let mut s = Session::default();
        duct(&mut s);
        let inlet = json!([{"face": [0, 1, 6], "speed": 0.05}]);
        for p in [
            json!({"body": "Duct", "inlets": []}),
            json!({"body": "Duct", "inlets": "x"}),
            json!({"body": "Duct", "inlets": [{"speed": 1}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6]}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": -1}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": 1e9}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "fan": {"type": "jet"}}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "fan": {"curve": [[1, 0], [0, 5]]}}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": 1, "region": "middle"}]}),
            json!({"body": "Duct", "inlets": [{"face": [6, 1, 6], "speed": 1, "region": "hole"}]}),
            json!({"body": "Duct", "inlets": [{"face": [500, 500, 500], "speed": 1}]}),
            json!({"body": "Duct", "inlets": inlet, "cell": 0}),
            json!({"body": "Duct", "inlets": inlet, "cell": f64::MAX}),
            json!({"body": "Duct", "inlets": inlet, "cell": 0.05}),
            json!({"body": "Duct", "inlets": inlet, "pad": -1}),
            json!({"body": "Duct", "inlets": inlet, "bodies": ["Nope"]}),
        ] {
            assert!(s.execute("sim.flow.new", &p).is_err(), "{p}");
        }
        for (id, p) in [
            ("sim.flow.run", json!({"study": "Nope"})),
            ("sim.flow.results", json!({})),
            ("sim.flow.probe", json!({"study": "Nope", "point": [0, 0, 0]})),
        ] {
            assert!(s.execute(id, &p).is_err(), "{id} {p}");
        }
        assert!(s.flow.list.is_empty());
    }
}
