//! Seeded goldens: what every robustness seed builds (`tests/robustness/goldens.json`).
//!
//! Each seed's random design (`fuzz::run`) is recorded: its steps, how it ended (ok, or the
//! kind and start of the error it was refused with) and, for what it left, the bodies' total
//! volume and area (within a relative 1e-6) and their face, edge and shell counts (exact). Any
//! change fails, the better and the worse alike: a seed that used to be refused and now builds
//! is recorded on purpose, with the reason. To record: `ROBUSTNESS_GOLDENS_BLESS="why" cargo
//! test --profile ci -p solidcraft-engine --lib robustness_goldens:: -- --include-ignored`
//! (docs/robustness.md). The reason goes into the file's `notes` with the seeds it changed.
//!
//! Pull requests that touch `crates/kernel` run a sample (`sample_0` … `sample_3`, every 8th
//! seed, split so the shards share them); pushes to main and the nightly run every seed
//! (`all_0` … `all_3`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::fuzz;

/// Seeds 0..SEEDS (about ten minutes for all of them on one core), each with up to this many steps (as `fixed_seeds_run_clean`).
const SEEDS: u64 = 96;
const MAX_STEPS: usize = 8;
const REL: f64 = 1e-6;

/// Tests share the file when they record.
static FILE: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/robustness/goldens.json")
}

fn load() -> Value {
    let allow_new = !cfg!(target_os = "windows") && std::env::var("ROBUSTNESS_GOLDENS_BLESS").is_ok();
    read_record(std::fs::read(path()), allow_new).unwrap_or_else(|e| panic!("{}: {e}", path().display()))
}

/// How `got` differs from `want` (None when it matches within the tolerances).
fn differs(want: &Value, got: &Value) -> Option<String> {
    let mut out = Vec::new();
    for (k, g) in got.as_object().into_iter().flatten() {
        let w = &want[k];
        let same = match (w.as_f64(), g.as_f64()) {
            (Some(a), Some(b)) if matches!(k.as_str(), "volume" | "area") => (a - b).abs() <= REL * a.abs().max(b.abs()).max(1.0),
            _ => w == g,
        };
        if !same {
            out.push(format!("{k}: {w} -> {g}"));
        }
    }
    (!out.is_empty()).then(|| out.join("; "))
}

const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

fn record_target() -> Option<&'static str> {
    cfg!(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", target_vendor = "pc")).then_some(WINDOWS_TARGET)
}

/// A qualified record can change only finite, nonnegative numeric metrics, never behavior.
fn validate_platforms(file: &Value) -> Result<(), String> {
    let Some(platforms) = file.get("platforms") else { return Ok(()) };
    let platforms = platforms.as_object().ok_or("platforms must be an object")?;
    for (target, block) in platforms {
        if target != WINDOWS_TARGET {
            return Err(format!("unsupported golden target: {target}"));
        }
        let block = block.as_object().ok_or("platform record must be an object")?;
        if block.keys().any(|key| !matches!(key.as_str(), "reason" | "seeds")) {
            return Err("platform record accepts only reason and seeds".into());
        }
        if !block.get("reason").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()) {
            return Err("platform record requires a nonempty reason".into());
        }
        let seeds = block.get("seeds").and_then(Value::as_object).ok_or("platform seeds must be an object")?;
        for (key, fields) in seeds {
            let seed = key.parse::<u64>().map_err(|_| format!("invalid platform seed: {key}"))?;
            if seed >= SEEDS || seed.to_string() != *key {
                return Err(format!("invalid platform seed: {key}"));
            }
            let common = file
                .get("seeds")
                .and_then(|s| s.get(key))
                .and_then(Value::as_object)
                .ok_or_else(|| format!("platform seed {key} has no common record"))?;
            let fields = fields.as_object().ok_or("platform override must be an object")?;
            if fields.is_empty() {
                return Err("platform override must contain a numeric field".into());
            }
            for (field, value) in fields {
                let valid = match field.as_str() {
                    "area" | "volume" => value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.),
                    "faces" | "edges" => value.as_u64().is_some(),
                    _ => return Err(format!("platform override cannot change {field}")),
                };
                if !valid || !common.contains_key(field) {
                    return Err(format!("platform override {key}.{field} must be finite and nonnegative; counts must be integers"));
                }
            }
        }
    }
    Ok(())
}

/// Existing files must contain the common-record map; only an authorized missing
/// file may start from `{}`. Partial maps remain valid while recording CI shards.
fn validate_common_record(file: &Value) -> Result<(), String> {
    let root = file.as_object().ok_or("golden record must be an object")?;
    let seeds = root.get("seeds").and_then(Value::as_object).ok_or("common seeds must be an object")?;
    for (key, record) in seeds {
        let seed = key.parse::<u64>().map_err(|_| format!("invalid common seed: {key}"))?;
        if seed >= SEEDS || seed.to_string() != *key {
            return Err(format!("invalid common seed: {key}"));
        }
        if !record.is_object() {
            return Err(format!("common seed {key} must be an object"));
        }
    }
    Ok(())
}

