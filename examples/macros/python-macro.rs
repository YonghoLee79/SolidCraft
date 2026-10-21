//! Run a Python macro against the open design. Python is only needed for this binary.
//!
//! `python-macro [script.py]`  with no arg, a file picker. Needs `python3`/`python`/`py` on PATH.

use std::path::PathBuf;
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python", "py"].into_iter().find(|name| Command::new(name).arg("-c").arg("import sys").status().ok().is_some_and(|s| s.success()))
}

fn pick_script() -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter("Python", &["py"]).set_title("Python macro").pick_file()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = match std::env::args().nth(1).filter(|s| !s.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => pick_script().ok_or("no Python script selected")?,
    };
    if !script.is_file() {
        return Err(format!("not a file: {}", script.display()).into());
    }
    let py = python().ok_or("python3, python or py is not on PATH")?;
    let mut cmd = Command::new(py);
    cmd.arg(&script);
    if let Some(dir) = script.parent().filter(|p| !p.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    for key in ["SOLIDCRAFT_CONTROL_PORT", "SOLIDCRAFT_CONTROL_TOKEN", "SOLIDCRAFT_CONTROL_DIR", "SOLIDCRAFT_CLI"] {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let status = cmd.status()?;
    if status.success() { Ok(()) } else { Err(format!("{py} {} failed", script.display()).into()) }
}
