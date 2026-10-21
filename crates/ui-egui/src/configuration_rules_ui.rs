//! Structured configuration rule editing and non-mutating row preview.
use crate::SolveApp;
use egui::{RichText, Ui};
use serde_json::{Value, json};
use solidcraft_engine::doc::config::{Column, ConfigRule, RuleCondition, RuleEffect, RuleOperator};

#[derive(Clone, Debug)]
pub struct Editor {
    rule: ConfigRule,
    row: String,
    preview: Option<Value>,

    preview_revision: u64,
    pub(super) drafts: std::collections::BTreeMap<(String, String), String>,
    pub(super) table: Option<(u64, Value)>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            rule: ConfigRule {
                id: String::new(),
                name: crate::i18n::tr("configuration.new_rule").into(),
                enabled: true,
                length_unit: None,
                condition: RuleCondition::Always,
                effect: RuleEffect::DriveParameter { parameter: String::new(), expression: "10 mm".into() },
            },
            row: String::new(),
            preview: None,

            preview_revision: 0,
            drafts: Default::default(),
            table: None,
        }
    }
}
fn choose(ui: &mut Ui, id: &str, value: &mut String, options: &[String]) {
    choose_impl(ui, id, value, options, false);
}
fn choose_labels(ui: &mut Ui, id: &str, value: &mut String, options: &[String]) {
    choose_impl(ui, id, value, options, true);
}
fn choose_impl(ui: &mut Ui, id: &str, value: &mut String, options: &[String], translate: bool) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(if value.is_empty() {
            crate::i18n::tr("ui.choose")
        } else if translate {
            crate::i18n::tr_label(value.as_str())
        } else {
            value.as_str()
        })
        .width(130.0)
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(value, option.clone(), if translate { crate::i18n::tr_label(option) } else { option.as_str() });
            }
        });
}
fn expression(ui: &mut Ui, label: &str, value: &mut String) {
    ui.label(crate::i18n::tr_label(label));
    ui.add(egui::TextEdit::singleline(value).desired_width(160.0));
}

