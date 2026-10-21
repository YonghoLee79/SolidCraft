//! Headless MCP runs of the example binaries (`solidcraft-cli mcp`).

use std::path::PathBuf;
use std::process::Command;

use serde_json::json;
use solidcraft_macro_client::App;

fn solidcraft_cli() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SOLIDCRAFT_CLI") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut dirs = Vec::new();
    if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
        dirs.push(PathBuf::from(td));
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    for root in dirs {
        for profile in ["ci", "debug", "release"] {
            let p = root.join(profile).join("solidcraft-cli");
            if p.is_file() {
                return Some(p);
            }
            let p = p.with_extension("exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn skip_without_cli() -> Option<PathBuf> {
    match solidcraft_cli() {
        Some(p) => Some(p),
        None => {
            eprintln!("skip: solidcraft-cli is not built (set SOLIDCRAFT_CLI or cargo build -p solidcraft-cli)");
            None
        }
    }
}

#[test]
fn plate_generator_over_mcp_makes_four_holes() {
    let Some(cli) = skip_without_cli() else { return };
    let mut app = App::headless_cli(&cli).expect("mcp");
    app.run("solid.box", json!({"length": "120 mm", "width": "80 mm", "height": "8 mm", "name": "Plate"})).unwrap();
    for (i, (x, y)) in [("10", "10"), ("120 - 10", "10"), ("10", "80 - 10"), ("120 - 10", "80 - 10")].into_iter().enumerate() {
        app.run("solid.hole", json!({"position": [x, y, "8 mm"], "diameter": "6.5 mm", "name": format!("Hole{}", i + 1)})).unwrap();
    }
    let found = app.run("inspect.find_holes", json!({"max_diameter": "7 mm"})).unwrap();
    assert_eq!(found["count"], 4, "{found}");
}

#[test]
fn plate_generator_bin_exits_ok() {
    let Some(cli) = skip_without_cli() else { return };
    let bin = env!("CARGO_BIN_EXE_plate-generator");
    let out = Command::new(bin).env("SOLIDCRAFT_CLI", &cli).env_remove("SOLIDCRAFT_CONTROL_PORT").output().expect("spawn plate-generator");
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn batch_mass_writes_csv_for_a_step_folder() {
    let Some(cli) = skip_without_cli() else { return };
    let dir = std::env::temp_dir().join(format!("sc-batch-mass-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let step = dir.join("cube.step");
    let csv = dir.join("mass.csv");
    {
        let mut app = App::headless_cli(&cli).expect("mcp");
        app.run("solid.box", json!({"length": 10, "width": 10, "height": 10})).unwrap();
        app.run("file.export", json!({"path": step.display().to_string()})).unwrap();
    }
    let bin = env!("CARGO_BIN_EXE_batch-mass");
    let out = Command::new(bin)
        .args([dir.as_os_str(), csv.as_os_str()])
        .env("SOLIDCRAFT_CLI", &cli)
        .env_remove("SOLIDCRAFT_CONTROL_PORT")
        .output()
        .expect("spawn batch-mass");
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.contains("cube.step"), "{text}");
    assert!(text.lines().count() >= 2, "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