fn decode_record(bytes: &[u8]) -> Result<Value, String> {
    let file: Value = serde_json::from_slice(bytes).map_err(|e| format!("invalid golden JSON: {e}"))?;
    validate_common_record(&file)?;
    validate_platforms(&file)?;
    Ok(file)
}

/// Preserve first recording on the default target, never replacing malformed/unreadable data.
fn read_record(bytes: std::io::Result<Vec<u8>>, allow_new: bool) -> Result<Value, String> {
    match bytes {
        Ok(bytes) => decode_record(&bytes),
        Err(e) if allow_new && e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.to_string()),
    }
}

/// Merge into a clone. Other targets and unlisted seeds always keep the common record.
fn expected_record(file: &Value, seed: u64, target: Option<&str>) -> Result<Value, String> {
    validate_platforms(file)?;
    let key = seed.to_string();
    let mut expected = file["seeds"][&key].clone();
    if target == Some(WINDOWS_TARGET)
        && let Some(fields) = file
            .get("platforms")
            .and_then(|p| p.get(WINDOWS_TARGET))
            .and_then(|p| p.get("seeds"))
            .and_then(|s| s.get(&key))
            .and_then(Value::as_object)
    {
        let common = expected.as_object_mut().ok_or("platform seed has no common object")?;
        for (field, value) in fields {
            common.insert(field.clone(), value.clone());
        }
    }
    Ok(expected)
}

fn common_recording_allowed(windows: bool) -> Result<(), &'static str> {
    if windows {
        Err(
            "ROBUSTNESS_GOLDENS_BLESS cannot overwrite common records on Windows; review supported target numeric overrides with a reason, and record common changes on the default CI target",
        )
    } else {
        Ok(())
    }
}

/// Keep common seeds readable and preserve every qualified record during default-target blessing.
fn formatted_record(file: &Value) -> String {
    let seeds: BTreeMap<u64, Value> =
        file["seeds"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.parse::<u64>().ok()?, v.clone()))).collect();
    let mut s = format!("{{\n \"about\": {},\n \"seeds\": {{\n", file["about"]);
    let n = seeds.len();
    for (i, (k, v)) in seeds.iter().enumerate() {
        s += &format!("  \"{k}\": {}{}\n", v, if i + 1 < n { "," } else { "" });
    }
    let notes: Vec<String> = file["notes"].as_array().into_iter().flatten().map(|n| format!("  {n}")).collect();
    s += &format!(" }},\n \"notes\": [\n{}\n ]", notes.join(",\n"));
    if let Some(platforms) = file.get("platforms") {
        s += &format!(",\n \"platforms\": {platforms}");
    }
    s += "\n}\n";
    s
}

