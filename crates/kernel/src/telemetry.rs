//! Geometry diagnostics, independent of any exporter. Hosts install the provider. Only
//! operation names, numerical diagnostics and complexity counts leave the kernel.

use opentelemetry::trace::{Span as _, Status, TraceContextExt, Tracer};
use opentelemetry::{Context, ContextGuard, KeyValue, global};

use crate::boolean_diagnostics::{BooleanDiagnostics, BooleanStrategy, capture_boolean_diagnostics, merge};
use crate::{Body, BoolOp, KernelError, Result};

pub(crate) struct Span {
    context: Context,
    _guard: ContextGuard,
}

impl Span {
    pub fn start(name: &'static str) -> Self {
        let tracer = global::tracer("solidcraft.kernel");
        let span = tracer.start(name);
        let context = Context::current_with_span(span);
        let guard = context.clone().attach();
        let scope = Self { context, _guard: guard };
        if scope.recording() {
            // Completed markers can be exported while the parent is still running. A hang
            // otherwise exports neither its operation nor events attached to that operation.
            let mut started = tracer.start("solidcraft.kernel.started");
            started.set_attribute(KeyValue::new("solidcraft.kernel.operation", name));
            started.end();
        }
        scope
    }

    pub fn recording(&self) -> bool {
        self.context.span().is_recording()
    }

    pub fn attributes(&self, attrs: impl IntoIterator<Item = KeyValue>) {
        self.context.span().set_attributes(attrs);
    }

    pub fn finish<T>(&self, result: &Result<T>) {
        match result {
            Ok(_) => self.context.span().set_status(Status::Ok),
            Err(error) => self.error(error_type(error)),
        }
    }

    fn error(&self, kind: &'static str) {
        self.context.span().set_attribute(KeyValue::new("error.type", kind));
        self.context.span().set_status(Status::error(kind));
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.error("panic");
        }
        self.context.span().end();
    }
}

fn error_type(error: &KernelError) -> &'static str {
    match error {
        KernelError::Invalid(_) => "invalid_input",
        KernelError::Failed(_) => "operation_failed",
        KernelError::TooLarge(_) => "too_large",
        KernelError::Internal(_) => "internal",
    }
}

fn operation(op: BoolOp) -> &'static str {
    match op {
        BoolOp::Union => "union",
        BoolOp::Cut => "cut",
        BoolOp::Intersect => "intersect",
    }
}

fn complexity(prefix: &'static str, body: &Body) -> [KeyValue; 3] {
    let (faces, shells, mesh) = match prefix {
        "a" => ("solidcraft.input.a.faces", "solidcraft.input.a.shells", "solidcraft.input.a.mesh"),
        "b" => ("solidcraft.input.b.faces", "solidcraft.input.b.shells", "solidcraft.input.b.mesh"),
        _ => ("solidcraft.output.faces", "solidcraft.output.shells", "solidcraft.output.mesh"),
    };
    [KeyValue::new(faces, body.face_count() as i64), KeyValue::new(shells, body.shell_count() as i64), KeyValue::new(mesh, body.is_mesh())]
}

fn strategy(strategy: BooleanStrategy) -> &'static str {
    match strategy {
        BooleanStrategy::RefinedTolerance => "refined_tolerance",
        BooleanStrategy::ShiftedOperands => "shifted_operands",
        BooleanStrategy::CoplanarSeparation => "coplanar_separation",
        BooleanStrategy::CommonBore => "common_bore",
        BooleanStrategy::GrownBoss => "grown_boss",
        BooleanStrategy::PlanarFallback => "planar_fallback",
        BooleanStrategy::ToolNudge => "tool_nudge",
        BooleanStrategy::ToolReseam => "tool_reseam",
        BooleanStrategy::Piecewise => "piecewise",
        BooleanStrategy::UnverifiedShortcut => "unverified_shortcut",
    }
}

