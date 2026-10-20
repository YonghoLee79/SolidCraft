//! The study setup dialogs of the Simulation workspace: Airflow Study (`sim.flow.new`),
//! Aerodynamics Study (`sim.aero.new`) and Gas Dynamics Study (`sim.gas.new`).
//!
//! Airflow picks the duct's body, its inlets and its outlets (faces: a duct's rim face stands for
//! its hole, a fan's outlet face for itself), then what drives the air (a speed, or a fan on its
//! curve) and the grid. OK sets the study up and starts it running (`sim.flow.run`); the results
//! panel (`simulation_workspace`) follows it. Aerodynamics and Gas Dynamics pick the body only:
//! their solvers are not built yet, and OK reports so.

use egui::RichText;
use serde_json::{Value, json};
use solidcraft_engine::Sel;

use crate::SolveApp;
use crate::dialogs::{Kind, row_label};
use crate::selection::{BODIES, FACES, SelInput};
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

/// What drives the air in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    Speed,
    Fan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FanType {
    Axial,
    Radial,
    Linear,
}

impl FanType {
    fn id(self) -> &'static str {
        match self {
            FanType::Axial => "axial",
            FanType::Radial => "radial",
            FanType::Linear => "linear",
        }
    }

    fn label(self) -> &'static str {
        match self {
            FanType::Axial => "sim.fan_axial",
            FanType::Radial => "sim.fan_radial",
            FanType::Linear => "sim.fan_linear",
        }
    }
}

/// The Airflow dialog's values, as typed.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowForm {
    pub drive: Drive,
    /// m/s.
    pub speed: String,
    pub fan: FanType,
    /// CFM.
    pub free_flow: String,
    /// Pa.
    pub max_pressure: String,
    /// mm; empty: chosen from the openings.
    pub cell: String,
    /// mm.
    pub pad: String,
    pub turbulence: bool,
}

impl Default for FlowForm {
    fn default() -> Self {
        // A 40 mm part-cooling fan, as on a printer's toolhead.
        FlowForm {
            drive: Drive::Fan,
            speed: "5".into(),
            fan: FanType::Axial,
            free_flow: "15".into(),
            max_pressure: "314".into(),
            cell: String::new(),
            pad: "20".into(),
            turbulence: true,
        }
    }
}

/// A study dialog's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Sim {
    pub study: Study,
    pub flow: FlowForm,
}

impl Sim {
    pub fn title(&self) -> &'static str {
        self.study.title()
    }
}

pub fn start(_app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    let study = match id {
        "sim.flow.new" => Study::Flow,
        "sim.aero.new" => Study::Aero,
        "sim.gas.new" => Study::Gas,
        _ => return None,
    };
    let mut inputs = vec![SelInput::new("Body", BODIES, false)];
    if study == Study::Flow {
        inputs.push(SelInput::new("Inlets", FACES, true));
        inputs.push(SelInput::new("Outlets", FACES, true));
    }
    Some((Kind::Sim(Box::new(Sim { study, flow: FlowForm::default() })), inputs))
}

/// A labelled value with its unit; `label` is English (the row translates it).
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, unit: &str, hint: &str) -> bool {
    row_label(ui, label);
    let enter = ui
        .horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(value).desired_width(72.0).hint_text(hint));
            ui.label(unit);
            r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
        })
        .inner;
    ui.end_row();
    enter
}