fn check(part: u64, parts: u64, every: u64) {
    if std::env::var_os("ROBUSTNESS_GOLDENS_BLESS").is_some() {
        common_recording_allowed(cfg!(target_os = "windows")).unwrap_or_else(|e| panic!("{e}"));
    }
    let target = record_target();
    let seeds: Vec<u64> = (0..SEEDS).filter(|s| s % every == 0 && (s / every) % parts == part).collect();
    let got: BTreeMap<u64, Value> = seeds.iter().map(|s| (*s, fuzz::golden(*s, MAX_STEPS))).collect();
    let _lock = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = load();
    let changed: Vec<(u64, String)> = got
        .iter()
        .filter_map(|(seed, g)| {
            let w = expected_record(&file, *seed, target).unwrap_or_else(|e| panic!("{}: {e}", path().display()));
            if w.is_null() { Some((*seed, "new".to_string())) } else { differs(&w, g).map(|d| (*seed, d)) }
        })
        .collect();
    if let Ok(reason) = std::env::var("ROBUSTNESS_GOLDENS_BLESS") {
        assert!(!reason.trim().is_empty(), "ROBUSTNESS_GOLDENS_BLESS needs the reason for the change");
        if changed.is_empty() {
            return;
        }
        if file.get("about").is_none() {
            file["about"] = json!(
                "What each robustness seed builds (crates/engine/src/tests/robustness_goldens.rs, docs/robustness.md). Record changes with ROBUSTNESS_GOLDENS_BLESS=\"why\"."
            );
        }
        for (seed, g) in &got {
            file["seeds"][seed.to_string()] = g.clone();
        }
        let mut notes = file["notes"].as_array().cloned().unwrap_or_default();
        let seeds: Vec<u64> = changed.iter().map(|(s, _)| *s).collect();
        // The parts of one record (the tests run side by side) share one note.
        match notes.last_mut() {
            Some(last) if last["reason"] == json!(reason) => {
                let mut all: Vec<u64> = last["seeds"].as_array().into_iter().flatten().filter_map(Value::as_u64).chain(seeds).collect();
                all.sort_unstable();
                all.dedup();
                last["seeds"] = json!(all);
            }
            _ => notes.push(json!({"reason": reason, "seeds": seeds})),
        }
        file["notes"] = Value::Array(notes);
        let s = formatted_record(&file);
        std::fs::write(path(), s).unwrap_or_else(|e| panic!("{}: {e}", path().display()));
        return;
    }
    assert!(
        changed.is_empty(),
        "{} seed(s) build differently from tests/robustness/goldens.json (if that is intended, record it with ROBUSTNESS_GOLDENS_BLESS=\"why\" and say why in the pull request):\n{}",
        changed.len(),
        changed.iter().map(|(s, d)| format!("seed {s}: {d}")).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn sample_0() {
    check(0, 4, 8);
}

#[test]
fn sample_1() {
    check(1, 4, 8);
}

#[test]
fn sample_2() {
    check(2, 4, 8);
}

#[test]
fn sample_3() {
    check(3, 4, 8);
}

#[test]
fn all_0() {
    check(0, 4, 1);
}

#[test]
fn all_1() {
    check(1, 4, 1);
}

#[test]
fn all_2() {
    check(2, 4, 1);
}

#[test]
fn all_3() {
    check(3, 4, 1);
}

#[cfg(test)]
mod target_baseline_tests {
    use super::*;

    fn fixture() -> Value {
        let common = json!({"area":200.,"volume":100.,"faces":5,"edges":9,"steps":["box","hole"],"outcome":"ok","error":null,"unsupported":2,"rejected":1,"bodies":1,"shells":1});
        json!({"about":"synthetic contract fixture","seeds":{"7":common,"8":common},"notes":[{"reason":"common reason","seeds":[7,8]}],"platforms":{WINDOWS_TARGET:{"reason":"reviewed target behavior","seeds":{"7":{"edges":11,"area":201.}}}}})
    }

    #[test]
    fn only_the_exact_target_and_listed_seed_receive_numeric_overrides() {
        let file = fixture();
        let common = file["seeds"]["7"].clone();
        let expected = expected_record(&file, 7, Some(WINDOWS_TARGET)).unwrap();
        assert_eq!(expected["edges"], 11);
        assert_eq!(expected["area"], 201.);
        for field in ["volume", "faces", "steps", "outcome", "error", "unsupported", "rejected", "bodies", "shells"] {
            assert_eq!(expected[field], common[field]);
        }
        assert_eq!(file["seeds"]["7"], common);
        assert_eq!(expected_record(&file, 8, Some(WINDOWS_TARGET)).unwrap(), file["seeds"]["8"]);
        for target in [
            None,
            Some("x86_64-unknown-linux-gnu"),
            Some("aarch64-pc-windows-msvc"),
            Some("x86_64-pc-windows-gnu"),
            Some("x86_64-uwp-windows-msvc"),
            Some("x86_64-win7-windows-msvc"),
            Some("windows"),
            Some("*"),
        ] {
            assert_eq!(expected_record(&file, 7, target).unwrap(), common);
        }
    }

    #[test]
    fn default_record_without_platforms_remains_identical() {
        let mut file = fixture();
        file.as_object_mut().unwrap().remove("platforms");
        assert_eq!(expected_record(&file, 7, Some(WINDOWS_TARGET)).unwrap(), file["seeds"]["7"]);
        assert_eq!(expected_record(&file, 7, None).unwrap(), file["seeds"]["7"]);
    }

    #[test]
    fn malformed_targets_seeds_and_protected_fields_are_rejected() {
        let base = fixture();
        let mut cases = Vec::new();
        for bad in [Value::Null, json!([]), json!("not a map")] {
            let mut f = base.clone();
            f["platforms"] = bad;
            cases.push(f);
        }
        let mut f = base.clone();
        f["platforms"] = json!({"*":{"reason":"wildcard","seeds":{}}});
        cases.push(f);
        let mut f = base.clone();
        f["platforms"][WINDOWS_TARGET]["reason"] = json!(" ");
        cases.push(f);
        let mut f = base.clone();
        f["platforms"][WINDOWS_TARGET]["unknown"] = json!(true);
        cases.push(f);
        for key in ["-1", "96", "007", "not-a-seed", "9"] {
            let mut f = base.clone();
            f["platforms"][WINDOWS_TARGET]["seeds"] = json!({key:{"edges":11}});
            cases.push(f);
        }
        for bad in [Value::Null, json!([]), json!({}), json!({"outcome":"failed"}), json!({"steps":[]}), json!({"shells":2}), json!({"bodies":2})] {
            let mut f = base.clone();
            f["platforms"][WINDOWS_TARGET]["seeds"]["7"] = bad;
            cases.push(f);
        }
        for f in cases {
            assert!(validate_platforms(&f).is_err(), "accepted malformed override: {f}");
            assert!(expected_record(&f, 7, None).is_err(), "bad metadata must fail on the default target too");
        }
    }

    #[test]
    fn numeric_fields_are_finite_nonnegative_and_counts_are_integers() {
        for fields in
            [json!({"area":"NaN"}), json!({"volume":null}), json!({"area":-1.}), json!({"edges":1.5}), json!({"faces":-1}), json!({"faces":true})]
        {
            let mut file = fixture();
            file["platforms"][WINDOWS_TARGET]["seeds"]["7"] = fields;
            assert!(validate_platforms(&file).is_err());
        }
        let mut file = fixture();
        file["platforms"][WINDOWS_TARGET]["seeds"]["7"] = json!({"area":0.,"volume":0,"faces":0,"edges":0});
        assert!(validate_platforms(&file).is_ok());
        assert!(decode_record(b"{").is_err());
        assert!(decode_record(br#"{"platforms":{"x86_64-pc-windows-msvc":{"reason":"overflow","seeds":{"7":{"area":1e999}}}}}"#).is_err());
    }

    #[test]
    fn original_relative_tolerance_and_exact_behavior_checks_still_apply() {
        let file = fixture();
        let expected = expected_record(&file, 7, Some(WINDOWS_TARGET)).unwrap();
        let mut got = expected.clone();
        got["volume"] = json!(100. * (1. + REL * 0.5));
        assert!(differs(&expected, &got).is_none());
        got["volume"] = json!(100. * (1. + REL * 2.));
        assert!(differs(&expected, &got).is_some());
        for (field, changed) in
            [("edges", json!(12)), ("faces", json!(6)), ("outcome", json!("failed")), ("steps", json!(["box"])), ("unsupported", json!(3))]
        {
            let mut got = expected.clone();
            got[field] = changed;
            assert!(differs(&expected, &got).is_some());
        }
    }

    #[test]
    fn common_writer_preserves_platforms_and_windows_recording_is_refused() {
        let file = fixture();
        let output = formatted_record(&file);
        assert_eq!(decode_record(output.as_bytes()).unwrap(), file);
        assert!(common_recording_allowed(false).is_ok());
        let error = common_recording_allowed(true).unwrap_err();
        assert!(error.contains("cannot overwrite common records"));
        assert!(error.contains("numeric overrides"));
        assert!(error.contains("reason"));
    }

    #[test]
    fn existing_common_schema_is_checked_even_when_recording_is_authorized() {
        let mut malformed = vec![
            Value::Null,
            json!([]),
            json!(true),
            json!("not a record"),
            json!({}),
            json!({"seeds":null}),
            json!({"seeds":[]}),
            json!({"seeds":"not a map"}),
            json!({"seeds":{"7":null}}),
            json!({"seeds":{"7":[]}}),
        ];
        for key in ["-1", "007", "96", "not-a-seed"] {
            malformed.push(json!({"seeds":{key:{"outcome":"ok"}}}));
        }
        for file in malformed {
            let bytes = serde_json::to_vec(&file).unwrap();
            for allow_new in [false, true] {
                assert!(read_record(Ok(bytes.clone()), allow_new).is_err(), "accepted existing malformed file: {file}");
            }
        }
        // A first shard may leave a valid partial map; recording the other shards
        // must still work. Schema validation does not reinterpret measurement nulls.
        let partial = json!({"seeds":{"7":{"area":null,"volume":null,"outcome":"failed"}}});
        let bytes = serde_json::to_vec(&partial).unwrap();
        assert_eq!(read_record(Ok(bytes), true).unwrap(), partial);
    }

    #[test]
    fn first_recording_accepts_only_an_authorized_missing_file() {
        let missing = || Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(read_record(missing(), true).unwrap(), json!({}));
        assert!(read_record(missing(), false).is_err());
        assert!(read_record(Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)), true).is_err());
        assert!(read_record(Ok(b"{".to_vec()), true).is_err());
        let mut malformed = fixture();
        malformed["platforms"] = json!(null);
        assert!(read_record(Ok(serde_json::to_vec(&malformed).unwrap()), true).is_err());
        let valid = fixture();
        assert_eq!(read_record(Ok(serde_json::to_vec(&valid).unwrap()), true).unwrap(), valid);
    }
}
