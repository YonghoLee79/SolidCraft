//! The study setup dialogs of the Simulation workspace: Airflow Study (`sim.flow.new`),
//! Aerodynamics Study (`sim.aero.new`) and Gas Dynamics Study (`sim.gas.new`). Each picks the body
//! the study runs on. The solvers are not built yet: the dialog says so, and OK runs the command,
//! which checks the body and refuses.

use egui::RichText;
use serde_json::{Value, json};
use solidcraft_engine::Sel;

use crate::SolveApp;
use crate::dialogs::{Kind, row_label};
use crate::selection::{BODIES, SelInput};
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Study {
    /// Steady, low-speed flow: ducts, fans, cooling.
    Flow,
    /// External flow around a body: drag and lift forces.
    Aero,
    /// Transient compressible flow with shocks: pressure pulses, blast, nozzles.
    Gas,
}

impl Study {
    pub fn command(self) -> &'static str {
        match self {
            Study::Flow => "sim.flow.new",
            Study::Aero => "sim.aero.new",
            Study::Gas => "sim.gas.new",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Study::Flow => "AIRFLOW STUDY",
            Study::Aero => "AERODYNAMICS STUDY",
            Study::Gas => "GAS DYNAMICS STUDY",
        }
    }

    /// The message id of the one-line summary of what the study is for.
    fn summary(self) -> &'static str {
        match self {
            Study::Flow => "sim.flow_summary",
            Study::Aero => "sim.aero_summary",
            Study::Gas => "sim.gas_summary",
        }
    }
}

pub fn start(_app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    let study = match id {
        "sim.flow.new" => Study::Flow,
        "sim.aero.new" => Study::Aero,
        "sim.gas.new" => Study::Gas,
        _ => return None,
    };
    Some((Kind::Sim(study), vec![SelInput::new("Body", BODIES, false)]))
}

/// The dialog's rows after the body picker; true when Enter was pressed in a value (there are none yet).
pub fn rows(ui: &mut egui::Ui, study: Study) -> bool {
    let t = Tokens::get();
    row_label(ui, "");
    ui.label(RichText::new(crate::i18n::tr(study.summary())).color(t.text_dim));
    ui.end_row();
    row_label(ui, "");
    ui.label(RichText::new(crate::i18n::tr("sim.solver_not_built")).color(t.warning));
    ui.end_row();
    false
}

/// The command OK runs.
pub fn commands(study: Study, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let body = match inputs.first().and_then(|i| i.items.first()) {
        Some(Sel::Body { name }) => name.clone(),
        _ => return Err("select the body to study".into()),
    };
    Ok(vec![(study.command().to_string(), json!({"body": body}))])
}
