//! Conservative display-only adapter for existing English diagnostic templates.
//! It never rewrites the engine's errors, saved data, command IDs or logs.
use std::collections::BTreeMap;

#[derive(Debug)]
enum Part {
    Literal(String),
    Argument(String),
}

#[derive(Debug)]
struct Pattern {
    parts: Vec<Part>,
    translation: String,
    weight: usize,
}

/// Only translated entries are indexed. Untranslated and ambiguous messages stay intact.
#[derive(Default)]
pub(super) struct Diagnostics {
    exact: BTreeMap<String, String>,
    patterns: Vec<Pattern>,
}

fn parts(template: &str) -> Option<Vec<Part>> {
    let mut result = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '{' | '}') && chars.peek() == Some(&c) {
            chars.next();
            literal.push(c);
        } else if c == '{' {
            if !literal.is_empty() {
                result.push(Part::Literal(std::mem::take(&mut literal)));
            }
            let mut name = String::new();
            loop {
                let next = chars.next()?;
                if next == '}' {
                    break;
                }
                if !(next.is_ascii_alphanumeric() || next == '_') {
                    return None;
                }
                name.push(next);
            }
            if name.is_empty() || matches!(result.last(), Some(Part::Argument(_))) {
                return None;
            }
            result.push(Part::Argument(name));
        } else if c == '}' {
            return None;
        } else {
            literal.push(c);
        }
    }
    if !literal.is_empty() {
        result.push(Part::Literal(literal));
    }
    Some(result)
}

impl Diagnostics {
    pub(super) fn new<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let mut out = Self::default();
        for (english, translation) in entries {
            if english == translation {
                continue;
            }
            let Some(parts) = parts(english) else { continue };
            let weight = parts
                .iter()
                .map(|part| match part {
                    Part::Literal(s) => s.len(),
                    _ => 0,
                })
                .sum();
            if parts.iter().all(|part| matches!(part, Part::Literal(_))) {
                let value: String = parts.iter().filter_map(|p| if let Part::Literal(s) = p { Some(s.as_str()) } else { None }).collect();
                out.exact.insert(value, translation.to_string());
            } else if weight >= 8 {
                // Generic templates such as "{arg1}: {arg2}" cannot identify an error safely.
                out.patterns.push(Pattern { parts, translation: translation.to_string(), weight });
            }
        }
        out.patterns.sort_by_key(|p| std::cmp::Reverse(p.weight));
        out
    }

    pub(super) fn translate(&self, message: &str) -> String {
        self.translate_at(message, 0)
    }

    fn translate_at(&self, message: &str, depth: usize) -> String {
        if message.len() > 16384 || depth > 4 {
            return message.to_string();
        }
        if let Some(translated) = self.exact.get(message) {
            return render(translated, &BTreeMap::new()).unwrap_or_else(|| message.to_string());
        }
        let mut best: Option<(usize, String)> = None;
        for pattern in &self.patterns {
            if best.as_ref().is_some_and(|(weight, _)| *weight > pattern.weight) {
                break;
            }
            let Some(mut args) = pattern.capture(message) else { continue };
            // These named fields explicitly contain another diagnostic, rather than a user name.
            for (name, value) in &mut args {
                if matches!(name.as_str(), "reason" | "error" | "msg" | "why") {
                    *value = self.translate_at(value, depth + 1);
                }
            }
            let Some(translated) = render(&pattern.translation, &args) else { continue };
            if best.as_ref().is_some_and(|(_, previous)| previous != &translated) {
                return message.to_string();
            }
            best = Some((pattern.weight, translated));
        }
        best.map_or_else(|| message.to_string(), |(_, translated)| translated)
    }
}

