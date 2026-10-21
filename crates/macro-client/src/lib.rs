//! Drive a SolidCraft session from a user Rust macro.
//!
//! The CAD kernel is not linked here. Every change goes through the same command ids the
//! toolbar, JSON macros, CLI and MCP use (`app.run("solid.box", json!({…}))`).
//!
//! - [`App::connect`] — live window (`solidcraft --control PORT`, `SOLIDCRAFT_CONTROL_PORT`)
//! - [`App::headless`] — spawn `solidcraft-cli mcp` (no window)

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

/// A connection to a live app or a headless MCP session.
pub struct App {
    inner: Inner,
}

enum Inner {
    Control { reader: BufReader<TcpStream>, writer: TcpStream, next_id: u64, token: Option<String> },
    Mcp { child: Child, reader: BufReader<std::process::ChildStdout>, writer: std::process::ChildStdin, next_id: u64 },
}

/// A command or protocol failure.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(String);

impl Error {
    fn msg(s: impl Into<String>) -> Self {
        Error(s.into())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl App {
    /// Connect to a running SolidCraft window (`SOLIDCRAFT_CONTROL_PORT`, token from the env or the app's token file).
    pub fn connect() -> Result<Self> {
        let port = std::env::var("SOLIDCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse::<u16>().ok()).filter(|p| *p != 0).ok_or_else(|| {
            Error::msg("SOLIDCRAFT_CONTROL_PORT is not set (Play a binary from the MACRO tab, or start with `solidcraft --control PORT`)")
        })?;
        Self::connect_addr(&format!("127.0.0.1:{port}"))
    }

    /// Connect to `host:port` on the control channel.
    pub fn connect_addr(addr: &str) -> Result<Self> {
        Self::connect_addr_token(addr, None)
    }

    /// [`connect_addr`] with an explicit token (tests; otherwise the env or the app's token file).
    pub fn connect_addr_token(addr: &str, token: Option<&str>) -> Result<Self> {
        let port = addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(0);
        let stream = TcpStream::connect(addr).map_err(|e| Error::msg(format!("cannot reach SolidCraft at {addr}: {e}")))?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(Duration::from_secs(300))).ok();
        let reader = BufReader::new(stream.try_clone()?);
        let token = token.map(str::to_string).or_else(|| control_token(port));
        Ok(App { inner: Inner::Control { reader, writer: stream, next_id: 1, token } })
    }

    /// Spawn `solidcraft-cli mcp` (or `SOLIDCRAFT_CLI`) and talk MCP on stdio.
    pub fn headless() -> Result<Self> {
        Self::headless_cli(cli_path())
    }

