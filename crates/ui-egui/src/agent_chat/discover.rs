//! Which ACP agents are installed here (desktop only). A desktop app often starts without the
//! shell's PATH, so the usual install folders are searched too. Detection never starts a login
//! or a session: each agent found is asked for `--version` (with a time limit), and whether it
//! looks logged in comes only from whether its own credentials file exists.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// An agent SolidCraft knows how to start in ACP mode.
pub struct Known {
    pub id: &'static str,
    pub name: &'static str,
    /// Program names to look for, in order of preference.
    pub programs: &'static [&'static str],
    /// Arguments that put it in ACP mode.
    pub args: &'static [&'static str],
    /// Its one-line install hint.
    pub install: &'static str,
    /// Its own credentials file, relative to the home folder (`None`: can't tell), and the
    /// variable that moves its settings folder (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) with the file's
    /// name inside it.
    pub credentials: Option<&'static str>,
    pub config_dir_var: Option<(&'static str, &'static str)>,
    /// Variables holding a key it uses instead of a login.
    pub key_vars: &'static [&'static str],
    /// Retired by its maker: still used when it is installed, never offered for install.
    pub retired: bool,
    /// Whether `--version` prints its version. Antigravity's server prints its build date and takes
    /// seconds to start, so it is not asked.
    pub reports_version: bool,
}

/// The agents shown in the picker. The first one found is the default.
pub const KNOWN: &[Known] = &[
    Known {
        id: "claude",
        name: "Claude Code",
        programs: &["claude-agent-acp", "claude-code-acp"],
        args: &[],
        install: "npm install -g @agentclientprotocol/claude-agent-acp",
        credentials: Some(".claude/.credentials.json"),
        config_dir_var: Some(("CLAUDE_CONFIG_DIR", ".credentials.json")),
        key_vars: &["ANTHROPIC_API_KEY"],
        retired: false,
        reports_version: true,
    },
    Known {
        id: "codex",
        name: "Codex",
        programs: &["codex-acp"],
        args: &[],
        install: "npm install -g @agentclientprotocol/codex-acp",
        credentials: Some(".codex/auth.json"),
        config_dir_var: Some(("CODEX_HOME", "auth.json")),
        key_vars: &["OPENAI_API_KEY", "CODEX_API_KEY"],
        retired: false,
        reports_version: true,
    },
    // Google's `agy` command line has no ACP mode; its ACP server is a separate download that the
    // ACP registry lists (`agy_acp_server.par`, `agy_acp_server.exe` on Windows).
    Known {
        id: "antigravity",
        name: "Google Antigravity",
        programs: &["agy_acp_server.par", "agy_acp_server"],
        args: &[],
        install: "download the Antigravity ACP server for your system (see agentclientprotocol.com/registry), unzip it and put agy_acp_server.par on your PATH",
        credentials: Some(".gemini/antigravity-acp/acp_token.json"),
        config_dir_var: None,
        key_vars: &["GEMINI_API_KEY"],
        retired: false,
        reports_version: false,
    },
    // Google retired it on 18 June 2026 in favour of Antigravity (#1790). Enterprise licences may
    // still work, so an installed one is found, but nobody is sent to install it.
    Known {
        id: "gemini",
        name: "Gemini CLI",
        programs: &["gemini"],
        args: &["--acp"],
        install: "npm install -g @google/gemini-cli",
        credentials: Some(".gemini/oauth_creds.json"),
        config_dir_var: None,
        key_vars: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        retired: true,
        reports_version: true,
    },
    Known {
        id: "opencode",
        name: "OpenCode",
        programs: &["opencode"],
        args: &["acp"],
        install: "npm install -g opencode-ai",
        credentials: Some(".local/share/opencode/auth.json"),
        config_dir_var: None,
        key_vars: &[],
        retired: false,
        reports_version: true,
    },
    Known {
        id: "grok",
        name: "XAI Grok",
        programs: &["grok"],
        args: &["agent", "stdio"],
        install: "see https://docs.x.ai/build/overview (Grok Build installer); then run grok login",
        credentials: Some(".grok/auth.json"),
        config_dir_var: None,
        key_vars: &["XAI_API_KEY"],
        retired: false,
        reports_version: true,
    },
    Known {
        id: "copilot",
        name: "GitHub Copilot CLI",
        programs: &["copilot"],
        args: &["--acp"],
        install: "npm install -g @github/copilot",
        credentials: None,
        config_dir_var: None,
        key_vars: &["GH_TOKEN", "GITHUB_TOKEN"],
        retired: false,
        reports_version: true,
    },
    Known {
        id: "qwen",
        name: "Qwen Code",
        programs: &["qwen"],
        args: &["--acp"],
        install: "npm install -g @qwen-code/qwen-code",
        credentials: Some(".qwen/oauth_creds.json"),
        config_dir_var: None,
        key_vars: &["DASHSCOPE_API_KEY"],
        retired: false,
        reports_version: true,
    },
    Known {
        id: "goose",
        name: "goose",
        programs: &["goose"],
        args: &["acp"],
        install: "see block.github.io/goose (installer script or Homebrew)",
        credentials: None,
        config_dir_var: None,
        key_vars: &[],
        retired: false,
        reports_version: true,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Login {
    /// Its credentials file is there.
    LooksLoggedIn,
    /// No login file, but one of its key variables is set.
    KeyInEnvironment,
    /// Its credentials file is missing (it may still use a key from the environment).
    NotLoggedIn,
    /// It keeps no file we can look at (system keychain, or unknown).
    Unknown,
}

impl Login {
    pub fn id(self) -> &'static str {
        match self {
            Login::LooksLoggedIn => "signed_in",
            Login::KeyInEnvironment => "key",
            Login::NotLoggedIn => "signed_out",
            Login::Unknown => "unknown",
        }
    }
}

/// A known agent that isn't installed, with its one-line install hint.
#[derive(Clone, Debug)]
pub struct Missing {
    pub id: &'static str,
    pub name: &'static str,
    pub install: &'static str,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub id: &'static str,
    pub name: &'static str,
    /// The full command: the program's path and its ACP arguments.
    pub argv: Vec<String>,
    pub version: Option<String>,
    pub login: Login,
    /// The folder of the Node.js that runs it, when the first `node` on the search path is too
    /// old for it and a newer one was found further on ([`node_dir_for`]); `None`: the PATH as it is.
    pub node_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct Detection {
    pub found: Vec<Found>,
    /// Known agents not found.
    pub missing: Vec<Missing>,
    pub took_ms: u64,
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// PATH, then the usual places npm, Homebrew, nvm, Volta, bun, pipx and Grok put commands.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(prefix) = std::env::var_os("NPM_CONFIG_PREFIX").map(PathBuf::from) {
        dirs.push(if cfg!(windows) { prefix.clone() } else { prefix.join("bin") });
    }
    if let Some(h) = home() {
        for d in [".local/bin", ".npm-global/bin", ".npm-packages/bin", ".volta/bin", ".bun/bin", ".cargo/bin", ".grok/bin", "bin"] {
            dirs.push(h.join(d));
        }
        // nvm: every installed Node version's bin.
        if let Ok(rd) = std::fs::read_dir(h.join(".nvm/versions/node")) {
            let mut v: Vec<PathBuf> = rd.flatten().take(64).map(|e| e.path().join("bin")).collect();
            v.sort();
            dirs.extend(v.into_iter().rev());
        }
    }
    if cfg!(windows) {
        if let Some(a) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(a).join("npm"));
        }
    } else {
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin", "/usr/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| d.is_absolute() && seen.insert(d.clone()));
    dirs
}