impl Pattern {
    fn capture(&self, message: &str) -> Option<BTreeMap<String, String>> {
        let mut rest = message;
        let mut args = BTreeMap::new();
        for (index, part) in self.parts.iter().enumerate() {
            match part {
                Part::Literal(s) => rest = rest.strip_prefix(s)?,
                Part::Argument(name) => {
                    let end = if let Some(Part::Literal(next)) = self.parts.get(index + 1) {
                        let mut positions = rest.match_indices(next);
                        let (offset, _) = positions.next()?;
                        // Do not guess where an argument ends when its separator occurs twice.
                        if positions.next().is_some() {
                            return None;
                        }
                        offset
                    } else {
                        rest.len()
                    };
                    let value = rest.get(..end)?;
                    if let Some(previous) = args.get(name) {
                        if previous != value {
                            return None;
                        }
                    } else {
                        args.insert(name.clone(), value.to_string());
                    }
                    rest = rest.get(end..)?;
                }
            }
        }
        rest.is_empty().then_some(args)
    }
}

fn render(template: &str, args: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::new();
    for part in parts(template)? {
        match part {
            Part::Literal(s) => out.push_str(&s),
            Part::Argument(name) => out.push_str(args.get(&name)?),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn axis_and_static_messages_translate_without_changing_arguments() {
        let d = Diagnostics::new(
            [
                ("the {axis} axis does not lie in the sketch plane", "{axis}-aksen ligger ikke i skitseplanet"),
                ("no active sketch", "Ingen aktiv skitse"),
            ]
            .into_iter(),
        );
        assert_eq!(d.translate("the Z axis does not lie in the sketch plane"), "Z-aksen ligger ikke i skitseplanet");
        assert_eq!(d.translate("no active sketch"), "Ingen aktiv skitse");
        assert_eq!(d.translate("unrecognised diagnostic"), "unrecognised diagnostic");
    }
    #[test]
    fn values_are_literal_and_can_be_reordered() {
        let d = Diagnostics::new([("body `{body}` has {count} faces", "{count} flader i `{body}`")].into_iter());
        assert_eq!(d.translate("body `Dør {count} 🛠` has 3 faces"), "3 flader i `Dør {count} 🛠`");
    }
    #[test]
    fn ambiguous_boundaries_and_generic_templates_are_preserved() {
        let d = Diagnostics::new([("body `{body}` has {count} faces", "{body}: {count}"), ("{x}: {y}", "{y}: {x}")].into_iter());
        let text = "body `A` has 3 faces` has 4 faces";
        assert_eq!(d.translate(text), text);
        assert_eq!(d.translate("arbitrary: user name"), "arbitrary: user name");
    }
    #[test]
    fn repeated_arguments_must_agree() {
        let d = Diagnostics::new([("parameter {name} refers to {name}", "Parameter {name} refererer til sig selv")].into_iter());
        assert_eq!(d.translate("parameter x refers to x"), "Parameter x refererer til sig selv");
        assert_eq!(d.translate("parameter x refers to y"), "parameter x refers to y");
    }
    #[test]
    fn malformed_translation_does_not_hide_diagnostics() {
        let d = Diagnostics::new([("invalid geometry", "fejl {missing}"), ("bad parameter {name}", "fejl {missing}")].into_iter());
        assert_eq!(d.translate("invalid geometry"), "invalid geometry");
        assert_eq!(d.translate("bad parameter radius"), "bad parameter radius");
    }
    #[test]
    fn nested_reasons_translate_but_names_stay_literal() {
        let d = Diagnostics::new(
            [
                ("invalid parameters for `{command}`: {reason}", "Ugyldige parametre til `{command}`: {reason}"),
                ("no active sketch", "Ingen aktiv skitse"),
            ]
            .into_iter(),
        );
        assert_eq!(
            d.translate("invalid parameters for `solid.revolve`: no active sketch"),
            "Ugyldige parametre til `solid.revolve`: Ingen aktiv skitse"
        );
    }
    #[test]
    fn escaped_json_braces_are_not_parameters() {
        let d = Diagnostics::new([("expected {{\"x\":1}}", "Forventede {{\"x\":1}}")].into_iter());
        assert_eq!(d.translate("expected {\"x\":1}"), "Forventede {\"x\":1}");
    }
    #[test]
    fn conflicting_templates_leave_original_intact() {
        let d = Diagnostics::new([("invalid value {name}", "Forkert {name}"), ("invalid value {value}", "Ugyldigt {value}")].into_iter());
        assert_eq!(d.translate("invalid value x"), "invalid value x");
    }
}