    /// [`App::headless`] with an explicit CLI binary.
    pub fn headless_cli(cli: impl AsRef<Path>) -> Result<Self> {
        let cli = cli.as_ref();
        let mut child = Command::new(cli)
            .args(["mcp", "--allow-any-path"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Error::msg(format!("{}: {e} (build solidcraft-cli or set SOLIDCRAFT_CLI)", cli.display())))?;
        let writer = child.stdin.take().ok_or_else(|| Error::msg("mcp: no stdin"))?;
        let reader = BufReader::new(child.stdout.take().ok_or_else(|| Error::msg("mcp: no stdout"))?);
        let mut app = App { inner: Inner::Mcp { child, reader, writer, next_id: 1 } };
        app.mcp_rpc(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "solidcraft-macro-client", "version": env!("CARGO_PKG_VERSION")}}),
        )?;
        app.mcp_notify("notifications/initialized")?;
        Ok(app)
    }

    /// Run one engine command. `params` is the JSON object the command takes (or `null`).
    pub fn run(&mut self, command: &str, params: Value) -> Result<Value> {
        match &mut self.inner {
            Inner::Control { .. } => self.control("engine.execute", json!({"command": command, "params": params})),
            Inner::Mcp { .. } => {
                let raw =
                    self.mcp_rpc("tools/call", json!({"name": "execute", "arguments": {"command": command, "params": params, "verbose": true}}))?;
                mcp_tool_value(&raw)
            }
        }
    }

    /// The open design (`document.inspect` with measurements).
    pub fn inspect(&mut self) -> Result<Value> {
        self.run("document.inspect", json!({"measure": true}))
    }

    /// Run `f` as one undoable unit: restore the design if it returns `Err`.
    pub fn transaction<R>(&mut self, f: impl FnOnce(&mut Self) -> Result<R>) -> Result<R> {
        let mark = self.run("edit.checkpoint", json!({}))?;
        let id = mark.get("checkpoint").cloned().unwrap_or(Value::Null);
        match f(self) {
            Ok(r) => {
                let _ = self.run("edit.restore_checkpoint", json!({"checkpoint": id, "forget": true}));
                Ok(r)
            }
            Err(e) => {
                let _ = self.run("edit.restore_checkpoint", json!({"checkpoint": id}));
                Err(e)
            }
        }
    }

    fn control(&mut self, method: &str, params: Value) -> Result<Value> {
        let Inner::Control { reader, writer, next_id, token } = &mut self.inner else {
            return Err(Error::msg("not a control connection"));
        };
        let id = *next_id;
        *next_id += 1;
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(t) = token {
            msg["token"] = json!(t);
        }
        writer.write_all(msg.to_string().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        let mut reply = String::new();
        reader.by_ref().take(64 << 20).read_line(&mut reply)?;
        if reply.is_empty() {
            return Err(Error::msg("the SolidCraft app closed the connection"));
        }
        let v: Value = serde_json::from_str(&reply)?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(Error::msg(v.get("error").and_then(Value::as_str).unwrap_or("error")))
        }
    }

    fn mcp_rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let Inner::Mcp { reader, writer, next_id, .. } = &mut self.inner else {
            return Err(Error::msg("not an MCP session"));
        };
        let id = *next_id;
        *next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writer.write_all(msg.to_string().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line.is_empty() {
                return Err(Error::msg("solidcraft-cli mcp exited"));
            }
            let v: Value = serde_json::from_str(&line)?;
            if v.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(Error::msg(err.get("message").and_then(Value::as_str).unwrap_or("mcp error")));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn mcp_notify(&mut self, method: &str) -> Result<()> {
        let Inner::Mcp { writer, .. } = &mut self.inner else {
            return Err(Error::msg("not an MCP session"));
        };
        let msg = json!({"jsonrpc": "2.0", "method": method});
        writer.write_all(msg.to_string().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Inner::Mcp { child, .. } = &mut self.inner {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn mcp_tool_value(raw: &Value) -> Result<Value> {
    let err = raw.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let text: String = raw
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect();
    if err {
        return Err(Error::msg(text));
    }
    match serde_json::from_str(&text) {
        Ok(v) => Ok(v),
        Err(_) => Ok(json!(text)),
    }
}

fn cli_path() -> PathBuf {
    if let Ok(p) = std::env::var("SOLIDCRAFT_CLI") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let cand = dir.join("solidcraft-cli");
        if cand.is_file() {
            return cand;
        }
        #[cfg(windows)]
        {
            let cand = dir.join("solidcraft-cli.exe");
            if cand.is_file() {
                return cand;
            }
        }
    }
    PathBuf::from("solidcraft-cli")
}

fn control_token(port: u16) -> Option<String> {
    if let Ok(t) = std::env::var("SOLIDCRAFT_CONTROL_TOKEN") {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Some(t);
        }
    }
    let dir = if let Ok(d) = std::env::var("SOLIDCRAFT_CONTROL_DIR") {
        PathBuf::from(d)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("SolidCraft/control")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/SolidCraft/control")
    } else {
        let data =
            std::env::var("XDG_DATA_HOME").ok().map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share")))?;
        data.join("solidcraft/control")
    };
    std::fs::read_to_string(dir.join(format!("{port}.token"))).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn connect_runs_a_command_over_json_lines() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let token = "a".repeat(32);
        let token_for_server = token.clone();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1];
            while s.read(&mut tmp).unwrap() == 1 {
                buf.push(tmp[0]);
                if tmp[0] == b'\n' {
                    break;
                }
            }
            let req: Value = serde_json::from_slice(&buf).unwrap();
            assert_eq!(req["method"], "engine.execute");
            assert_eq!(req["params"]["command"], "solid.box");
            assert_eq!(req["token"], token_for_server);
            s.write_all(br#"{"id":1,"ok":true,"result":{"feature":1}}"#).unwrap();
            s.write_all(b"\n").unwrap();
        });
        let mut app = App::connect_addr_token(&addr.to_string(), Some(&token)).unwrap();
        let v = app.run("solid.box", json!({"length": 10, "width": 10, "height": 10})).unwrap();
        assert_eq!(v["feature"], 1);
        server.join().unwrap();
    }
}
