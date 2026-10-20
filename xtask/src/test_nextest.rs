//! Optional test-level scheduling for full local CI. Reuse the built binaries so the Wasm and
//! doc-test jobs can use Cargo alongside the native run, without rebuilding native tests.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Instant;

use crate::cargo;

/// Select the full-CI runner without changing `xtask test`'s argument forwarding.
pub fn run_selected(side: Vec<(String, Command)>) -> Result<(), String> {
    match std::env::var("SOLIDCRAFT_TEST_RUNNER") {
        Err(std::env::VarError::NotPresent) => crate::test::run(&[], &[], side),
        Ok(value) if value == "libtest" => crate::test::run(&[], &[], side),
        Ok(value) if value == "nextest" => run(side),
        _ => Err("SOLIDCRAFT_TEST_RUNNER must be libtest or nextest".into()),
    }
}

pub struct Prepared {
    pub cargo_metadata: PathBuf,
    pub binaries_metadata: PathBuf,
}

impl Prepared {
    pub fn command(&self) -> Command {
        self.command_with_threads(std::env::var_os("NEXTEST_TEST_THREADS"))
    }

    fn command_with_threads(&self, threads: Option<OsString>) -> Command {
        let mut c = cargo();
        c.args(["nextest", "run", "--cargo-metadata"]).arg(&self.cargo_metadata).arg("--binaries-metadata").arg(&self.binaries_metadata).args([
            "--profile",
            "ci",
            "--ignore-default-filter",
            "--no-fail-fast",
            "--retries",
            "0",
            "--no-tests",
            "fail",
            "--test-threads",
        ]);
        // One geometry test can also use Rayon. Bound the default; callers can explicitly
        // choose another nextest concurrency without changing libtest's per-binary setting.
        let default = std::thread::available_parallelism().map(|n| n.get().min(4)).unwrap_or(1);
        c.arg(threads.unwrap_or_else(|| default.to_string().into()));
        c
    }
}

pub fn run(side: Vec<(String, Command)>) -> Result<(), String> {
    let prepared = prepare()?;
    execute(("nextest full workspace".into(), prepared.command()), side)
}

pub fn prepare() -> Result<Prepared, String> {
    let mut version = cargo();
    version.args(["nextest", "--version"]);
    capture(version, "cargo nextest --version")
        .map_err(|e| format!("{e}; install cargo-nextest first (https://nexte.st/docs/installation/pre-built-binaries/)"))?;
    let mut metadata = cargo();
    metadata.args(["metadata", "--format-version", "1", "--locked"]);
    let cargo_json = capture(metadata, "cargo metadata")?;
    let parsed: serde_json::Value = serde_json::from_slice(&cargo_json).map_err(|e| format!("Cargo metadata: {e}"))?;
    let target = parsed.get("target_directory").and_then(|v| v.as_str()).ok_or("Cargo metadata has no target_directory")?;
    let dir = PathBuf::from(target).join("xtask-nextest").join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let cargo_metadata = dir.join("cargo-metadata.json");
    std::fs::write(&cargo_metadata, cargo_json).map_err(|e| format!("write {}: {e}", cargo_metadata.display()))?;
    let mut build = cargo();
    build.args(["nextest", "list", "--workspace", "--cargo-profile", "ci", "--locked", "--list-type", "binaries-only", "--message-format", "json"]);
    let binaries_json = capture(build, "cargo nextest list (build all workspace tests)")?;
    let binaries_metadata = dir.join("binaries-metadata.json");
    // Retain both metadata files under target for diagnostics and reproducible listing.
    std::fs::write(&binaries_metadata, binaries_json).map_err(|e| format!("write {}: {e}", binaries_metadata.display()))?;
    Ok(Prepared { cargo_metadata, binaries_metadata })
}

fn capture(mut c: Command, label: &str) -> Result<Vec<u8>, String> {
    eprintln!("$ {label}");
    let out = c.stdin(Stdio::null()).stderr(Stdio::inherit()).output().map_err(|e| format!("{label}: {e}"))?;
    if out.status.success() { Ok(out.stdout) } else { Err(format!("{label}: {}", out.status)) }
}

/// Keep every gate running after failures, and stream native test names/times as they finish.
pub fn execute(native: (String, Command), side: Vec<(String, Command)>) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let mut failures = Vec::new();
    std::thread::scope(|scope| {
        let tx_native = tx.clone();
        scope.spawn(move || {
            let _ = tx_native.send(run_job(native));
        });
        scope.spawn(move || {
            for job in side {
                let _ = tx.send(run_job(job));
            }
        });
        for (label, result, seconds) in rx {
            match result {
                Ok(()) => eprintln!("ok    {label:<44} {seconds:6.1}s"),
                Err(e) => {
                    eprintln!("FAIL  {label:<44} {seconds:6.1}s  {e}");
                    failures.push(label);
                }
            }
        }
    });
    if failures.is_empty() { Ok(()) } else { Err(format!("failed: {}", failures.join(", "))) }
}

fn run_job((label, mut c): (String, Command)) -> (String, Result<(), String>, f64) {
    let started = Instant::now();
    let result = c.stdin(Stdio::null()).status().map_err(|e| e.to_string()).and_then(|s| if s.success() { Ok(()) } else { Err(s.to_string()) });
    (label, result, started.elapsed().as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn full_ci_does_not_drop_heavy_tests_stop_on_failure_or_retry_failures() {
        let p = Prepared { cargo_metadata: "cargo.json".into(), binaries_metadata: "binaries.json".into() };
        let c = p.command_with_threads(Some("3".into()));
        let args: Vec<_> = c.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(args.iter().any(|s| s == "--ignore-default-filter"));
        assert!(args.iter().any(|s| s == "--no-fail-fast"));
        assert!(args.windows(2).any(|s| s == ["--retries", "0"]));
        assert!(args.windows(2).any(|s| s == ["--no-tests", "fail"]));
        assert!(args.windows(2).any(|s| s == ["--test-threads", "3"]));
        assert!(!args.iter().any(|s| s == "--workspace" || s == "--cargo-profile"));
    }

    #[test]
    fn subprocess_fixture() {
        let Some(path) = std::env::var_os("SOLIDCRAFT_XTASK_TEST_MARKER") else { return };
        std::fs::write(path, b"ran").unwrap();
        assert_ne!(std::env::var("SOLIDCRAFT_XTASK_TEST_FAIL").ok().as_deref(), Some("1"), "intentional child-process failure");
    }

    fn fixture(path: &std::path::Path, fail: bool) -> Command {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args(["--exact", "test_nextest::tests::subprocess_fixture", "--test-threads=1"])
            .env("SOLIDCRAFT_XTASK_TEST_MARKER", path)
            .env("SOLIDCRAFT_XTASK_TEST_FAIL", if fail { "1" } else { "0" });
        c
    }

    #[test]
    fn a_native_failure_keeps_all_side_gates_running_and_fails_the_run() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("solidcraft-xtask-nextest-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let native = dir.join("native");
        let first = dir.join("first");
        let last = dir.join("last");
        let result = execute(
            ("native".into(), fixture(&native, true)),
            vec![("first".into(), fixture(&first, false)), ("last".into(), fixture(&last, false))],
        );
        assert_eq!(result, Err("failed: native".into()));
        assert!(native.is_file() && first.is_file() && last.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
