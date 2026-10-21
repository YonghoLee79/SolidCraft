# Local OpenTelemetry traces

SolidCraft can export native geometry traces to an OTLP/HTTP collector for local bug investigation.
Boolean attempts, result checks, tessellation and blends retain the command and evaluation that triggered them.
The `telemetry` Cargo feature includes the exporter. `SOLIDCRAFT_OTEL=1` activates it at startup.
Normal builds contain no exporter. Browser builds contain no exporter, even with this feature enabled.

The collector is optional. [Logal](https://github.com/CaliLuke/autok-logal) is one local option with a disposable SQLite database.
Other OTLP/HTTP collectors work through the same interface. SolidCraft has no dependency on Logal or its database.

## Start a traced run

Start your collector on a loopback address. Then run the desktop app:

```sh
SOLIDCRAFT_OTEL=1 \
SOLIDCRAFT_OTEL_RUN_ID=bug-before \
OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318 \
cargo run --profile ci -p solidcraft --features telemetry -- --sample
```

For a script, use the CLI:

```sh
SOLIDCRAFT_OTEL=1 \
SOLIDCRAFT_OTEL_RUN_ID=bug-before \
OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318 \
cargo run --profile ci -p solidcraft-cli --features telemetry -- \
  run reproduction.json --results --profile
```

The same feature covers headless MCP through `solidcraft-cli mcp`.
For MCP connected to a desktop control port, enable telemetry in the desktop process that runs the commands.
Each process owns its exporter. Cross-process trace propagation is not implemented.

| Configuration | Behavior |
|---|---|
| `SOLIDCRAFT_OTEL=1` | Enables export. All other values leave export off. |
| `SOLIDCRAFT_OTEL_RUN_ID` | Optional comparison label, limited to 128 characters. Exported as `solidcraft.run.id`. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Base URL. Defaults to `http://localhost:4318`. The exporter appends `/v1/traces`. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Complete trace URL. Overrides the base URL. Include `/v1/traces`. |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `http/protobuf` by default. `http/json` is also supported. No gRPC transport. |
| `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` | Overrides the general protocol for traces. |
| `OTEL_RESOURCE_ATTRIBUTES` | Optional resource attributes, for example `vcs.ref.head.revision=abc123`. |
| `OTEL_TRACES_SAMPLER` | Standard SDK sampler. The default records all root traces and follows each parent decision. |

The service name is `solidcraft` or `solidcraft-cli`. The resource also includes `service.version`.
This development exporter uses plain HTTP. Use a loopback collector as the example shows.
The exporter supports standard OTLP headers, but it has no TLS transport configured.

## What the traces contain

| Span | Diagnostics |
|---|---|
| `solidcraft.kernel.boolean` | Operation (`union`, `cut`, `intersect`), input/output face and shell counts, mesh-body flags, empty result, accepted fallback strategies and result checks. |
| `solidcraft.kernel.boolean.attempt` | Each underlying B-rep attempt, its tolerance, operand translation, output face count and error class. |
| `solidcraft.kernel.faceted_boolean` | Faceted boolean operation, tolerance, input body complexity, tool triangles, empty result and output complexity. |
| `solidcraft.kernel.tessellate` | Display/reference mode, requested tolerance and angle when available, refinement, cache hit, vertex and triangle counts. |
| `solidcraft.kernel.tessellation_plan` | Plan construction, requested tolerance and angle, input complexity and errors. |
| `solidcraft.kernel.fillet`, `solidcraft.kernel.chamfer` | Standard constant-radius fillet and equal-distance chamfer: edge count, size, input/output complexity and errors. |
| `solidcraft.command` | Registered command ID, validation/execution/rollback/refresh duration and aggregate kernel timing. |
| `solidcraft.evaluate` | Full model evaluation, error count, recomputed and restored feature counts. |

Kernel spans use the `solidcraft.kernel` instrumentation scope. Command and evaluation spans use `solidcraft.engine`.
Geometry dispatched through the kernel worker helpers keeps its parent context, including local Rayon pools.
Nested commands share their parent trace. Initial evaluations can appear as root spans.
Live sketch updates that skip a full evaluation have no evaluation span.

### Boolean failures and incorrect successes

Each boolean records `solidcraft.boolean.operation`. Attempts include `solidcraft.boolean.tolerance_mm`
and `solidcraft.boolean.translation_mm`. A failed attempt can be followed by a successful retry;
inspect the enclosing boolean before treating an attempt error as a failed feature.

The `solidcraft.boolean.candidate` event reports the existing result checks:
`mesh_failed`, `volume_bounds`, `unfused_shells`, `membership`, `membership_unchecked`, or `passed`.
It includes finite candidate and operand volumes plus the volume slack, in cubic millimetres.
`membership_unchecked` means the sample check was unavailable. These events observe existing checks;
they do not add geometry validation or change acceptance criteria.

Accepted fallback events are named `solidcraft.boolean.fallback.accepted`. They contain
`solidcraft.boolean.strategy`, translation, rotation, face shift and tolerance.
The enclosing span reports `solidcraft.boolean.fallback_count`, `solidcraft.boolean.fallbacks_truncated`,
`solidcraft.boolean.alters_result` and `solidcraft.boolean.unverified`.
An accepted `unverified_shortcut` marks the boolean span as an error with `error.type=unverified_result`,
even if the API returns a body successfully. The existing result and warning remain unchanged.
Other kernel failures use `invalid_input`, `operation_failed`, `too_large` or `internal`.
A scope that unwinds uses `panic`.
Caught kernel errors also leave a `solidcraft.kernel.guard_error` event, without the error text.

A successful command can still produce an invalid model. Such an evaluation has error status,
`error.type=model_evaluation`, and `solidcraft.model.error_count` greater than zero.
Commands include `solidcraft.command.id`, `solidcraft.kernel.boolean_ms` and `solidcraft.kernel.tessellate_ms`.
The timing uses the same thread-local counters as `run --profile`; worker spans give separate durations.
Unknown command IDs export as `unknown`. Command error classes are `unknown_command`, `disabled`,
`bad_params`, `internal` and `other`.

### Long-running operations

Each recorded kernel operation first completes a short `solidcraft.kernel.started` child span.
Its `solidcraft.kernel.operation` attribute identifies the operation. The batch exporter can send
this marker before the parent finishes, so a hang can leave evidence of where work started.
An unfinished parent will not appear as a completed span. Missing completion can also mean export
loss or an abrupt process exit; a started marker alone does not prove a hang.

### Data and limits

Command arguments, return values, error messages, design names and paths are excluded.
Numerical geometry diagnostics include volumes, blend sizes, tolerances and complexity counts.
Run labels and explicit resource attributes contain text you supply.
This integration exports traces only. It does not export application logs or metrics.
Existing [crash reports](crash-reports.md) retain the detailed reproduction data locally.

The hooks cover the operations listed above, not every kernel routine or every blend variant.
Only accepted retries enter the caller's existing fallback report. Traces can also show unsuccessful
nested work that the caller discarded. Read the enclosing result and its accepted fallback events.
SDK span/event limits and the export queue can truncate diagnostic evidence.

## Optional Logal setup

Install Logal with the instructions in its repository. The following configuration works with Logal 0.5.0.
From the SolidCraft repository, create a disposable collector directory:

```sh
mkdir -p target/telemetry
cat > target/telemetry/logal.yaml <<'YAML'
extensions:
  logal_store:
    path: ./target/telemetry/otel.debug.sqlite
    retention_hours: 48
  logal_status:
    store: logal_store
    endpoint: 127.0.0.1:13133
    max_in_flight_requests: 8
receivers:
  otlp:
    protocols:
      http:
        endpoint: 127.0.0.1:4318
exporters:
  logal_sqlite/traces:
    store: logal_store
service:
  telemetry:
    metrics:
      level: none
  extensions: [logal_store, logal_status]
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [logal_sqlite/traces]
YAML
logal serve --config target/telemetry/logal.yaml
```

Logal owns this disposable database. Do not point it at an existing database with valuable data.
In another terminal, check readiness before you run SolidCraft:

```sh
curl --fail http://127.0.0.1:13133/readyz
```

After a traced run, read its spans:

```sh
logal spans --db target/telemetry/otel.debug.sqlite \
  --service solidcraft-cli --resource solidcraft.run.id=bug-before --json
logal spans --db target/telemetry/otel.debug.sqlite \
  --service solidcraft-cli --resource solidcraft.run.id=bug-before --status error --json
```

Use `logal trace --db target/telemetry/otel.debug.sqlite --json TRACE_ID` to inspect a complete trace.
Collector files under `target/` are local build artifacts. They are not part of a design or a PR.

## Reproduce, fix and evaluate

1. Create a small script that reproduces the bug. Record the expected geometry or command result.
2. Run it with `SOLIDCRAFT_OTEL_RUN_ID=bug-before` and `--results --profile`.
3. Check that the collector contains spans for that run. No spans means missing evidence, not a successful reproduction.
4. Inspect error spans. Follow the trace from the command and evaluation to the boolean attempts,
   candidate checks, accepted fallbacks and tessellation. For a hang, inspect started markers.
5. For a crash, inspect the local crash report. Use its saved design and replay script as the regression test input.
6. Add a regression test for the incorrect result. Apply the fix.
7. Run the same input with `SOLIDCRAFT_OTEL_RUN_ID=bug-after`.
8. Compare geometry assertions, evaluation errors, accepted fallback strategies, tolerances and timings. Repeat timing measurements under similar load.
9. Run the regression test and quality gates. Include the reproduction, expected result and relevant evidence in the PR.

An empty error query does not prove correct geometry. The regression test must check the expected behavior.
For visible CAD changes, include screenshots or a recording from the reproduction.
Trace IDs and run labels help locate evidence. They do not replace that evidence or the regression test.

## Failure behavior and tests

Export uses a background thread with a bounded queue of 1,024 spans.
HTTP requests have a fixed one-second timeout. Normal exit waits up to two seconds to flush pending spans.
An unavailable collector does not change command results. A full queue, failed export or abrupt exit can lose spans.
Invalid exporter configuration leaves the app operational. Setup and shutdown diagnostics go to stderr, preserving MCP and CLI stdout.

The transport tests use an ephemeral loopback receiver. They require no collector installation:

```sh
cargo test --profile ci -p solidcraft-cli --features telemetry --test telemetry
cargo xtask ci
```

Native CI includes the optional feature. The wasm check also enables it to check the native dependency boundary.