/// The dialog's rows after the pickers; true when Enter was pressed in a value.
pub fn rows(app: &SolveApp, ui: &mut egui::Ui, k: &mut Sim) -> bool {
    let t = Tokens::get();
    let tr = crate::i18n::tr;
    row_label(ui, "");
    ui.label(RichText::new(tr(k.study.summary())).color(t.text_dim));
    ui.end_row();
    if k.study != Study::Flow {
        row_label(ui, "");
        ui.label(RichText::new(tr("sim.solver_not_built")).color(t.warning));
        ui.end_row();
        return false;
    }
    let f = &mut k.flow;
    let mut enter = false;
    row_label(ui, "Air from");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut f.drive, Drive::Fan, tr("sim.drive_fan"));
        ui.selectable_value(&mut f.drive, Drive::Speed, tr("sim.drive_speed"));
    });
    ui.end_row();
    match f.drive {
        Drive::Speed => enter |= field(ui, "Speed", &mut f.speed, "m/s", ""),
        Drive::Fan => {
            row_label(ui, "Fan type");
            egui::ComboBox::from_id_salt("sim_fan_type").selected_text(tr(f.fan.label())).show_ui(ui, |ui| {
                for ty in [FanType::Axial, FanType::Radial, FanType::Linear] {
                    ui.selectable_value(&mut f.fan, ty, tr(ty.label()));
                }
            });
            ui.end_row();
            enter |= field(ui, "Free-air flow", &mut f.free_flow, "CFM", "");
            enter |= field(ui, "Max pressure", &mut f.max_pressure, "Pa", "");
        }
    }
    enter |= field(ui, "Cell size", &mut f.cell, "mm", tr("sim.auto"));
    enter |= field(ui, "Open air", &mut f.pad, "mm", "");
    row_label(ui, "");
    ui.checkbox(&mut f.turbulence, tr("sim.turbulence"));
    ui.end_row();
    row_label(ui, "");
    ui.label(RichText::new(crate::i18n::format("sim.runs_on", &[("solver", &app.session.flow_backend_name())])).color(t.text_dim).small());
    ui.end_row();
    enter
}

fn number(text: &str, what: &str) -> Result<f64, String> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("{what}: enter a number"))
}

fn openings(input: Option<&SelInput>) -> Vec<Value> {
    input
        .map(|i| {
            i.items
                .iter()
                .filter_map(|s| match s {
                    Sel::Face { body, point, .. } => Some(json!({"face": [point.x, point.y, point.z], "body": body})),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The commands OK runs.
pub fn commands(app: &SolveApp, k: &Sim, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let body = match inputs.first().and_then(|i| i.items.first()) {
        Some(Sel::Body { name }) => name.clone(),
        _ => return Err("select the body to study".into()),
    };
    if k.study != Study::Flow {
        return Ok(vec![(k.study.command().to_string(), json!({"body": body}))]);
    }
    let f = &k.flow;
    let mut inlets = openings(inputs.get(1));
    if inlets.is_empty() {
        return Err("pick the inlet faces: a duct's rim, or a fan's outlet face".into());
    }
    let drive = match f.drive {
        Drive::Speed => json!({"speed": number(&f.speed, "speed")?}),
        Drive::Fan => {
            json!({"fan": {"type": f.fan.id(), "free_flow_cfm": number(&f.free_flow, "free-air flow")?, "max_pressure_pa": number(&f.max_pressure, "maximum pressure")?}})
        }
    };
    for (i, o) in inlets.iter_mut().enumerate() {
        if let (Some(o), Some(d)) = (o.as_object_mut(), drive.as_object()) {
            o.extend(d.clone());
            o.insert("name".into(), json!(format!("Inlet {}", i + 1)));
        }
    }
    let mut outlets = openings(inputs.get(2));
    for (i, o) in outlets.iter_mut().enumerate() {
        if let Some(o) = o.as_object_mut() {
            o.insert("name".into(), json!(format!("Outlet {}", i + 1)));
        }
    }
    let name = (1..).map(|i| format!("Airflow {i}")).find(|n| app.session.flow_studies().get(n).is_none()).unwrap_or_else(|| "Airflow".into());
    let mut params =
        json!({"body": body, "name": name, "inlets": inlets, "outlets": outlets, "pad": number(&f.pad, "open air")?, "turbulence": f.turbulence});
    if !f.cell.trim().is_empty() {
        params["cell"] = json!(number(&f.cell, "cell size")?);
    }
    Ok(vec![("sim.flow.new".to_string(), params), ("sim.flow.run".to_string(), json!({"study": name}))])
}
