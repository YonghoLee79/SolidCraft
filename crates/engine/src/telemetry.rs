//! Opt-in native OTLP traces for reproducing bugs. The host owns the exporter lifetime;
//! library users are never connected to a collector just by constructing a session.
//! No command arguments, results, design names, paths or error messages are recorded.

use std::time::Duration;

use opentelemetry::trace::{Status, TraceContextExt, Tracer};
use opentelemetry::{Context, ContextGuard, KeyValue, global};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};

use crate::{EngineError, Result};

const EXPORT_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Keep alive until the host exits. Normal exit flushes short CLI runs as well as desktop runs.
pub struct Telemetry(SdkTracerProvider);

impl Drop for Telemetry {
    fn drop(&mut self) {
        if self.0.shutdown_with_timeout(SHUTDOWN_TIMEOUT).is_err() {
            eprintln!("SolidCraft telemetry: shutdown could not export all pending spans");
        }
    }
}

/// Install once at native host startup, only with `SOLIDCRAFT_OTEL=1`. OTLP configuration
/// comes from the standard `OTEL_EXPORTER_OTLP_*` variables; the default is localhost:4318.
/// An unavailable collector never blocks a command. Invalid setup leaves tracing disabled.
pub fn init(service: &'static str) -> Option<Telemetry> {
    if std::env::var("SOLIDCRAFT_OTEL").as_deref() != Ok("1") {
        return None;
    }
    // SDK startup can fail to spawn its background thread under resource pressure. Keep that
    // third-party failure outside the application, just as an exporter build error is.
    match std::panic::catch_unwind(|| build(service)) {
        Ok(Ok(provider)) => {
            global::set_tracer_provider(provider.clone());
            Some(Telemetry(provider))
        }
        _ => {
            eprintln!("SolidCraft telemetry: initialization failed; check OTLP/HTTP configuration; continuing without export");
            None
        }
    }
}

fn build(service: &'static str) -> std::result::Result<SdkTracerProvider, opentelemetry_otlp::ExporterBuildError> {
    let exporter = opentelemetry_otlp::SpanExporter::builder().with_http().with_timeout(EXPORT_TIMEOUT).build()?;
    let processor = BatchSpanProcessor::builder(exporter)
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(1024)
                .with_max_export_batch_size(256)
                .with_scheduled_delay(Duration::from_secs(1))
                .build(),
        )
        .build();
    let mut resource = Resource::builder().with_service_name(service).with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")));
    if let Ok(run) = std::env::var("SOLIDCRAFT_OTEL_RUN_ID") {
        resource = resource.with_attribute(KeyValue::new("solidcraft.run.id", run.chars().take(128).collect::<String>()));
    }
    Ok(SdkTracerProvider::builder().with_resource(resource.build()).with_span_processor(processor).build())
}

struct Scope {
    context: Context,
    _guard: ContextGuard,
}

impl Scope {
    fn start(name: &'static str) -> Option<Self> {
        let span = global::tracer("solidcraft.engine").start(name);
        // A sampled-out span still carries a valid parent. Keep it attached so children
        // inherit that sampling decision rather than becoming independent root traces.
        if !opentelemetry::trace::Span::span_context(&span).is_valid() {
            return None;
        }
        let context = Context::current_with_span(span);
        let guard = context.clone().attach();
        Some(Self { context, _guard: guard })
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let span = self.context.span();
        if std::thread::panicking() {
            span.set_attribute(KeyValue::new("error.type", "panic"));
            span.set_status(Status::error("panic"));
        }
        span.end();
    }
}

/// The wrapper includes validation, caught panics, rollback and the final model refresh.
pub(crate) fn command<T>(id: &str, run: impl FnOnce() -> Result<T>) -> Result<T> {
    let Some(scope) = Scope::start("solidcraft.command") else { return run() };
    let span = scope.context.span();
    if !span.is_recording() {
        return run();
    }
    // Never export an unknown id: it is untrusted input and may contain private text.
    span.set_attribute(KeyValue::new("solidcraft.command.id", crate::find_command(id).map_or("unknown", |s| s.id)));
    let (result, times) = crate::kernel::profile::capture(run);
    span.set_attribute(KeyValue::new("solidcraft.kernel.boolean_ms", times.boolean_ms));
    span.set_attribute(KeyValue::new("solidcraft.kernel.tessellate_ms", times.tessellate_ms));
    match &result {
        Ok(_) => span.set_status(Status::Ok),
        Err(error) => {
            let kind = match error {
                EngineError::UnknownCommand(_) => "unknown_command",
                EngineError::Disabled(..) => "disabled",
                EngineError::BadParams { .. } => "bad_params",
                EngineError::Internal(..) => "internal",
                EngineError::Other(_) => "other",
            };
            span.set_attribute(KeyValue::new("error.type", kind));
            span.set_status(Status::error(kind));
        }
    }
    result
}

/// Evaluation can return model errors even when the command itself succeeds. Record it as
/// its own child span so that a successful command is not mistaken for a valid model.
pub(crate) struct Evaluation(Option<Scope>);

impl Evaluation {
    pub fn start() -> Self {
        Self(Scope::start("solidcraft.evaluate"))
    }

    pub fn finish(self, model: &crate::doc::Model) {
        let Some(scope) = &self.0 else { return };
        let span = scope.context.span();
        if !span.is_recording() {
            return;
        }
        let errors = model.param_errors.len() + model.results.iter().filter(|r| r.error.is_some()).count();
        span.set_attribute(KeyValue::new("solidcraft.model.error_count", errors as i64));
        span.set_attribute(KeyValue::new("solidcraft.model.recomputed", model.last_recomputed as i64));
        span.set_attribute(KeyValue::new("solidcraft.model.restored", model.last_restored as i64));
        if errors == 0 {
            span.set_status(Status::Ok);
        } else {
            span.set_attribute(KeyValue::new("error.type", "model_evaluation"));
            span.set_status(Status::error("model_evaluation"));
        }
    }
}
