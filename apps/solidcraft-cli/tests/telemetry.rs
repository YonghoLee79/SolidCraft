//! Exercise the real host lifecycle and OTLP transport without requiring a collector install.
#![cfg(feature = "telemetry")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::time::Duration;

use serde_json::{Value, json};

fn cli(endpoint: &str, enabled: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"));
    // Each child has its own provider and environment; parallel tests do not share globals.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("OTEL_") || key.to_string_lossy().starts_with("SOLIDCRAFT_OTEL") {
            command.env_remove(key);
        }
    }
    command.env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint).env("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json");
    if enabled {
        command.env("SOLIDCRAFT_OTEL", "1").env("SOLIDCRAFT_OTEL_RUN_ID", "regression-before");
    }
    command
}

fn receiver() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    (listener, endpoint)
}

fn receive(listener: TcpListener, done: std::sync::mpsc::Receiver<()>) -> Value {
    let mut resources = Vec::new();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let payload = read_request(stream);
                resources.extend(payload["resourceSpans"].as_array().unwrap().iter().cloned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if matches!(done.try_recv(), Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected)) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("OTLP request was not received: {error}"),
        }
    }
    assert!(!resources.is_empty(), "CLI exited without exporting spans");
    json!({"resourceSpans": resources})
}

fn read_request(mut stream: TcpStream) -> Value {
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.starts_with("POST /v1/traces "), "{line}");
    let mut length = None;
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let length = length.unwrap();
    assert!(length < 1_000_000);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let payload = serde_json::from_slice(&body).unwrap();
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
    payload
}

fn export(id: &str, params: Value, crash_test: bool) -> (Output, Value) {
    let (listener, endpoint) = receiver();
    let (done, signal) = std::sync::mpsc::channel();
    let receiver = std::thread::spawn(move || receive(listener, signal));
    let mut command = cli(&endpoint, true);
    let crash_dir = std::env::temp_dir().join(format!("solidcraft-telemetry-crashes-{}", std::process::id()));
    if crash_test {
        std::fs::create_dir(&crash_dir).unwrap();
        command.env("SOLIDCRAFT_CRASH_TEST", "1").env("SOLIDCRAFT_CRASH_DIR", &crash_dir);
    }
    let output = command.args(["exec", id, &params.to_string()]).output().unwrap();
    if crash_test {
        std::fs::remove_dir_all(crash_dir).unwrap();
    }
    done.send(()).unwrap();
    (output, receiver.join().unwrap())
}

fn export_script(script: Value) -> (Output, Value) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let index = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("solidcraft-telemetry-{}-{index}.json", std::process::id()));
    std::fs::write(&path, script.to_string()).unwrap();
    let (listener, endpoint) = receiver();
    let (done, signal) = std::sync::mpsc::channel();
    let receiver = std::thread::spawn(move || receive(listener, signal));
    let output = cli(&endpoint, true).arg("run").arg(&path).arg("--results").output().unwrap();
    std::fs::remove_file(path).unwrap();
    done.send(()).unwrap();
    (output, receiver.join().unwrap())
}

fn spans(payload: &Value) -> Vec<&Value> {
    payload["resourceSpans"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|resource| resource["scopeSpans"].as_array().unwrap())
        .flat_map(|scope| scope["spans"].as_array().unwrap())
        .collect()
}

fn attribute(record: &Value, key: &str) -> Value {
    record["attributes"].as_array().unwrap().iter().find(|a| a["key"] == key).map(|a| a["value"].clone()).unwrap_or(Value::Null)
}

fn command_span(payload: &Value) -> &Value {
    spans(payload).into_iter().find(|s| s["name"] == "solidcraft.command").unwrap()
}