pub(super) fn show(ui: &mut Ui, app: &mut SolveApp, error: &mut Option<String>, editor: &mut Editor) {
    if editor.preview_revision != app.session.revision {
        editor.preview = None;
    }
    let rules = app.session.doc.configs.rules.clone();
    let parameters: Vec<String> =
        app.session.doc.configs.columns.iter().filter_map(|c| if let Column::Param(n) = c { Some(n.clone()) } else { None }).collect();
    let all_parameters: Vec<String> = app.session.doc.all_param_exprs().into_iter().map(|p| p.0).collect();
    let features: Vec<(u64, String)> = app.session.doc.features.iter().map(|f| (f.id, f.name.clone())).collect();
    let suppression_features: Vec<(u64, String)> =
        features.iter().filter(|(id, _)| app.session.doc.configs.columns.contains(&Column::Suppress(*id))).cloned().collect();
    ui.separator();
    ui.label(RichText::new(crate::i18n::tr_label("Configuration rules")).strong());
    ui.add(egui::Label::new(crate::i18n::tr("configuration.rules_order")).wrap());
    ui.add(egui::Label::new(crate::i18n::tr("configuration.restrictions_note")).wrap());
    for (index, rule) in rules.iter().enumerate() {
        ui.push_id(&rule.id, |ui| {
            ui.horizontal(|ui| {
                let mut enabled = rule.enabled;
                if ui.checkbox(&mut enabled, "").on_hover_text(crate::i18n::tr_label("Enable rule")).changed() {
                    let mut r = rule.clone();
                    r.enabled = enabled;
                    *error = app.run("config.rule", json!({"rule":r})).err();
                }
                ui.add(egui::Label::new(&rule.name).truncate());
                if ui.small_button(crate::i18n::tr_label("Edit")).clicked() {
                    editor.rule = rule.clone();
                }
                if ui.add_enabled(index > 0, egui::Button::new("↑").small()).on_hover_text(crate::i18n::tr_label("Move earlier")).clicked() {
                    *error = app.run("config.rule", json!({"id":rule.id,"index":index-1})).err();
                }
                if ui
                    .add_enabled(index + 1 < rules.len(), egui::Button::new("↓").small())
                    .on_hover_text(crate::i18n::tr_label("Move later"))
                    .clicked()
                {
                    *error = app.run("config.rule", json!({"id":rule.id,"index":index+1})).err();
                }
                if ui.small_button(crate::i18n::tr_label("Delete")).clicked() {
                    *error = app.run("config.rule", json!({"id":rule.id,"delete":true})).err();
                }
            });
            ui.label(match &rule.effect {
                RuleEffect::DriveParameter { parameter, expression } => {
                    crate::i18n::format("configuration.drives_parameter", &[("parameter", parameter), ("expression", expression)])
                }
                RuleEffect::DriveSuppression { feature, value } => crate::i18n::format(
                    "configuration.drives_suppression",
                    &[
                        (
                            "feature",
                            features
                                .iter()
                                .find(|f| f.0 == *feature)
                                .map(|f| f.1.as_str())
                                .unwrap_or(crate::i18n::tr("configuration.missing_feature")),
                        ),
                        ("value", &value.to_string()),
                    ],
                ),
                RuleEffect::ParameterRange { parameter, minimum, maximum } => {
                    crate::i18n::format("configuration.restricts_range", &[("parameter", parameter), ("minimum", minimum), ("maximum", maximum)])
                }
                RuleEffect::AllowParameterValues { parameter, expressions } => {
                    crate::i18n::format("configuration.restricts_values", &[("parameter", parameter), ("values", &expressions.join(", "))])
                }
            });
        });
    }
    ui.collapsing(crate::i18n::tr_label("Add or edit rule"), |ui| {
        ui.horizontal(|ui| {
            expression(ui, "Rule name", &mut editor.rule.name);
            ui.checkbox(&mut editor.rule.enabled, crate::i18n::tr_label("Enabled"));
            if ui.small_button(crate::i18n::tr("ui.new")).clicked() {
                editor.rule = Editor::default().rule;
            }
        });
        let mut length_unit = editor.rule.length_unit.clone().unwrap_or_else(|| app.session.doc.units.clone());
        let old_unit = length_unit.clone();
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr_label("Length units"));
            choose(ui, "rule_length_units", &mut length_unit, &["mm".into(), "cm".into(), "m".into(), "in".into(), "ft".into()]);
        });
        if length_unit != old_unit {
            editor.rule.length_unit = Some(length_unit);
        }
        condition_editor(ui, &mut editor.rule.condition, &all_parameters, &features, 1);
        let mut effect_kind = match editor.rule.effect {
            RuleEffect::DriveParameter { .. } => "Drive parameter",
            RuleEffect::DriveSuppression { .. } => "Drive suppression",
            RuleEffect::ParameterRange { .. } => "Range",
            RuleEffect::AllowParameterValues { .. } => "Allowed values",
        }
        .to_string();
        let prior = effect_kind.clone();
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr_label("Then"));
            choose_labels(
                ui,
                "rule_effect_kind",
                &mut effect_kind,
                &["Drive parameter".into(), "Drive suppression".into(), "Range".into(), "Allowed values".into()],
            );
        });
        if prior != effect_kind {
            let parameter = parameters.first().cloned().unwrap_or_default();
            editor.rule.effect = match effect_kind.as_str() {
                "Drive suppression" => RuleEffect::DriveSuppression { feature: suppression_features.first().map(|f| f.0).unwrap_or(0), value: true },
                "Range" => RuleEffect::ParameterRange { parameter, minimum: "0 mm".into(), maximum: "100 mm".into() },
                "Allowed values" => RuleEffect::AllowParameterValues { parameter, expressions: vec!["10 mm".into(), "20 mm".into()] },
                _ => RuleEffect::DriveParameter { parameter, expression: "10 mm".into() },
            };
        }
        match &mut editor.rule.effect {
            RuleEffect::DriveParameter { parameter, expression: value } => {
                ui.horizontal(|ui| {
                    choose(ui, "effect_parameter", parameter, &parameters);
                    expression(ui, "Value", value);
                });
            }
            RuleEffect::DriveSuppression { feature, value } => {
                ui.horizontal(|ui| {
                    feature_choice(ui, "effect_feature", feature, &suppression_features);
                    ui.checkbox(value, crate::i18n::tr_label("Suppressed"));
                });
            }
            RuleEffect::ParameterRange { parameter, minimum, maximum } => {
                choose(ui, "range_parameter", parameter, &parameters);
                ui.horizontal(|ui| {
                    expression(ui, "Minimum", minimum);
                });
                ui.horizontal(|ui| {
                    expression(ui, "Maximum", maximum);
                });
            }
            RuleEffect::AllowParameterValues { parameter, expressions } => {
                choose(ui, "allowed_parameter", parameter, &parameters);
                let mut text = expressions.join("; ");
                ui.label(crate::i18n::tr_label("Allowed expressions, separated by semicolons"));
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(300.0)).changed() {
                    *expressions = text.split(';').map(|s| s.trim().to_string()).collect();
                }
            }
        }
        if ui.add_enabled(!app.session.doc.configs.rows.is_empty(), egui::Button::new(crate::i18n::tr_label("Save Rule"))).clicked() {
            let result = (|| -> Result<(), String> {
                let mut rule = editor.rule.clone();
                if rule.id.is_empty() {
                    rule.id = (1..).map(|n| format!("Rule{n}")).find(|id| !rules.iter().any(|r| r.id == *id)).unwrap_or_default();
                }
                app.run("config.rule", json!({"rule":rule.clone()}))?;
                editor.rule = app.session.doc.configs.rules.iter().find(|existing| existing.id == rule.id).cloned().unwrap_or(rule);
                Ok(())
            })();
            *error = result.err();
        }
    });
    let rows: Vec<String> = app.session.doc.configs.rows.iter().map(|r| r.name.clone()).collect();
    if !rows.contains(&editor.row) {
        editor.row = app.session.doc.configs.active.clone().or_else(|| rows.first().cloned()).unwrap_or_default();
    }
    let previous_row = editor.row.clone();
    ui.horizontal(|ui| {
        choose(ui, "preview_configuration", &mut editor.row, &rows);
        if ui.add_enabled(!editor.row.is_empty(), egui::Button::new(crate::i18n::tr_label("Preview Row"))).clicked() {
            match app.session.execute("config.preview", &json!({"row":editor.row})) {
                Ok(result) => {
                    editor.preview = Some(result);
                    editor.preview_revision = app.session.revision;
                    *error = None;
                }
                Err(e) => {
                    editor.preview = None;
                    *error = Some(e.to_string());
                }
            }
        }
    });
    if previous_row != editor.row {
        editor.preview = None;
    }
    if let Some(preview) = &editor.preview {
        ui.collapsing(
            crate::i18n::format("ui.preview_label", &[("name", preview["active"].as_str().unwrap_or(crate::i18n::tr("configuration.row")))]),
            |ui| {
                ui.label(crate::i18n::tr_label("Preview leaves the current design unchanged."));
                if let Some(styles) = preview["styles"].as_array() {
                    for style in styles {
                        ui.add(
                            egui::Label::new(crate::i18n::format(
                                "configuration.style_state",
                                &[
                                    ("target", style["target"].as_str().unwrap_or(crate::i18n::tr_label("Appearance"))),
                                    (
                                        "state",
                                        crate::i18n::tr(if style["dormant"].as_bool() == Some(true) {
                                            "configuration.dormant"
                                        } else {
                                            "configuration.resolved"
                                        }),
                                    ),
                                ],
                            ))
                            .wrap(),
                        );
                    }
                }
                if let Some(trace) = preview["trace"].as_array() {
                    for entry in trace {
                        ui.label(crate::i18n::format(
                            "configuration.trace_state",
                            &[
                                ("name", entry["name"].as_str().unwrap_or(crate::i18n::tr("configuration.rule"))),
                                (
                                    "state",
                                    crate::i18n::tr(if entry["matched"].as_bool() == Some(true) {
                                        "configuration.matched"
                                    } else {
                                        "configuration.skipped"
                                    }),
                                ),
                            ],
                        ));
                        if entry["matched"].as_bool() == Some(true)
                            && let Ok(effect) = serde_json::from_value::<RuleEffect>(entry["effect"].clone())
                        {
                            ui.label(match effect {
                                RuleEffect::DriveParameter { parameter, expression } => {
                                    crate::i18n::format("configuration.driven_parameter", &[("parameter", &parameter), ("expression", &expression)])
                                }
                                RuleEffect::DriveSuppression { feature, value } => crate::i18n::format(
                                    "configuration.driven_suppression",
                                    &[
                                        (
                                            "feature",
                                            features
                                                .iter()
                                                .find(|f| f.0 == feature)
                                                .map(|f| f.1.as_str())
                                                .unwrap_or(crate::i18n::tr("configuration.feature")),
                                        ),
                                        (
                                            "state",
                                            crate::i18n::tr(if value {
                                                "configuration.suppressed_state"
                                            } else {
                                                "configuration.unsuppressed_state"
                                            }),
                                        ),
                                    ],
                                ),
                                RuleEffect::ParameterRange { parameter, minimum, maximum } => crate::i18n::format(
                                    "configuration.restricted_range",
                                    &[("parameter", &parameter), ("minimum", &minimum), ("maximum", &maximum)],
                                ),
                                RuleEffect::AllowParameterValues { parameter, expressions } => crate::i18n::format(
                                    "configuration.restricted_values",
                                    &[("parameter", &parameter), ("values", &expressions.join("; "))],
                                ),
                            });
                        }
                    }
                }
                if let Some(parameters) = preview["parameters"].as_array() {
                    for parameter in parameters {
                        if let Some(name) = parameter["name"].as_str()
                            && all_parameters.iter().any(|p| p == name)
                        {
                            ui.label(format!("{name} = {}", parameter["expression"].as_str().unwrap_or("")));
                        }
                    }
                }
            },
        );
    }
}
fn feature_choice(ui: &mut Ui, id: &str, feature: &mut u64, features: &[(u64, String)]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(features.iter().find(|f| f.0 == *feature).map(|f| f.1.as_str()).unwrap_or(crate::i18n::tr("configuration.choose_feature")))
        .show_ui(ui, |ui| {
            for (id, name) in features {
                ui.selectable_value(feature, *id, name);
            }
        });
}

