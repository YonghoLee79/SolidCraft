//! Metric thread choices use the document's sizing table and the existing designation
//! parameter. Browsing choices does not edit the model or freeze an automatic feature.

use solidcraft_engine::doc::{metric_thread_sizes, parse_metric_thread};

use crate::dialogs::{combo, form_row};
use crate::i18n::tr;
use crate::ui_kit::text_row;

/// Common pitch presets, not an exhaustive catalogue of standard diameter/pitch pairs.
/// The coarse and current pitches are included separately; custom pairs remain editable.
const PITCHES: &[f64] = &[0.2, 0.25, 0.35, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

/// Tapped holes use the existing metric catalogue; the engine derives the tap drill.
/// Explicit designations also allow pitches outside the coarse-size presets.
pub(crate) fn hole_rows(ui: &mut egui::Ui, designation: &mut String) -> bool {
    row_label(ui, "Profile");
    ui.label(tr("ISO metric"));
    ui.end_row();

    row_label(ui, "Size");
    let sizes = metric_thread_sizes();
    let current = parse_metric_thread(designation);
    let mut selected = current.and_then(|pair| sizes.iter().position(|s| *s == pair)).unwrap_or(sizes.len());
    let before = selected;
    let mut labels: Vec<String> = sizes.iter().map(|(d, p)| format!("M{d} × {p}")).collect();
    labels.push(tr("Custom").to_string());
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    combo(ui, "hole_thread_catalogue", &labels, &mut selected);
    if selected != before
        && let Some((d, p)) = sizes.get(selected)
    {
        *designation = format!("M{d}x{p}");
    }
    ui.end_row();

    row_label(ui, "Designation");
    let r = ui.add(egui::TextEdit::singleline(designation).hint_text("M5 or M5x0.5"));
    ui.end_row();
    row_label(ui, "Tap drill diameter");
    match parse_metric_thread(designation) {
        Some((d, p)) => {
            ui.label(format!("{:.3} mm", d - p));
        }
        None => {
            ui.label(tr("Select size"));
        }
    }
    ui.end_row();
    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Returns (Enter pressed, automatic sizing requested).
pub(crate) fn rows(ui: &mut egui::Ui, designation: &mut String, suggested: &mut Option<String>, modeled: &mut bool, full: &mut bool) -> (bool, bool) {
    form_row(ui, "Profile", |ui| {
        ui.label(tr("ISO metric"));
    });

    let before = designation.trim().is_empty() || suggested.as_deref() == Some(designation.as_str());
    let mut automatic = before;
    crate::ui_kit::section(ui, "thread_options", "Options", true, |ui| {
        crate::ui_kit::checkbox_grid(ui, &mut [("Modeled", modeled), ("Full Length", full)], 2);
        ui.add_enabled_ui(!designation.trim().is_empty(), |ui| crate::ui_kit::checkbox_grid(ui, &mut [("Automatic size", &mut automatic)], 2));
    });
    let reset = automatic && !before;
    if automatic != before {
        *suggested = None;
        if automatic {
            designation.clear();
        }
    }

    let current = parse_metric_thread(designation);
    let mut diameters: Vec<f64> = metric_thread_sizes().iter().map(|(d, _)| *d).collect();
    if let Some((d, _)) = current
        && !diameters.contains(&d)
    {
        diameters.push(d);
        diameters.sort_by(f64::total_cmp);
    }
    form_row(ui, "Size", |ui| {
        let labels: Vec<String> = diameters.iter().map(|d| format!("{d} mm")).collect();
        let mut selected = current.and_then(|(d, _)| diameters.iter().position(|n| *n == d)).unwrap_or(diameters.len());
        let before = selected;
        let mut labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        if current.is_none() {
            labels.push("Select size");
        }
        combo(ui, "thread_diameter", &labels, &mut selected);
        // A new diameter starts with its coarse pitch. The current custom diameter, if any,
        // is already selected and retains its explicit pitch.
        if selected != before
            && let Some(d) = diameters.get(selected)
            && let Some((_, coarse)) = metric_thread_sizes().iter().find(|(n, _)| n == d)
        {
            *designation = format!("M{d}x{coarse}");
            *suggested = None;
        }
    });

    let current = parse_metric_thread(designation);
    form_row(ui, "Pitch", |ui| {
        ui.add_enabled_ui(current.is_some(), |ui| {
            let mut pitches = Vec::new();
            if let Some((d, p)) = current {
                let coarse = metric_thread_sizes().iter().find(|(n, _)| *n == d).map(|(_, p)| *p);
                pitches.extend(PITCHES.iter().copied().filter(|p| *p < d && *p <= coarse.unwrap_or(d)));
                pitches.extend(coarse);
                pitches.push(p);
            }
            pitches.sort_by(|a, b| b.total_cmp(a));
            pitches.dedup();
            let labels: Vec<String> = pitches.iter().map(|p| format!("{p} mm")).collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut selected = current.and_then(|(_, p)| pitches.iter().position(|n| *n == p)).unwrap_or(0);
            let before = selected;
            combo(ui, "thread_pitch", &labels, &mut selected);
            if selected != before
                && let (Some((d, _)), Some(p)) = (current, pitches.get(selected))
            {
                *designation = format!("M{d}x{p}");
                *suggested = None;
            }
        });
    });

    let (enter, changed) = text_row(ui, "Designation", designation, tr("fits the face, e.g. M8"));
    if let Some(at) = crate::scenario::handle_at("dialog:Designation") {
        crate::scenario::publish_handle("dialog:thread_size", at);
    }
    if changed {
        *suggested = None;
    }
    (enter, reset)
}