/// The first `program` in `dirs` (on Windows with `.exe`/`.cmd`/`.bat`).
pub fn find_in(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let names = names_for(program, cfg!(windows));
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// The file names `program` can have. On Windows only ones it can run: npm puts a Unix shell
/// script with the bare name next to the `.cmd` shim, and starting that fails with "not a valid
/// Win32 application".
fn names_for(program: &str, windows: bool) -> Vec<String> {
    if windows { [".exe", ".cmd", ".bat"].iter().map(|e| format!("{program}{e}")).collect() } else { vec![program.to_string()] }
}

/// A Windows command saved without an extension (`…\npm\codex-acp`, from an older version's
/// lookup): the `.exe`/`.cmd`/`.bat` beside it, if there is one.
pub fn runnable_sibling(program: &Path) -> Option<PathBuf> {
    if program.extension().is_some() {
        return None;
    }
    let dir = program.parent()?;
    let name = program.file_name()?.to_str()?;
    names_for(name, true).into_iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Whether Node.js `major.minor` can run the npm ACP adapters. They load JSON with import
/// attributes (`import … with { type: "json" }`), which Node reads from 18.20, 20.10 and 21 on;
/// an older one stops at once with `SyntaxError: Unexpected token 'with'`.
fn node_runs_adapters((major, minor): (u32, u32)) -> bool {
    match major {
        18 => minor >= 20,
        20 => minor >= 10,
        m => m >= 21,
    }
}

/// `v18.19.1` (what `node --version` prints, and the `Node.js v18.19.1` line Node ends an
/// uncaught error with) as `(18, 19)`.
fn node_version_of(text: &str) -> Option<(u32, u32)> {
    let v = text.trim().strip_prefix("Node.js").unwrap_or(text).trim().strip_prefix('v')?;
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((major, minor))
}

/// Whether `program` is a script that asks `env` for `node` (`#!/usr/bin/env node`, as npm's
/// adapters are), so the PATH decides which Node runs it.
fn runs_on_env_node(program: &Path) -> bool {
    let mut head = [0u8; 256];
    let n = std::fs::File::open(program).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let first = head.get(..n).unwrap_or_default().split(|&b| b == b'\n').next().unwrap_or_default();
    let Some(line) = std::str::from_utf8(first).ok().and_then(|l| l.strip_prefix("#!")) else { return false };
    let mut words = line.split_whitespace();
    if !words.next().is_some_and(|w| w.ends_with("/env")) {
        return false;
    }
    // `env -S node --flag` and `env node` both name it as the first word that isn't an option.
    words.find(|w| !w.starts_with('-')).is_some_and(|w| w == "node")
}

/// One alternative of an npm `engines.node` range, by its lower bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeRange {
    /// `>=22`, `>20.10`, `>=18 <23` (the upper bound is not checked).
    AtLeast((u32, u32)),
    /// `^20.10`, `20`, `20.x`: that major version, from that minor on.
    SameMajor((u32, u32)),
    /// `~20.10`, `20.10.x`: that major and minor version.
    SameMinor((u32, u32)),
}

impl NodeRange {
    fn allows(self, v: (u32, u32)) -> bool {
        match self {
            NodeRange::AtLeast(min) => v >= min,
            NodeRange::SameMajor(min) => v.0 == min.0 && v >= min,
            NodeRange::SameMinor(min) => v == min,
        }
    }

    fn min(self) -> (u32, u32) {
        match self {
            NodeRange::AtLeast(m) | NodeRange::SameMajor(m) | NodeRange::SameMinor(m) => m,
        }
    }
}

/// An npm `engines.node` range (`>=22`, `^20.10.0 || >=22`) as its alternatives, or `None` when
/// any part is not understood: then only the syntax the adapters need is checked.
fn node_ranges_of(range: &str) -> Option<Vec<NodeRange>> {
    let number = |s: &str| -> Option<(u32, u32, bool)> {
        let mut parts = s.trim_start_matches('v').split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = match parts.next() {
            None | Some("x" | "X" | "*") => return Some((major, 0, false)),
            Some(m) => m.parse().ok()?,
        };
        Some((major, minor, true))
    };
    let mut out = Vec::new();
    for alt in range.split("||").take(8) {
        // The first comparator is the lower bound (`>=18 <23` → `>=18`); npm allows a space after
        // its operator (`>= 20.10.0`).
        let alt = alt.trim();
        let op_len = alt.len() - alt.trim_start_matches(['>', '=', '^', '~']).len();
        let (op, rest) = alt.split_at_checked(op_len)?;
        let version = rest.split_whitespace().next()?;
        let r = match op {
            ">=" | ">" => {
                let (a, b, _) = number(version)?;
                NodeRange::AtLeast((a, b))
            }
            "^" => {
                let (a, b, _) = number(version)?;
                NodeRange::SameMajor((a, b))
            }
            "~" => {
                let (a, b, _) = number(version)?;
                NodeRange::SameMinor((a, b))
            }
            "" | "=" => match number(version)? {
                (a, b, true) => NodeRange::SameMinor((a, b)),
                (a, _, false) => NodeRange::SameMajor((a, 0)),
            },
            _ => return None,
        };
        out.push(r);
    }
    (!out.is_empty()).then_some(out)
}

/// The `engines.node` range of the npm package `program` belongs to (`claude-agent-acp` asks for
/// `>=22`): the `package.json` in the nearest folder above the script it points to. Only files are
/// read; nothing is started.
fn node_ranges_for(program: &Path) -> Option<Vec<NodeRange>> {
    let script = std::fs::canonicalize(program).ok()?;
    let manifest = script.ancestors().skip(1).take(6).map(|d| d.join("package.json")).find(|p| p.is_file())?;
    if std::fs::metadata(&manifest).ok()?.len() > 1 << 20 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest).ok()?).ok()?;
    node_ranges_of(json.get("engines")?.get("node")?.as_str()?)
}