fn condition_editor(ui: &mut Ui, condition: &mut RuleCondition, parameters: &[String], features: &[(u64, String)], depth: usize) {
    let mut kind = match condition {
        RuleCondition::Always => "Always",
        RuleCondition::Parameter { .. } => "Parameter",
        RuleCondition::Suppression { .. } => "Suppression",
        RuleCondition::All { .. } => "All",
        RuleCondition::Any { .. } => "Any",
        RuleCondition::Not { .. } => "Not",
    }
    .to_string();
    let previous = kind.clone();
    let mut choices = vec!["Always".into(), "Parameter".into(), "Suppression".into()];
    if depth < 8 {
        choices.extend(["All".into(), "Any".into(), "Not".into()]);
    }
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr_label("When"));
        choose_labels(ui, "condition_kind", &mut kind, &choices);
    });
    if previous != kind {
        *condition = match kind.as_str() {
            "Parameter" => RuleCondition::Parameter {
                parameter: parameters.first().cloned().unwrap_or_default(),
                operator: RuleOperator::Ge,
                expression: "10 mm".into(),
            },
            "Suppression" => RuleCondition::Suppression { feature: features.first().map(|f| f.0).unwrap_or(0), value: true },
            "All" => RuleCondition::All { conditions: vec![RuleCondition::Always] },
            "Any" => RuleCondition::Any { conditions: vec![RuleCondition::Always] },
            "Not" => RuleCondition::Not { condition: Box::new(RuleCondition::Always) },
            _ => RuleCondition::Always,
        };
    }
    match condition {
        RuleCondition::Parameter { parameter, operator, expression } => {
            ui.horizontal(|ui| {
                choose(ui, "condition_parameter", parameter, parameters);
                egui::ComboBox::from_id_salt("condition_operator")
                    .width(45.0)
                    .selected_text(match operator {
                        RuleOperator::Eq => "=",
                        RuleOperator::Ne => "≠",
                        RuleOperator::Lt => "<",
                        RuleOperator::Le => "≤",
                        RuleOperator::Gt => ">",
                        RuleOperator::Ge => "≥",
                    })
                    .show_ui(ui, |ui| {
                        for (label, op) in [
                            ("=", RuleOperator::Eq),
                            ("≠", RuleOperator::Ne),
                            ("<", RuleOperator::Lt),
                            ("≤", RuleOperator::Le),
                            (">", RuleOperator::Gt),
                            ("≥", RuleOperator::Ge),
                        ] {
                            ui.selectable_value(operator, op, label);
                        }
                    });
                ui.add(egui::TextEdit::singleline(expression).desired_width(110.0));
            });
        }
        RuleCondition::Suppression { feature, value } => {
            ui.horizontal(|ui| {
                feature_choice(ui, "condition_feature", feature, features);
                ui.checkbox(value, crate::i18n::tr_label("Suppressed"));
            });
        }
        RuleCondition::All { conditions } | RuleCondition::Any { conditions } => {
            let mut remove = None;
            for (index, child) in conditions.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.indent("condition_group", |ui| {
                        condition_editor(ui, child, parameters, features, depth + 1);
                        if ui.small_button(crate::i18n::tr("configuration.remove_condition")).clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                conditions.remove(index);
            }
            if ui.add_enabled(conditions.len() < 32, egui::Button::new(crate::i18n::tr_label("Add condition")).small()).clicked() {
                conditions.push(RuleCondition::Always);
            }
        }
        RuleCondition::Not { condition } => {
            ui.indent("negated_condition", |ui| {
                condition_editor(ui, condition, parameters, features, depth + 1);
            });
        }
        RuleCondition::Always => {}
    }
}
