//! Simulation studies: Airflow (steady, low-speed flow through ducts, fans and cooling paths),
//! Aerodynamics (external flow around a body: drag and lift forces) and Gas Dynamics (transient
//! compressible flow with shocks: pressure pulses, blast, nozzles).
//! They are registered so the toolbar, palette, scripts, CLI and MCP all reach them, and they
//! check their inputs; the solvers are not built yet, so a valid request is refused with
//! "not supported yet". Nothing is saved in the design.

use serde_json::Value;

use super::CommandSpec;
use crate::params::bad;
use crate::{EngineError, Result, Session};

pub static COMMANDS: &[CommandSpec] = &[
CommandSpec::new("sim.flow.new", "Airflow Study", flow_new).noundo().at("SIMULATION", "STUDY").icon("sim_flow")
    .params("body: the solid body the air flows through or around. Steady, low-speed flow (ducts, fans, cooling). Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
CommandSpec::new("sim.aero.new", "Aerodynamics Study", aero_new).noundo().at("SIMULATION", "STUDY").icon("sim_aero")
    .params("body: the solid body the air flows around. External flow at a wind speed, reporting drag and lift forces. Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
CommandSpec::new("sim.gas.new", "Gas Dynamics Study", gas_new).noundo().at("SIMULATION", "STUDY").icon("sim_gas")
    .params("body: the solid body the gas flows through or around. Transient compressible flow with shocks (pressure pulses, blast, nozzles). Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
];

fn flow_new(s: &mut Session, params: &Value) -> Result<Value> {
    new_study(s, params, "sim.flow.new", "airflow")
}

fn aero_new(s: &mut Session, params: &Value) -> Result<Value> {
    new_study(s, params, "sim.aero.new", "aerodynamics")
}

fn gas_new(s: &mut Session, params: &Value) -> Result<Value> {
    new_study(s, params, "sim.gas.new", "gas dynamics")
}

/// Check the request as the finished study will, then refuse it: there is no solver to run.
fn new_study(s: &mut Session, params: &Value, cmd: &str, solver: &str) -> Result<Value> {
    let body = params.get("body").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`body` is required: the name of the solid body to study"))?;
    if s.world_state().body(body).is_none() {
        return Err(bad(cmd, format!("no body `{body}`")));
    }
    Err(EngineError::Other(format!("not supported yet: the {solver} solver is not built")))
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
    fn a_study_checks_its_body_then_says_the_solver_is_not_built() {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10, "body_name": "Duct"})).unwrap();
        let before = serde_json::to_string(&*s.doc).unwrap();
        let undo = s.undo.len();
        for id in ["sim.flow.new", "sim.aero.new", "sim.gas.new"] {
            let missing = s.execute(id, &json!({})).unwrap_err().to_string();
            assert!(missing.contains("`body` is required"), "{id}: {missing}");
            let wrong = s.execute(id, &json!({"body": "Nope"})).unwrap_err().to_string();
            assert!(wrong.contains("no body `Nope`"), "{id}: {wrong}");
            let refused = s.execute(id, &json!({"body": "Duct"})).unwrap_err().to_string();
            assert!(refused.contains("not supported yet"), "{id}: {refused}");
        }
        assert_eq!(serde_json::to_string(&*s.doc).unwrap(), before, "a refused study leaves the design alone");
        assert_eq!(s.undo.len(), undo);
    }
}