/// Whether Node `v` can run a program needing `ranges` (from [`node_ranges_for`]).
fn node_can_run(ranges: Option<&[NodeRange]>, v: (u32, u32)) -> bool {
    node_runs_adapters(v) && ranges.is_none_or(|r| r.iter().any(|a| a.allows(v)))
}

/// The lowest Node.js version that runs a program needing `ranges`, for the panel (`22`, `20.10`).
fn node_needed(ranges: Option<&[NodeRange]>) -> String {
    let min = ranges.and_then(|r| r.iter().map(|a| a.min()).min()).map_or((20, 10), |m| m.max((18, 20)));
    if min.1 == 0 { min.0.to_string() } else { format!("{}.{}", min.0, min.1) }
}

/// The Node.js version in an agent's last stderr lines and the version `program` needs, when the
/// one it ran on can't run it: the panel then says so instead of leaving only Node's own error.
pub(super) fn node_too_old_in(tail: &[String], program: &Path) -> Option<(String, String)> {
    let line = tail.iter().rev().map(|l| l.trim()).find(|l| l.starts_with("Node.js v"))?;
    let v = node_version_of(line)?;
    let ranges = node_ranges_for(program);
    (!node_can_run(ranges.as_deref(), v)).then(|| (line.trim_start_matches("Node.js ").to_string(), node_needed(ranges.as_deref())))
}

/// The most `node`s looked at for one program.
const MAX_NODES: usize = 16;
/// How long one `node --version` may take, and all of them in one detection together.
const NODE_PROBE: Duration = Duration::from_secs(2);
const NODE_PROBE_TOTAL: Duration = Duration::from_secs(4);