#[test]
fn export_is_off_even_with_an_otlp_endpoint_unless_explicitly_enabled() {
    let (listener, endpoint) = receiver();
    let output = cli(&endpoint, false).args(["exec", "solid.box", r#"{"length":10,"width":20,"height":30}"#]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn normal_cli_exit_flushes_commands_and_their_evaluation_children() {
    let (output, payload) = export("solid.box", json!({"length":10,"width":20,"height":30,"body_name": "private-design-name"}), false);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let command = command_span(&payload);
    assert_eq!(attribute(command, "solidcraft.command.id"), json!({"stringValue": "solid.box"}));
    assert_eq!(command["status"]["code"], 1);
    assert!(attribute(command, "solidcraft.kernel.tessellate_ms")["doubleValue"].as_f64().unwrap() >= 0.0);
    let all = spans(&payload);
    let child = all.iter().find(|s| s["name"] == "solidcraft.evaluate" && s["parentSpanId"] == command["spanId"]).unwrap();
    assert_eq!(child["traceId"], command["traceId"]);
    assert_eq!(attribute(child, "solidcraft.model.error_count"), json!({"intValue": "0"}));
    let resource = &payload["resourceSpans"][0]["resource"];
    assert_eq!(attribute(resource, "service.name"), json!({"stringValue": "solidcraft-cli"}));
    assert_eq!(attribute(resource, "solidcraft.run.id"), json!({"stringValue": "regression-before"}));
    assert!(!payload.to_string().contains("private-design-name"));
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_ok(), "stdout remains machine-readable");
}

#[test]
fn boolean_geometry_and_attempts_are_correlated_with_the_command() {
    let (output, payload) = export_script(json!([
        {"command":"solid.box","params":{"length":10,"width":10,"height":10,"body_name":"Block"}},
        {"command":"solid.cylinder","params":{"base":[0,5,0],"axis":[1,0,1],"radius":2,"height":14,"body_name":"Tool"}},
        {"command":"solid.combine","params":{"target":"Block","tools":["Tool"],"operation":"cut"}}
    ]));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let all = spans(&payload);
    let command = all.iter().find(|s| attribute(s, "solidcraft.command.id")["stringValue"] == "solid.combine").unwrap();
    let boolean = all.iter().find(|s| s["name"] == "solidcraft.kernel.boolean" && s["traceId"] == command["traceId"]).unwrap();
    let mesh = all.iter().find(|s| s["name"] == "solidcraft.kernel.tessellate" && s["traceId"] == command["traceId"]).unwrap();
    assert!(attribute(mesh, "solidcraft.mesh.triangles")["intValue"].as_str().unwrap().parse::<usize>().unwrap() > 0);
    assert_eq!(attribute(boolean, "solidcraft.boolean.operation")["stringValue"], "cut");
    assert_eq!(boolean["status"]["code"], 1);
    let attempt = all.iter().find(|s| s["name"] == "solidcraft.kernel.boolean.attempt" && s["parentSpanId"] == boolean["spanId"]).unwrap();
    assert!(attribute(attempt, "solidcraft.boolean.tolerance_mm")["doubleValue"].as_f64().unwrap() > 0.0);
    let started = all.iter().find(|s| s["name"] == "solidcraft.kernel.started" && s["parentSpanId"] == attempt["spanId"]).unwrap();
    assert!(
        started["endTimeUnixNano"].as_str().unwrap().parse::<u64>().unwrap() <= attempt["endTimeUnixNano"].as_str().unwrap().parse::<u64>().unwrap()
    );
    assert!(boolean["events"].as_array().unwrap().iter().any(|e| e["name"] == "solidcraft.boolean.candidate"));
}

#[test]
fn rejected_parameters_export_an_error_without_the_parameter_value() {
    let (output, payload) = export("solid.box", json!({"length": "private-invalid-length", "width":20,"height":30}), false);
    assert!(!output.status.success());
    let command = command_span(&payload);
    assert_eq!(command["status"]["code"], 2);
    assert_eq!(attribute(command, "error.type"), json!({"stringValue": "bad_params"}));
    assert!(!payload.to_string().contains("private-invalid-length"));
}

#[test]
fn unknown_commands_do_not_export_the_user_supplied_id() {
    let (output, payload) = export("private-unknown-command", json!({}), false);
    assert!(!output.status.success());
    let command = command_span(&payload);
    assert_eq!(attribute(command, "solidcraft.command.id"), json!({"stringValue": "unknown"}));
    assert_eq!(attribute(command, "error.type"), json!({"stringValue": "unknown_command"}));
    assert!(!payload.to_string().contains("private-unknown-command"));
}

#[test]
fn caught_command_panics_are_exported_as_internal_errors() {
    let (output, payload) = export("diagnostics.crash_test", json!({"mode": "panic"}), true);
    assert!(!output.status.success());
    let command = command_span(&payload);
    assert_eq!(command["status"]["code"], 2);
    assert_eq!(attribute(command, "error.type"), json!({"stringValue": "internal"}));
}

#[test]
fn nested_commands_share_the_outer_commands_trace() {
    let (output, payload) = export_script(json!([
        {"command":"sketch.create","params":{"plane":"XY"}},
        {"command":"sketch.draw.dimensioned","params":{
            "command":"sketch.line", "params":{"points":[[0,0],[10,0]]}, "dimensions":{"length":10}
        }}
    ]));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let all = spans(&payload);
    let outer = all.iter().find(|s| attribute(s, "solidcraft.command.id")["stringValue"] == "sketch.draw.dimensioned").unwrap();
    let inner = all.iter().find(|s| attribute(s, "solidcraft.command.id")["stringValue"] == "sketch.line").unwrap();
    assert_eq!(inner["parentSpanId"], outer["spanId"]);
    assert_eq!(inner["traceId"], outer["traceId"]);
}

#[test]
fn successful_parameter_change_can_have_a_failed_evaluation() {
    let (output, payload) = export_script(json!([
        {"command":"parameters.add","params":{"name":"length","expression":"10 mm"}},
        {"command":"solid.box","params":{"length":"length","width":20,"height":30}},
        {"command":"parameters.change","params":{"name":"length","expression":"0 mm"}}
    ]));
    // The command succeeds, but `run` refuses to export the invalid final model.
    assert!(!output.status.success());
    let all = spans(&payload);
    let command = all.iter().find(|s| attribute(s, "solidcraft.command.id")["stringValue"] == "parameters.change").unwrap();
    assert_eq!(command["status"]["code"], 1);
    let failed = all.iter().find(|s| s["parentSpanId"] == command["spanId"] && s["status"]["code"] == 2).unwrap();
    assert_eq!(attribute(failed, "error.type")["stringValue"], "model_evaluation");
    assert_eq!(attribute(failed, "solidcraft.model.error_count")["intValue"], "1");
}

#[test]
fn disabled_commands_are_exported_as_errors() {
    let (output, payload) = export("sketch.line", json!({"points":[[0,0],[10,0]]}), false);
    assert!(!output.status.success());
    assert_eq!(attribute(command_span(&payload), "error.type")["stringValue"], "disabled");
}

#[test]
fn sampler_can_disable_export_without_affecting_commands() {
    let (listener, endpoint) = receiver();
    let output = cli(&endpoint, true)
        .env("OTEL_TRACES_SAMPLER", "always_off")
        .args(["exec", "solid.box", r#"{"length":10,"width":20,"height":30}"#])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn unavailable_collector_does_not_change_command_results() {
    let (listener, endpoint) = receiver();
    drop(listener);
    let output = cli(&endpoint, true).args(["exec", "solid.box", r#"{"length":10,"width":20,"height":30}"#]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_ok());
}

#[test]
fn invalid_export_configuration_does_not_prevent_commands() {
    let output = cli("http://[invalid", true).args(["exec", "solid.box", r#"{"length":10,"width":20,"height":30}"#]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_ok());
}
