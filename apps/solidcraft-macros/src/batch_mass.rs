//! Measure every STEP in a folder and write a CSV of mass properties.
//!
//! `batch-mass [dir] [out.csv]`

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use solidcraft_macro_client::App;

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn is_step(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let out = PathBuf::from(std::env::args().nth(2).unwrap_or_else(|| "mass.csv".into()));
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    let mut files: Vec<_> = fs::read_dir(&dir)?.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_step(p)).collect();
    files.sort();
    let mut rows = vec!["file,volume_mm3,mass_g,area_mm2".to_string()];
    for path in &files {
        app.run("doc.open", json!({"path": path.to_string_lossy()}))?;
        let m = app.run("inspect.measure", json!({}))?;
        let t = &m["total"];
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        rows.push(format!(
            "{},{},{},{}",
            csv_cell(&name),
            t["volume_mm3"].as_f64().unwrap_or(0.0),
            t["mass_g"].as_f64().unwrap_or(0.0),
            t["area_mm2"].as_f64().unwrap_or(0.0),
        ));
    }
    fs::write(&out, rows.join("\n") + "\n")?;
    eprintln!("wrote {} ({} parts)", out.display(), files.len());
    Ok(())
}