/// The folder to put first on `program`'s PATH: when `program` runs on `env node` and the first
/// `node` in `dirs` is too old for it (Ubuntu's packaged Node ahead of a newer one from nvm, say),
/// the folder of the first `node` that can run it. "Can run" is the syntax the adapters use and
/// the package's own `engines.node` (`claude-agent-acp` asks for 22). `None` when nothing needs to
/// move. `probe` reads a `node`'s version (`None`: unknown, so it is not chosen).
pub(super) fn node_dir_for(program: &Path, dirs: &[PathBuf], mut probe: impl FnMut(&Path) -> Option<(u32, u32)>) -> Option<PathBuf> {
    if !cfg!(unix) || !runs_on_env_node(program) {
        return None;
    }
    let ranges = node_ranges_for(program);
    let mut ok = |node: &Path| probe(node).is_some_and(|v| node_can_run(ranges.as_deref(), v));
    let mut nodes = dirs.iter().filter(|d| d.join("node").is_file()).take(MAX_NODES);
    let first = nodes.next()?;
    if ok(&first.join("node")) {
        return None;
    }
    nodes.find(|d| ok(&d.join("node"))).cloned()
}

/// `dirs` with `node_dir` (from [`node_dir_for`]) moved to the front; everything else keeps its order.
fn dirs_with(mut dirs: Vec<PathBuf>, node_dir: Option<&Path>) -> Vec<PathBuf> {
    if let Some(n) = node_dir {
        dirs.retain(|d| d != n);
        dirs.insert(0, n.to_path_buf());
    }
    dirs
}

/// Reads `node --version`s for one detection: each `node` at most once, each within
/// [`NODE_PROBE`], all within [`NODE_PROBE_TOTAL`] (after that, versions are unknown).
struct NodeProbe {
    deadline: Instant,
    seen: Vec<(PathBuf, Option<(u32, u32)>)>,
}

impl NodeProbe {
    fn new(total: Duration) -> NodeProbe {
        NodeProbe { deadline: Instant::now() + total, seen: Vec::new() }
    }

    fn version(&mut self, node: &Path) -> Option<(u32, u32)> {
        if let Some((_, v)) = self.seen.iter().find(|(p, _)| p == node) {
            return *v;
        }
        let left = self.deadline.saturating_duration_since(Instant::now()).min(NODE_PROBE);
        let v = if left.is_zero() {
            None
        } else {
            let mut cmd = Command::new(node);
            cmd.arg("--version");
            first_line(cmd, left).as_deref().and_then(node_version_of)
        };
        self.seen.push((node.to_path_buf(), v));
        v
    }
}

/// The Node folders [`detect`] chose, by program: [`command_for`] reads them and never starts a
/// `node` itself, as Connect calls it on the window thread.
static NODE_DIRS: std::sync::Mutex<Vec<(PathBuf, PathBuf)>> = std::sync::Mutex::new(Vec::new());

fn remember_node_dir(program: &Path, node_dir: Option<&Path>) {
    if let Ok(mut m) = NODE_DIRS.lock() {
        m.retain(|(p, _)| p != program);
        if let Some(n) = node_dir {
            m.push((program.to_path_buf(), n.to_path_buf()));
        }
    }
}

fn remembered_node_dir(program: &Path) -> Option<PathBuf> {
    NODE_DIRS.lock().ok()?.iter().find(|(p, _)| p == program).map(|(_, n)| n.clone())
}

/// Start probes and sessions with the same PATH used for discovery. Desktop launchers often
/// omit Node's install folder even when we found an adapter with a `#!/usr/bin/env node` shebang.
/// Only the child's environment changes; inherited absolute PATH entries keep precedence, except
/// that a Node too old to run the adapter gives way to the newer one [`detect`] chose. Nothing is
/// started here: this runs on the window thread.
pub(super) fn command_for(program: &Path) -> Command {
    // Older saved Windows commands may name npm's bare Unix script rather than its .cmd shim.
    let shim = if cfg!(windows) { runnable_sibling(program) } else { None };
    let program = shim.as_deref().unwrap_or(program);
    let mut cmd = if cfg!(windows) && program.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat")) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(program);
        c
    } else {
        Command::new(program)
    };
    if let Ok(path) = std::env::join_paths(dirs_with(search_dirs(), remembered_node_dir(program).as_deref())) {
        cmd.env("PATH", path);
    }
    cmd
}

/// `program --version`'s first line, or `None` after `limit` (the probe is then stopped).
fn version(program: &Path, limit: Duration) -> Option<String> {
    let mut cmd = command_for(program);
    cmd.arg("--version");
    first_line(cmd, limit).map(|line| version_of(&line))
}

/// The first non-empty line `cmd` prints, or `None` after `limit` (it is then stopped).
fn first_line(mut cmd: Command, limit: Duration) -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: probing a console agent doesn't flash a terminal window.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    out.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// The version number in a `--version` line (`codex-cli 0.162.1`, `2.1.295 (Claude Code)`,
/// `@scope/pkg 2.2.2`), else the line itself, cut short.
fn version_of(line: &str) -> String {
    let v = line.split_whitespace().find(|w| w.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit())).unwrap_or(line);
    v.chars().take(40).collect()
}