fn fallbacks(span: &Span, report: &BooleanDiagnostics) {
    let unverified = report.events.iter().any(|e| e.strategy == BooleanStrategy::UnverifiedShortcut);
    span.attributes([
        KeyValue::new("solidcraft.boolean.fallback_count", report.events.len() as i64),
        KeyValue::new("solidcraft.boolean.fallbacks_truncated", report.truncated),
        KeyValue::new("solidcraft.boolean.unverified", unverified),
        KeyValue::new("solidcraft.boolean.alters_result", report.events.iter().any(|e| e.strategy.alters_result())),
    ]);
    for event in &report.events {
        span.context.span().add_event(
            "solidcraft.boolean.fallback.accepted",
            vec![
                KeyValue::new("solidcraft.boolean.strategy", strategy(event.strategy)),
                KeyValue::new("solidcraft.boolean.translation_mm", event.translation_mm),
                KeyValue::new("solidcraft.boolean.rotation_rad", event.rotation_rad),
                KeyValue::new("solidcraft.boolean.face_shift_mm", event.face_shift_mm),
                KeyValue::new("solidcraft.boolean.tolerance_mm", event.tolerance_mm),
            ],
        );
    }
    if unverified {
        // The API can succeed with an explicit warning that an intersection was ignored.
        // Make that searchable as an error without changing the geometry or API result.
        span.error("unverified_result");
    } else {
        span.context.span().set_status(Status::Ok);
    }
}

pub(crate) fn boolean(a: &Body, b: &Body, op: BoolOp, run: impl FnOnce() -> Result<Option<Body>>) -> Result<Option<Body>> {
    let span = Span::start("solidcraft.kernel.boolean");
    if !span.recording() {
        return run();
    }
    span.attributes(complexity("a", a).into_iter().chain(complexity("b", b)).chain([KeyValue::new("solidcraft.boolean.operation", operation(op))]));
    let (result, report) = capture_boolean_diagnostics(run);
    if let Ok(body) = &result {
        span.attributes([KeyValue::new("solidcraft.output.empty", body.is_none())]);
        if let Some(body) = body {
            span.attributes(complexity("output", body));
        }
        fallbacks(&span, &report);
    } else {
        span.finish(&result);
    }
    // Preserve the existing diagnostic capture semantics, including truncation, for callers.
    merge(report);
    result
}

pub(crate) fn attempt<T>(op: BoolOp, tol: f64, shift: f64, run: impl FnOnce() -> Result<(crate::body::Solid, T)>) -> Result<(crate::body::Solid, T)> {
    let span = Span::start("solidcraft.kernel.boolean.attempt");
    if span.recording() {
        span.attributes([
            KeyValue::new("solidcraft.boolean.operation", operation(op)),
            KeyValue::new("solidcraft.boolean.tolerance_mm", tol),
            KeyValue::new("solidcraft.boolean.translation_mm", shift),
        ]);
    }
    let result = run();
    span.finish(&result);
    if span.recording()
        && let Ok((solid, _)) = &result
    {
        span.attributes([KeyValue::new("solidcraft.output.faces", solid.face_iter().count() as i64)]);
    }
    result
}

pub(crate) fn faceted_boolean(
    a: &Body,
    b: &solidcraft_geom::Mesh,
    op: BoolOp,
    tol: f64,
    run: impl FnOnce() -> Result<Option<Body>>,
) -> Result<Option<Body>> {
    let span = Span::start("solidcraft.kernel.faceted_boolean");
    if span.recording() {
        span.attributes(complexity("a", a));
        span.attributes([
            KeyValue::new("solidcraft.boolean.operation", operation(op)),
            KeyValue::new("solidcraft.input.b.triangles", b.triangles.len() as i64),
        ]);
        if tol.is_finite() {
            span.attributes([KeyValue::new("solidcraft.boolean.tolerance_mm", tol)]);
        }
    }
    let result = run();
    span.finish(&result);
    if span.recording()
        && let Ok(body) = &result
    {
        span.attributes([KeyValue::new("solidcraft.output.empty", body.is_none())]);
        if let Some(body) = body {
            span.attributes(complexity("output", body));
        }
    }
    result
}

pub(crate) fn mesh(
    body: &Body,
    mode: &'static str,
    tol: f64,
    angle: f64,
    refine: bool,
    run: impl FnOnce() -> Result<solidcraft_geom::Mesh>,
) -> Result<solidcraft_geom::Mesh> {
    let span = Span::start("solidcraft.kernel.tessellate");
    if span.recording() {
        span.attributes(complexity("a", body));
        span.attributes([
            KeyValue::new("solidcraft.mesh.mode", mode),
            KeyValue::new("solidcraft.mesh.refine", refine),
            KeyValue::new("solidcraft.mesh.cached", false),
        ]);
        if tol.is_finite() {
            span.attributes([KeyValue::new("solidcraft.mesh.tolerance_mm", tol)]);
        }
        if angle.is_finite() {
            span.attributes([KeyValue::new("solidcraft.mesh.angle_rad", angle)]);
        }
    }
    let result = run();
    span.finish(&result);
    if let Ok(mesh) = &result {
        span.attributes([
            KeyValue::new("solidcraft.mesh.vertices", mesh.positions.len() as i64),
            KeyValue::new("solidcraft.mesh.triangles", mesh.triangles.len() as i64),
        ]);
    }
    result
}

