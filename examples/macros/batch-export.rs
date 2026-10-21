//! Export every STEP in a folder to STEP or STL.
//!
//! `batch-export [dir] [out_dir] [ext]`  ext is `step` (default) or `stl`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use solidcraft_macro_client::App;

fn is_step(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let out_dir = PathBuf::from(std::env::args().nth(2).unwrap_or_else(|| "export".into()));
    let ext = std::env::args().nth(3).filter(|s| !s.is_empty()).unwrap_or_else(|| "step".into());
    if !matches!(ext.as_str(), "step" | "stl") {
        return Err(format!("ext must be step or stl, not {ext}").into());
    }
    fs::create_dir_all(&out_dir)?;
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    let mut files: Vec<_> = fs::read_dir(&dir)?.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_step(p)).collect();
    files.sort();
    let mut n = 0usize;
    for path in &files {
        app.run("doc.open", json!({"path": path.to_string_lossy()}))?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "part".into());
        let dest = out_dir.join(format!("{stem}.{ext}"));
        if path.canonicalize().ok().is_some_and(|p| dest.canonicalize().ok().as_ref() == Some(&p)) {
            continue;
        }
        app.run("file.export", json!({"path": dest.to_string_lossy()}))?;
        n += 1;
    }
    eprintln!("wrote {n} file(s) to {}", out_dir.display());
    Ok(())
}
