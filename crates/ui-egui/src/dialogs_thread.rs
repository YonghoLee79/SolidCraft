//! Editable metric thread dimensions and optional presets write the existing designation
//! parameter. Browsing choices does not edit the model or freeze an automatic feature.

use solidcraft_engine::doc::{metric_thread_coarse_presets, parse_metric_thread};

use crate::dialogs::form_row;
use crate::i18n::tr;
use crate::ui_kit;

/// Common pitch presets, not an exhaustive catalogue of standard diameter/pitch pairs.
/// The coarse and current pitches are included separately; custom pairs remain editable.
const PITCHES: &[f64] =
    &[0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.6, 0.7, 0.75, 0.8, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0];

// Whole-millimetre convenience values avoid gaps left by a coarse-only catalogue.
// This is a browsable range, never a limit or a list of standard diameter/pitch pairs.
fn diameter_presets() -> Vec<f64> {
    let mut diameters: Vec<f64> = (1..=100).map(f64::from).chain(metric_thread_coarse_presets().map(|(d, _)| d)).collect();
    diameters.sort_by(f64::total_cmp);
    diameters.dedup();
    diameters
}

fn coarse_pitch(diameter: f64) -> Option<f64> {
    metric_thread_coarse_presets().find(|(d, _)| *d == diameter).map(|(_, p)| p)
}

/// Keep incomplete input while typing. The feature still stores only its designation;
/// suggestions, presets and direct designation edits refresh these fields.
#[derive(Clone, Debug, Default)]
pub struct Dimensions {
    source: String,
    diameter: String,
    pitch: String,
}

impl Dimensions {
    fn sync(&mut self, designation: &str) {
        if self.source == designation {
            return;
        }
        self.source = designation.into();
        (self.diameter, self.pitch) = parse_metric_thread(designation).map(|(d, p)| (format!("{d} mm"), format!("{p} mm"))).unwrap_or_default();
    }

    fn write(&mut self, designation: &mut String, suggested: &mut Option<String>) {
        let number = |text: &str| crate::prefs::parse_length_mm(text, 1.0).map(|n| n.to_string()).unwrap_or_else(|| text.trim().into());
        *designation = format!("M{}x{}", number(&self.diameter), number(&self.pitch));
        // Even incomplete or invalid input must be sent to validation, never replaced by
        // an automatic size or the last valid pair when the user presses OK.
        self.source = designation.clone();
        *suggested = None;
    }
}

/// A directly editable metric length with a separate menu of optional presets.
/// Returns (text changed, preset chosen, Enter pressed).
fn dimension(ui: &mut egui::Ui, label: &str, text: &mut String, presets: &[f64]) -> (bool, Option<f64>, bool) {
    form_row(ui, label, |ui| {
        let height = ui_kit::row_height();
        // Even an empty dropdown reserves its icon, icon gap and button padding.
        let menu_width = height.max(ui.spacing().icon_width + ui.spacing().icon_spacing + 2.0 * ui.spacing().button_padding.x);
        let width = (ui.available_width() - menu_width - ui.spacing().item_spacing.x).max(1.0);
        let (enter, changed) = ui.allocate_ui(egui::vec2(width, height), |ui| ui_kit::text_field(ui, text, "mm")).inner;
        let input_rect = crate::scenario::rect_of(&format!("dialog:widget:{label}"));
        let mut chosen = None;
        let was_open = egui::Popup::is_id_open(ui.ctx(), ui.make_persistent_id((label, "presets")).with("popup"));
        let menu = ui_kit::dropdown_menu(ui, (label, "presets"), "", |ui| {
            let current = crate::prefs::parse_length_mm(text, 1.0);
            for &value in presets {
                let label_value = format!("{value} mm");
                let key = format!("{label}:{label_value}");
                let item = ui_kit::list_picker(ui, &key, &label_value, current == Some(value));
                if !was_open && current == Some(value) {
                    item.scroll_to_me_animation(Some(egui::Align::Center), egui::style::ScrollAnimation::none());
                }
                crate::scenario::publish_handle(&format!("dialog:{key}"), item.rect.center());
                if item.clicked() {
                    chosen = Some(value);
                    ui.close();
                }
            }
        });
        crate::scenario::publish_target(&format!("dialog:{label} presets"), menu.rect);
        // The preset menu shares the row label; the editable field remains its primary target.
        if let Some(rect) = input_rect {
            crate::scenario::publish_target_as(&format!("dialog:{label}"), &format!("dialog:widget:{label}"), rect);
        }
        menu.on_hover_text(crate::i18n::tr_label("Preset"));
        (changed, chosen, enter)
    })
}