pub(crate) fn cached_mesh() {
    Context::current().span().set_attribute(KeyValue::new("solidcraft.mesh.cached", true));
}

pub(crate) fn plan<T>(body: &Body, tol: f64, angle: Option<f64>, run: impl FnOnce() -> Result<T>) -> Result<T> {
    let span = Span::start("solidcraft.kernel.tessellation_plan");
    if span.recording() {
        span.attributes(complexity("a", body));
        if tol.is_finite() {
            span.attributes([KeyValue::new("solidcraft.mesh.tolerance_mm", tol)]);
        }
        if let Some(angle) = angle.filter(|a| a.is_finite()) {
            span.attributes([KeyValue::new("solidcraft.mesh.angle_rad", angle)]);
        }
    }
    let result = run();
    span.finish(&result);
    result
}

pub(crate) fn blend(name: &'static str, body: &Body, edges: usize, size: f64, run: impl FnOnce() -> Result<Body>) -> Result<Body> {
    let span = Span::start(name);
    if span.recording() {
        span.attributes(complexity("a", body));
        span.attributes([KeyValue::new("solidcraft.blend.edge_count", edges as i64)]);
        if size.is_finite() {
            span.attributes([KeyValue::new("solidcraft.blend.size_mm", size)]);
        }
    }
    let result = run();
    span.finish(&result);
    if span.recording()
        && let Ok(body) = &result
    {
        span.attributes(complexity("output", body));
    }
    result
}

pub(crate) fn candidate(reason: &'static str, volume: f64, va: f64, vb: f64, slack: f64) {
    let context = Context::current();
    if !context.span().is_recording() {
        return;
    }
    let mut attrs = vec![KeyValue::new("solidcraft.boolean.check", reason)];
    for (key, value) in [
        ("solidcraft.boolean.volume_mm3", volume),
        ("solidcraft.boolean.input_a_volume_mm3", va),
        ("solidcraft.boolean.input_b_volume_mm3", vb),
        ("solidcraft.boolean.volume_slack_mm3", slack),
    ] {
        if value.is_finite() {
            attrs.push(KeyValue::new(key, value));
        }
    }
    context.span().add_event("solidcraft.boolean.candidate", attrs);
}

pub(crate) fn guard_error(error: &KernelError) {
    let context = Context::current();
    if context.span().is_recording() {
        context.span().add_event("solidcraft.kernel.guard_error", vec![KeyValue::new("error.type", error_type(error))]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BooleanFallback;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

    fn exported_fallback(strategy: BooleanStrategy) -> SpanData {
        // A local provider keeps this test independent of other kernel tests and globals.
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder().with_simple_exporter(exporter.clone()).build();
        let context = Context::new().with_span(provider.tracer("test").start("boolean"));
        let guard = context.clone().attach();
        let span = Span { context, _guard: guard };
        let report = BooleanDiagnostics { events: vec![BooleanFallback::new(strategy)], truncated: false, rejected: vec![] };
        fallbacks(&span, &report);
        drop(span);
        exporter.get_finished_spans().unwrap().remove(0)
    }

    #[test]
    fn unverified_success_is_exported_as_error() {
        let span = exported_fallback(BooleanStrategy::UnverifiedShortcut);
        assert_eq!(span.status, Status::error("unverified_result"));
        assert!(span.attributes.contains(&KeyValue::new("solidcraft.boolean.unverified", true)));
        assert_eq!(span.events.events[0].name, "solidcraft.boolean.fallback.accepted");
    }

    #[test]
    fn exact_retry_keeps_success_status() {
        let span = exported_fallback(BooleanStrategy::PlanarFallback);
        assert_eq!(span.status, Status::Ok);
        assert!(span.attributes.contains(&KeyValue::new("solidcraft.boolean.unverified", false)));
    }
}