/// Whether `k` looks signed in, from files and variables only: nothing is started or asked.
fn login_of(k: &Known, home: Option<&Path>) -> Login {
    let file = match k.config_dir_var.and_then(|(var, name)| std::env::var_os(var).filter(|v| !v.is_empty()).map(|d| PathBuf::from(d).join(name))) {
        Some(p) => Some(p),
        None => k.credentials.zip(home).map(|(rel, h)| h.join(rel)),
    };
    let key = || k.key_vars.iter().any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()));
    match file {
        Some(f) if f.is_file() => Login::LooksLoggedIn,
        _ if key() => Login::KeyInEnvironment,
        // Claude Code keeps its login in the keychain on macOS: the file says nothing there.
        _ if cfg!(target_os = "macos") && k.id == "claude" => Login::Unknown,
        Some(_) => Login::NotLoggedIn,
        None => Login::Unknown,
    }
}

/// Look for every known agent. Blocking (each probe has a time limit): run it off the UI thread.
pub fn detect() -> Detection {
    detect_in(&search_dirs(), home().as_deref())
}

fn detect_in(dirs: &[PathBuf], home: Option<&Path>) -> Detection {
    let t0 = Instant::now();
    let mut d = Detection::default();
    let mut nodes = NodeProbe::new(NODE_PROBE_TOTAL);
    for k in KNOWN {
        let Some(path) = k.programs.iter().find_map(|p| find_in(p, dirs)) else {
            if !k.retired {
                d.missing.push(Missing { id: k.id, name: k.name, install: k.install });
            }
            continue;
        };
        let login = login_of(k, home);
        let mut argv = vec![path.to_string_lossy().into_owned()];
        argv.extend(k.args.iter().map(|a| a.to_string()));
        // Chosen before the version probe, which then runs on the same Node the session will.
        let node_dir = node_dir_for(&path, dirs, |n| nodes.version(n));
        remember_node_dir(&path, node_dir.as_deref());
        let version = if k.reports_version { version(&path, Duration::from_secs(3)) } else { None };
        d.found.push(Found { id: k.id, name: k.name, version, argv, login, node_dir });
    }
    d.took_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_found_in_the_first_folder_that_has_it() {
        let root = std::env::temp_dir().join(format!("sc-agent-find-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        // A name this platform runs: Windows looks only for `.exe`/`.cmd`/`.bat` (#1554).
        let name = names_for("codex-acp", cfg!(windows)).remove(0);
        std::fs::write(b.join(&name), "").unwrap();
        assert_eq!(find_in("codex-acp", &[a.clone(), b.clone()]), Some(b.join(name)));
        assert_eq!(find_in("gemini", &[a, b]), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn on_windows_only_runnable_shims_are_candidates() {
        // npm's bare `codex-acp` is a shell script; Windows can't start it (os error 193).
        assert_eq!(names_for("codex-acp", true), ["codex-acp.exe", "codex-acp.cmd", "codex-acp.bat"]);
        assert_eq!(names_for("codex-acp", false), ["codex-acp"]);
    }

    #[test]
    fn a_saved_bare_npm_path_runs_its_cmd_shim() {
        let root = std::env::temp_dir().join(format!("sc-agent-shim-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("codex-acp"), "#!/bin/sh").unwrap();
        std::fs::write(root.join("codex-acp.cmd"), "@echo off").unwrap();
        assert_eq!(runnable_sibling(&root.join("codex-acp")), Some(root.join("codex-acp.cmd")));
        assert_eq!(runnable_sibling(&root.join("codex-acp.cmd")), None);
        assert_eq!(runnable_sibling(&root.join("gemini")), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_version_number_is_picked_out_of_the_line() {
        assert_eq!(version_of("@agentclientprotocol/codex-acp 2.2.2"), "2.2.2");
        assert_eq!(version_of("2.1.295 (Claude Code)"), "2.1.295");
        assert_eq!(version_of("goose v1.9.0"), "v1.9.0");
        assert_eq!(version_of("grok 1.0.58 (26283d10dd58) [alpha]"), "1.0.58");
        assert_eq!(version_of("dev build"), "dev build");
    }

    #[test]
    fn node_versions_that_read_import_attributes() {
        assert_eq!(node_version_of("v18.19.1"), Some((18, 19)));
        assert_eq!(node_version_of("Node.js v22.11.0"), Some((22, 11)));
        assert_eq!(node_version_of("v20.10.0-nightly"), Some((20, 10)));
        assert_eq!(node_version_of("node: not found"), None);
        // Ubuntu 24.04's packaged Node (18.19) stops at `with`; 18.20, 20.10 and 21 on read it.
        for (v, ok) in [((18, 19), false), ((18, 20), true), ((19, 9), false), ((20, 9), false), ((20, 10), true), ((21, 0), true), ((24, 1), true)] {
            assert_eq!(node_runs_adapters(v), ok, "{v:?}");
        }
    }

    #[test]
    fn an_agents_stderr_tells_when_its_node_is_too_old() {
        let tail = |last: &str| {
            ["SyntaxError: Unexpected token 'with'", "    at ModuleLoader.moduleStrategy (node:internal/modules/esm/translators:152:18)", "", last]
                .map(String::from)
        };
        let unknown = Path::new("/nonexistent/claude-agent-acp");
        let old = |v: &str, need: &str| Some((v.to_string(), need.to_string()));
        assert_eq!(node_too_old_in(&tail("Node.js v18.19.1"), unknown), old("v18.19.1", "20.10"));
        assert_eq!(node_too_old_in(&tail("Node.js v22.11.0"), unknown), None, "a new enough Node failed for another reason");
        assert_eq!(node_too_old_in(&tail("error: not signed in"), unknown), None);
        assert_eq!(node_too_old_in(&[], unknown), None);

        // An adapter whose package asks for Node 22: 20.11 reads the syntax but is still too old.
        let root = std::env::temp_dir().join(format!("sc-agent-node-engines-{}", std::process::id()));
        let pkg = root.join("lib/node_modules/@agentclientprotocol/claude-agent-acp");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{"name":"claude-agent-acp","engines":{"node":">=22"}}"#).unwrap();
        std::fs::write(pkg.join("dist/index.js"), "#!/usr/bin/env node\n").unwrap();
        let script = pkg.join("dist/index.js");
        assert_eq!(node_too_old_in(&tail("Node.js v20.11.1"), &script), old("v20.11.1", "22"));
        assert_eq!(node_too_old_in(&tail("Node.js v22.11.0"), &script), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn engines_ranges_read_by_their_lower_bound() {
        use NodeRange::*;
        assert_eq!(node_ranges_of(">=22"), Some(vec![AtLeast((22, 0))]));
        assert_eq!(node_ranges_of(">= 20.10.0"), Some(vec![AtLeast((20, 10))]));
        assert_eq!(node_ranges_of("<23"), None, "no lower bound");
        assert_eq!(node_ranges_of(">=18 <23"), Some(vec![AtLeast((18, 0))]));
        assert_eq!(node_ranges_of("^20.10.0 || >=22"), Some(vec![SameMajor((20, 10)), AtLeast((22, 0))]));
        assert_eq!(node_ranges_of("~20.10 || 22.x"), Some(vec![SameMinor((20, 10)), SameMajor((22, 0))]));
        assert_eq!(node_ranges_of("lts/*"), None);
        assert_eq!(node_ranges_of(""), None);

        let need = node_ranges_of("^20.10.0 || >=22");
        for (v, ok) in [((20, 9), false), ((20, 12), true), ((21, 0), false), ((22, 1), true), ((18, 20), false)] {
            assert_eq!(node_can_run(need.as_deref(), v), ok, "{v:?}");
        }
        // No range known: only the syntax decides.
        assert!(node_can_run(None, (18, 20)) && !node_can_run(None, (18, 19)));
        // A range below what the syntax needs never lets an older Node through.
        assert!(!node_can_run(node_ranges_of(">=16").as_deref(), (18, 19)));
        assert_eq!(node_needed(node_ranges_of(">=22").as_deref()), "22");
        assert_eq!(node_needed(node_ranges_of("^20.10.0 || >=22").as_deref()), "20.10");
        assert_eq!(node_needed(node_ranges_of(">=16").as_deref()), "18.20");
        assert_eq!(node_needed(None), "20.10");
    }

    /// Ubuntu's packaged Node 18.19 in /usr/bin comes before nvm's folders in the desktop PATH,
    /// and an adapter with `#!/usr/bin/env node` would run on it and stop at its first import.
    /// The first `node` that can run it is moved to the front; nothing else changes.
    #[test]
    #[cfg(unix)]
    fn an_env_node_adapter_skips_a_node_too_old_for_it() {
        let root = std::env::temp_dir().join(format!("sc-agent-node-{}", std::process::id()));
        let (old, other, new, bin) = (root.join("usr-bin"), root.join("other"), root.join("nvm-22"), root.join("adapters"));
        for d in [&old, &other, &new, &bin] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(old.join("node"), "18").unwrap();
        std::fs::write(new.join("node"), "22").unwrap();
        let adapter = bin.join("claude-agent-acp");
        std::fs::write(&adapter, "#!/usr/bin/env node\nimport pkg from './package.json' with { type: 'json' };\n").unwrap();
        let native = bin.join("goose");
        std::fs::write(&native, "\x7fELF").unwrap();
        // The fake `node`s hold their major version; nothing is started.
        let probe = |p: &Path| std::fs::read_to_string(p).ok().and_then(|s| s.parse().ok()).map(|m| (m, if m == 18 { 19 } else { 11 }));
        let dirs = vec![old.clone(), other.clone(), new.clone()];

        let chosen = node_dir_for(&adapter, &dirs, probe);
        assert_eq!(chosen.as_deref(), Some(new.as_path()));
        assert_eq!(dirs_with(dirs.clone(), chosen.as_deref()), [new.clone(), old.clone(), other.clone()]);
        // A new enough first Node, a program that isn't a Node script, or no better Node: unchanged.
        assert_eq!(node_dir_for(&adapter, &[new.clone(), old.clone()], probe), None);
        assert_eq!(node_dir_for(&native, &dirs, probe), None);
        assert_eq!(node_dir_for(&adapter, &[old.clone(), other.clone()], probe), None);
        assert_eq!(dirs_with(dirs.clone(), None), dirs);

        // The package asks for Node 22 (as claude-agent-acp does): a 20.11 first on the PATH reads
        // the syntax but is still passed over for the 22 further on.
        let pkg = root.join("lib/node_modules/claude-agent-acp");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{"engines":{"node":">=22"}}"#).unwrap();
        std::fs::write(pkg.join("dist/index.js"), "#!/usr/bin/env node\n").unwrap();
        let mid = root.join("usr-bin-20");
        std::fs::create_dir_all(&mid).unwrap();
        std::fs::write(mid.join("node"), "20").unwrap();
        let script = pkg.join("dist/index.js");
        assert_eq!(node_dir_for(&script, &[mid.clone(), new.clone()], probe).as_deref(), Some(new.as_path()));
        assert_eq!(node_dir_for(&adapter, &[mid.clone(), new.clone()], probe), None, "without a package range 20.11 is enough");

        std::fs::write(&adapter, "#!/usr/bin/env -S node --no-warnings\n").unwrap();
        assert!(runs_on_env_node(&adapter));
        std::fs::write(&adapter, "#!/bin/sh\nexec node \"$0\"\n").unwrap();
        assert!(!runs_on_env_node(&adapter), "a shell script picks its Node itself");
        assert!(!runs_on_env_node(&root.join("missing")));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Review of #2208: Connect calls `command_for` on the window thread, so it must not start a
    /// `node`. It uses only the folder detection chose, and a program detection never saw keeps
    /// the search path as it is.
    #[test]
    #[cfg(unix)]
    fn starting_an_agent_uses_the_node_detection_chose_and_starts_none() {
        let root = std::env::temp_dir().join(format!("sc-agent-node-cache-{}", std::process::id()));
        let (new, bin) = (root.join("nvm-22"), root.join("adapters"));
        std::fs::create_dir_all(&new).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let adapter = bin.join("claude-agent-acp");
        std::fs::write(&adapter, "#!/usr/bin/env node\n").unwrap();
        let path_of = |cmd: &Command| {
            cmd.get_envs().find(|(k, _)| *k == "PATH").and_then(|(_, v)| v).map(|v| std::env::split_paths(v).collect::<Vec<_>>()).unwrap_or_default()
        };

        assert_eq!(path_of(&command_for(&adapter)), search_dirs(), "nothing chosen yet: the search path as it is");
        remember_node_dir(&adapter, Some(&new));
        assert_eq!(path_of(&command_for(&adapter)).first(), Some(&new));
        assert_eq!(path_of(&command_for(&bin.join("codex-acp"))), search_dirs(), "chosen for one program only");
        remember_node_dir(&adapter, None);
        assert_eq!(path_of(&command_for(&adapter)), search_dirs(), "a later detection that found the PATH fine clears it");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Review of #2208: hanging `node`s can't hold detection up for long. Each is asked once, for
    /// at most two seconds, and all of them together within the total.
    #[test]
    #[cfg(unix)]
    fn hanging_nodes_are_probed_within_the_total_time() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("sc-agent-node-hang-{}", std::process::id()));
        let mut dirs = Vec::new();
        for i in 0..MAX_NODES {
            let d = root.join(format!("n{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("node"), "#!/bin/sh\nexec sleep 30\n").unwrap();
            std::fs::set_permissions(d.join("node"), std::fs::Permissions::from_mode(0o755)).unwrap();
            dirs.push(d);
        }
        let adapter = root.join("claude-agent-acp");
        std::fs::write(&adapter, "#!/usr/bin/env node\n").unwrap();

        let mut probe = NodeProbe::new(Duration::from_millis(600));
        let t0 = Instant::now();
        assert_eq!(node_dir_for(&adapter, &dirs, |n| probe.version(n)), None);
        // A second adapter in the same detection asks none of them again.
        assert_eq!(node_dir_for(&adapter, &dirs, |n| probe.version(n)), None);
        let took = t0.elapsed();
        assert!(took < Duration::from_secs(3), "16 hanging nodes took {took:?}");
        assert_eq!(probe.seen.len(), MAX_NODES, "each node asked once");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn signed_in_comes_from_the_agents_own_file_or_a_key_variable() {
        let home = std::env::temp_dir().join(format!("sc-agent-login-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".gemini")).unwrap();
        let gemini = KNOWN.iter().find(|k| k.id == "gemini").unwrap();
        let qwen = KNOWN.iter().find(|k| k.id == "qwen").unwrap();
        let goose = KNOWN.iter().find(|k| k.id == "goose").unwrap();
        let grok = KNOWN.iter().find(|k| k.id == "grok").unwrap();
        if std::env::var_os("GEMINI_API_KEY").is_none() && std::env::var_os("GOOGLE_API_KEY").is_none() {
            assert_eq!(login_of(gemini, Some(&home)), Login::NotLoggedIn);
        }
        std::fs::write(home.join(".gemini/oauth_creds.json"), "{}").unwrap();
        assert_eq!(login_of(gemini, Some(&home)), Login::LooksLoggedIn);
        if std::env::var_os("DASHSCOPE_API_KEY").is_none() {
            assert_eq!(login_of(qwen, Some(&home)), Login::NotLoggedIn);
        }
        assert_eq!(login_of(goose, Some(&home)), Login::Unknown, "keeps no file we know");
        if std::env::var_os("XAI_API_KEY").is_none() {
            assert_eq!(login_of(grok, Some(&home)), Login::NotLoggedIn);
        }
        // Antigravity's server keeps its own login, apart from the retired Gemini CLI's.
        let agy = KNOWN.iter().find(|k| k.id == "antigravity").unwrap();
        if std::env::var_os("GEMINI_API_KEY").is_none() {
            assert_eq!(login_of(agy, Some(&home)), Login::NotLoggedIn, "the Gemini CLI's file is not its login");
        }
        std::fs::create_dir_all(home.join(".gemini/antigravity-acp")).unwrap();
        std::fs::write(home.join(".gemini/antigravity-acp/acp_token.json"), "not credential data").unwrap();
        assert_eq!(login_of(agy, Some(&home)), Login::LooksLoggedIn);
        std::fs::create_dir_all(home.join(".grok")).unwrap();
        // Discovery checks only for the file; it never needs to parse the user's tokens.
        std::fs::write(home.join(".grok/auth.json"), "not credential data").unwrap();
        assert_eq!(login_of(grok, Some(&home)), Login::LooksLoggedIn);
        let _ = std::fs::remove_dir_all(home);
    }

    /// Google retired the Gemini CLI (#1790): one that is installed is still found, but nobody is
    /// told to install it. Antigravity's ACP server is offered, and found under its file name.
    #[test]
    #[cfg(unix)]
    fn a_retired_agent_is_found_but_not_offered_and_antigravity_is() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("sc-agent-retired-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let run = |name: &str, body: &str| {
            let path = root.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let ids = |d: &Detection| (d.found.iter().map(|f| f.id).collect::<Vec<_>>(), d.missing.iter().map(|m| m.id).collect::<Vec<_>>());

        let none = detect_in(std::slice::from_ref(&root), None);
        let (found, missing) = ids(&none);
        assert!(found.is_empty());
        assert!(missing.contains(&"antigravity") && !missing.contains(&"gemini"), "{missing:?}");
        let hint = none.missing.iter().find(|m| m.id == "antigravity").map(|m| m.install).unwrap_or_default();
        assert!(hint.contains("agy_acp_server.par") && !hint.contains("npm"), "{hint}");

        run("gemini", "echo gemini 0.64.0");
        // A server that would take seconds to answer `--version`, which Antigravity is never asked.
        run("agy_acp_server.par", "sleep 30");
        let both = detect_in(std::slice::from_ref(&root), None);
        let (found, missing) = ids(&both);
        assert_eq!(found, ["antigravity", "gemini"]);
        assert!(!missing.contains(&"antigravity") && !missing.contains(&"gemini"), "{missing:?}");
        let agy = both.found.iter().find(|f| f.id == "antigravity");
        assert_eq!(agy.map(|f| f.version.clone()), Some(None), "its --version is a build date, not asked");
        assert_eq!(agy.and_then(|f| f.argv.get(1)), None, "the registry starts it without arguments");
        assert!(both.took_ms < 5000, "Antigravity's server was started to read a version");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn every_known_agent_has_a_program_and_an_install_hint() {
        for k in KNOWN {
            assert!(!k.programs.is_empty() && !k.install.is_empty(), "{}", k.id);
        }
        assert_eq!(KNOWN.first().map(|k| k.id), Some("claude"), "the Claude Code adapter is the default");
    }

    /// Finder's PATH lacks user-installed runtimes. Discovery, the version probe and a live
    /// agent must all work with an env shebang whose interpreter is in ~/.local/bin.
    #[test]
    #[cfg(unix)]
    fn desktop_path_can_start_an_agent_and_its_runtime() {
        use super::super::process::{AgentProcess, FromAgent};
        use std::os::unix::fs::PermissionsExt;

        const CHILD: &str = "SOLIDCRAFT_AGENT_PATH_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let agent = find_in("solidcraft-test-agent", &search_dirs()).unwrap();
            assert_eq!(version(&agent, Duration::from_secs(3)).as_deref(), Some("1.2.3"));
            let mut p = AgentProcess::spawn(&[agent.to_string_lossy().into_owned()], &root, &[], || {}).unwrap();
            let message = serde_json::json!({"id": 1, "method": "initialize"});
            p.send(&message);
            let reply = p.from_agent.recv_timeout(Duration::from_secs(5)).unwrap();
            p.shutdown(true);
            match reply {
                FromAgent::Line(line) => assert_eq!(serde_json::from_str::<serde_json::Value>(&line).unwrap(), message),
                FromAgent::Exited(tail) => panic!("the agent could not start its runtime: {tail:?}"),
            }
            return;
        }

        let root = std::env::temp_dir().join(format!("sc-agent-desktop-path-{}", std::process::id()));
        let bin = root.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        for (name, script) in [
            ("solidcraft-test-agent", "#!/usr/bin/env solidcraft-test-runtime\n"),
            ("solidcraft-test-runtime", "#!/bin/sh\nif [ \"$2\" = \"--version\" ]; then printf 'test-agent 1.2.3\\n'; else cat; fi\n"),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Change only a child process's environment: no global mutation or unsafe set_var.
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "agent_chat::discover::tests::desktop_path_can_start_an_agent_and_its_runtime", "--nocapture"])
            .env(CHILD, &root)
            .env("HOME", &root)
            .env("PATH", "/usr/bin:/bin")
            .env_remove("NPM_CONFIG_PREFIX")
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(root);
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}