/// Returns (Enter pressed, automatic sizing requested).
pub(crate) fn rows(
    ui: &mut egui::Ui,
    designation: &mut String,
    suggested: &mut Option<String>,
    dimensions: &mut Dimensions,
    modeled: &mut bool,
    full: &mut bool,
) -> (bool, bool) {
    form_row(ui, "Profile", |ui| {
        ui.label(tr("ISO metric"));
    });

    let before = designation.trim().is_empty() || suggested.as_deref() == Some(designation.as_str());
    let mut automatic = before;
    ui_kit::section(ui, "thread_options", "Options", true, |ui| {
        ui_kit::checkbox_grid(ui, &mut [("Modeled", modeled), ("Full Length", full)], 2);
        ui.add_enabled_ui(!designation.trim().is_empty(), |ui| ui_kit::checkbox_grid(ui, &mut [("Automatic size", &mut automatic)], 2));
    });
    let reset = automatic && !before;
    if automatic != before {
        *suggested = None;
        if automatic {
            designation.clear();
        }
    }

    dimensions.sync(designation);
    let current = parse_metric_thread(designation);
    let mut diameters = diameter_presets();
    if let Some((d, _)) = current
        && !diameters.contains(&d)
    {
        diameters.push(d);
    }
    diameters.sort_by(f64::total_cmp);
    diameters.dedup();
    let (changed, selected, mut enter) = dimension(ui, "Size", &mut dimensions.diameter, &diameters);
    if let Some(d) = selected {
        dimensions.diameter = format!("{d} mm");
        // Start with coarse pitch where defined, otherwise a convenient fine pitch.
        // Fine pitch selection: ISO 261:1998, section 5.4; M44x1.5 is covered by DIN 13-6:1999-11.
        // The two menus are independent values, not a catalogue of standard pairs.
        // Typing a diameter keeps the authored pitch; neither field is limited to presets.
        if current.is_none_or(|(n, _)| n != d) {
            let pitch = coarse_pitch(d).unwrap_or(1.5_f64.min(d / 2.0));
            dimensions.pitch = format!("{pitch} mm");
        }
    }
    if changed || selected.is_some() {
        dimensions.write(designation, suggested);
    }

    let current = parse_metric_thread(designation);
    let diameter = crate::prefs::parse_length_mm(&dimensions.diameter, 1.0);
    let coarse = diameter.and_then(coarse_pitch);
    let mut pitches: Vec<f64> = PITCHES.iter().copied().filter(|p| diameter.is_none_or(|d| *p < d) && coarse.is_none_or(|c| *p <= c)).collect();
    pitches.extend(coarse);
    pitches.extend(current.map(|(_, p)| p));
    pitches.sort_by(|a, b| b.total_cmp(a));
    pitches.dedup();
    let (changed, selected, submitted) = dimension(ui, "Pitch", &mut dimensions.pitch, &pitches);
    enter |= submitted;
    if let Some(p) = selected {
        dimensions.pitch = format!("{p} mm");
    }
    if changed || selected.is_some() {
        dimensions.write(designation, suggested);
    }

    let (submitted, changed) = ui_kit::text_row(ui, "Designation", designation, tr("fits the face, e.g. M8"));
    if let Some(at) = crate::scenario::handle_at("dialog:Designation") {
        crate::scenario::publish_handle("dialog:thread_size", at);
    }
    if changed {
        *suggested = None;
    }
    enter |= submitted;
    (enter, reset)
}
