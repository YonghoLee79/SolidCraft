//! Passive selection estimates from the same engine query as `inspect.measure`.
use std::sync::{Arc, mpsc};

use egui::{FontId, Label, Rect, RichText, Stroke, vec2};
use solidcraft_engine::{Sel, Session, doc::ModelState};

use crate::{SolveApp, theme::Tokens};

#[derive(Clone)]
struct Key {
    world: Arc<ModelState>,
    revision: u64,
    items: Vec<Sel>,
}

impl Key {
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.world, &other.world) && self.revision == other.revision && self.items == other.items
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Length,
    Distance,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    kind: Kind,
    mm: f64,
}

struct Running {
    key: Key,
    result: mpsc::Receiver<Option<Reading>>,
}

/// One cached selection and at most one native worker; obsolete work never replaces current data.
#[derive(Default)]
pub(crate) struct State {
    key: Option<Key>,
    reading: Option<Reading>,
    attempted: bool,
    running: Option<Running>,
    #[cfg(test)]
    computations: usize,
}

impl State {
    fn clear(&mut self) {
        self.key = None;
        self.reading = None;
        self.attempted = false;
    }

    fn poll(&mut self) {
        let ready = self.running.as_ref().and_then(|job| match job.result.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Disconnected) => Some(None),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(value) = ready
            && let Some(job) = self.running.take()
            && self.key.as_ref().is_some_and(|key| key.matches(&job.key))
        {
            self.reading = value;
        }
    }

    fn update(&mut self, session: &Session, ctx: &egui::Context) {
        self.poll();
        let items = &session.selection;
        if !(items.len() == 1 && matches!(items.first(), Some(Sel::Edge { .. })) || items.len() == 2)
            || !items.iter().all(|item| match item {
                Sel::Edge { point, .. } | Sel::Face { point, .. } | Sel::Vertex { point, .. } => point.is_finite(),
                _ => false,
            })
        {
            self.clear();
            return;
        }
        let key = Key { world: session.world_state(), revision: session.revision, items: items.clone() };
        if !self.key.as_ref().is_some_and(|old| old.matches(&key)) {
            self.key = Some(key.clone());
            self.reading = None;
            self.attempted = false;
        }
        if !self.attempted && self.running.is_none() {
            self.attempted = true;
            if let Some(cost) = validate(&key) {
                #[cfg(test)]
                {
                    self.computations = self.computations.saturating_add(1);
                }
                // Tiny selections are cheap and deterministic in the UI harness. Large native
                // queries run away from input/rendering, with only the newest selection queued.
                #[cfg(not(target_arch = "wasm32"))]
                if cost > 250_000 {
                    let scratch = session.scratch();
                    let items = key.items.clone();
                    let (tx, rx) = mpsc::channel();
                    let repaint = ctx.clone();
                    match std::thread::Builder::new().name("quick-measure".into()).spawn(move || {
                        let _ = tx.send(compute(&scratch, &items));
                        repaint.request_repaint();
                    }) {
                        Ok(_) => self.running = Some(Running { key, result: rx }),
                        Err(e) => log::warn!("cannot start selection measurement: {e}"),
                    }
                    return;
                }
                // Match the existing Measure/preview web path: no threads, once per change.
                let _ = cost;
                self.reading = compute(session, &key.items);
            }
        }
        if self.running.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

/// Reject stale/unsupported picks before Measure's nearest-edge fallback can retarget them.
/// The cost estimate covers its capped point/triangle comparisons, including farthest distance.
fn validate(key: &Key) -> Option<usize> {
    let mut sizes = Vec::new();
    for item in &key.items {
        let (body, point) = match item {
            Sel::Edge { body, point, .. } | Sel::Face { body, point, .. } | Sel::Vertex { body, point } => (body, point),
            _ => return None,
        };
        let body = key.world.body(body)?;
        if body.body.is_mesh() {
            return None;
        }
        let mesh = body.mesh();
        let tol = mesh.bounds().diagonal().max(1.0) * 1e-6;
        let size = match item {
            Sel::Edge { index, .. } => {
                let edge = mesh.edges.get(*index)?;
                if !edge.windows(2).any(|w| match w {
                    [a, b] => point.dist_to_segment(*a, *b) <= tol,
                    _ => false,
                }) {
                    return None;
                }
                let points = mesh
                    .circular_rim(*index)
                    .iter()
                    .filter_map(|&i| mesh.edges.get(i))
                    .fold(0usize, |n, e| n.saturating_add(e.len().saturating_add(64)));
                (points.min(4000), 0)
            }
            Sel::Face { index, .. } => {
                if !mesh.tri_face.iter().any(|f| *f as usize == *index) {
                    return None;
                }
                let patch: std::collections::HashSet<_> = mesh.surface_patch(*index).into_iter().collect();
                let tris = mesh.tri_face.iter().filter(|f| patch.contains(&(**f as usize))).count();
                (tris.saturating_mul(3).min(4000), tris.min(20000))
            }
            Sel::Vertex { .. } => {
                if !mesh.edges.iter().flat_map(|e| [e.first(), e.last()]).flatten().any(|p| p.dist(*point) <= tol) {
                    return None;
                }
                (1, 0)
            }
            _ => return None,
        };
        sizes.push(size);
    }
    match sizes.as_slice() {
        [_] => Some(0),
        [(a, at), (b, bt)] => Some(a.saturating_mul(*b).saturating_add(a.saturating_mul(*bt)).saturating_add(b.saturating_mul(*at))),
        _ => None,
    }
}

fn compute(session: &Session, items: &[Sel]) -> Option<Reading> {
    let result = session.measure_items(items);
    let (kind, mm) = if items.len() == 1 {
        let one = result.get("items")?.as_array()?.first()?;
        let mm = ["arc_length", "circumference", "length_mm"].iter().find_map(|name| one.get(*name)?.as_f64())?;
        (Kind::Length, mm)
    } else {
        (Kind::Distance, result.get("distance_mm")?.as_f64()?)
    };
    (mm.is_finite() && mm >= 0.0 && (kind != Kind::Length || mm > 0.0)).then_some(Reading { kind, mm })
}

pub(crate) fn paint(app: &mut SolveApp, ui: &mut egui::Ui, painter: &egui::Painter, view: Rect, covered: &[Rect]) {
    if app.dialog.is_some()
        || app.tool.is_some()
        || app.applying.is_some()
        || app.deferred.is_some()
        || app.params.is_some()
        || app.design_properties.is_some()
        || app.prefs_window.open
        || app.home.open
        || !app.recoverable.is_empty()
        || app.docs.closing.is_some()
        || app.docs.quitting.is_some()
    {
        return;
    }
    app.quick_measure.update(&app.session, ui.ctx());
    let Some(reading) = app.quick_measure.reading else { return };
    let label = app.tr(match reading.kind {
        Kind::Length => "ui.quick_measure_length",
        Kind::Distance => "ui.quick_measure_distance",
    });
    let text = format!("{label}: ≈ {}", crate::prefs::show_mm(app, reading.mm, 1));
    let t = Tokens::get();
    let font = FontId::proportional(12.5);
    let galley = painter.layout_no_wrap(text.clone(), font.clone(), t.text);
    let mut obstacles = covered.to_vec();
    if let Some(layout) = &app.viewport.layout {
        obstacles.extend([layout.axis_view, layout.navigation]);
    }
    obstacles.extend(app.viewport.palette_rect.filter(|_| app.session.active_sketch.is_some()));
    obstacles.extend(app.frame_rate.rect());
    let Some(rect) = crate::viewport_layout::place_overlay(view, galley.size() + vec2(16.0, 8.0), &obstacles) else { return };
    painter.rect(rect, 4.0, t.overlay, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    // Hover-only label: its tooltip explains the estimate without taking clicks or drags.
    ui.put(rect.shrink2(vec2(8.0, 4.0)), Label::new(RichText::new(text).font(font).color(t.text)).selectable(false))
        .on_hover_text(app.tr("ui.quick_measure_estimate_hint"));
    crate::scenario::publish_rect("viewport:quick_measure", rect);
    crate::scenario::publish_handle("viewport:quick_measure", rect.center());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use solidcraft_engine::geom::Vec3;

    fn box_session(length: f64) -> Session {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":length,"width":30,"height":20,"body_name":"Box"})).unwrap();
        let world = s.world_state();
        let mesh = world.body("Box").unwrap().mesh();
        let (index, points) =
            mesh.edges.iter().enumerate().find(|(_, p)| (p.first().unwrap().dist(*p.last().unwrap()) - length).abs() < 1e-8).unwrap();
        s.selection = vec![Sel::Edge { body: "Box".into(), index, point: points.first().unwrap().lerp(*points.last().unwrap(), 0.5) }];
        s
    }

    fn key(s: &Session) -> Key {
        Key { world: s.world_state(), revision: s.revision, items: s.selection.clone() }
    }

    #[test]
    fn cache_reuses_results_and_retains_document_identity() {
        let ctx = egui::Context::default();
        let mut s = box_session(40.0);
        let before = (s.doc.to_json(), s.undo.len(), s.log.clone());
        let mut state = State::default();
        for _ in 0..30 {
            state.update(&s, &ctx);
        }
        assert_eq!(state.computations, 1, "idle/render frames reuse the result");
        assert_eq!(state.reading.unwrap().mm, 40.0);
        assert_eq!((s.doc.to_json(), s.undo.len(), s.log.clone()), before, "the readout is read-only");
        s.revision += 1;
        state.update(&s, &ctx);
        assert_eq!(state.computations, 2);
        let mut another = box_session(80.0);
        another.revision = s.revision;
        // Same numeric revision and selection; a different world must never reuse the old length.
        another.selection = s.selection.clone();
        state.update(&another, &ctx);
        assert_eq!(state.computations, 3);
        assert_eq!(state.reading.unwrap().mm, 80.0);
        another.selection.clear();
        state.update(&another, &ctx);
        assert_eq!(state.reading, None);
    }

    #[test]
    fn only_the_latest_selection_can_receive_a_worker_result() {
        let s = box_session(40.0);
        let mut next = box_session(80.0);
        let old = key(&s);
        let current = key(&next);
        let (tx, rx) = mpsc::channel();
        let mut state = State { key: Some(old.clone()), running: Some(Running { key: old, result: rx }), attempted: true, ..Default::default() };
        let ctx = egui::Context::default();
        state.update(&next, &ctx);
        assert_eq!(state.computations, 0, "do not flood workers when selection changes");
        assert!(state.key.as_ref().unwrap().matches(&current));
        tx.send(Some(Reading { kind: Kind::Length, mm: 40.0 })).unwrap();
        state.poll();
        assert_eq!(state.reading, None, "discard the obsolete result");
        state.update(&next, &ctx);
        assert_eq!(state.reading.unwrap().mm, 80.0);
        assert_eq!(state.computations, 1);
        next.selection.clear();
        state.update(&next, &ctx);
        assert_eq!(state.reading, None);
    }

    #[test]
    fn stale_unknown_and_nonfinite_picks_are_quietly_hidden_and_cached() {
        let mut s = box_session(40.0);
        let ctx = egui::Context::default();
        let mut state = State::default();
        let cases = [
            vec![Sel::Edge { body: "Box".into(), index: usize::MAX, point: Vec3::ZERO }],
            vec![Sel::Edge { body: "missing".into(), index: 0, point: Vec3::ZERO }],
            vec![Sel::Edge { body: "Box".into(), index: 0, point: Vec3::new(1e6, 0.0, 0.0) }],
            vec![Sel::Edge { body: "Box".into(), index: 0, point: Vec3::new(f64::NAN, 0.0, 0.0) }],
            vec![Sel::Face { body: "Box".into(), index: usize::MAX, point: Vec3::ZERO }, Sel::Vertex { body: "Box".into(), point: Vec3::ZERO }],
            vec![Sel::Vertex { body: "Box".into(), point: Vec3::new(1.0, 1.0, 1.0) }, Sel::Vertex { body: "Box".into(), point: Vec3::ZERO }],
        ];
        for items in cases {
            s.selection = items;
            for _ in 0..3 {
                state.update(&s, &ctx);
                assert_eq!(state.reading, None);
            }
        }
        assert_eq!(state.computations, 0, "do not run geometry for rejected inputs");
    }

    #[test]
    fn circular_and_fillet_arc_lengths_use_the_existing_round_measurements() {
        let mut cylinder = Session::default();
        cylinder.execute("solid.cylinder", &json!({"radius":5,"height":10,"body_name":"Cylinder"})).unwrap();
        let world = cylinder.world_state();
        let body = world.body("Cylinder").unwrap();
        let mesh = body.mesh();
        let (index, points) =
            mesh.edges.iter().enumerate().find(|(_, p)| p.len() > 2 && (p.first().unwrap().z - p.last().unwrap().z).abs() < 1e-8).unwrap();
        let item = Sel::Edge { body: "Cylinder".into(), index, point: points[points.len() / 2] };
        assert!((compute(&cylinder, &[item]).unwrap().mm - std::f64::consts::TAU * 5.0).abs() < 1e-8);

        let mut s = box_session(40.0);
        let Sel::Edge { point, .. } = s.selection.first().unwrap() else { panic!("expected an edge") };
        s.execute("solid.fillet", &json!({"body":"Box","edges":[point],"radius":3})).unwrap();
        let world = s.world_state();
        let body = world.body("Box").unwrap();
        let mesh = body.mesh();
        let (index, circle) = mesh
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, _)| solidcraft_engine::kernel::edge_circle(&body.body, i).map(|c| (i, c)))
            .find(|(_, c)| (c.radius - 3.0).abs() < 1e-8 && (c.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-8)
            .unwrap();
        let points = mesh.edges.get(index).unwrap();
        let item = Sel::Edge { body: "Box".into(), index, point: points[points.len() / 2] };
        assert!((compute(&s, &[item]).unwrap().mm - circle.radius * std::f64::consts::FRAC_PI_2).abs() < 1e-8);
    }

    #[test]
    fn mesh_bodies_are_not_eligible_even_with_a_valid_cached_edge() {
        let s = box_session(40.0);
        let mut k = key(&s);
        let mut world = (*k.world).clone();
        let body = world.bodies.iter_mut().find(|b| b.name == "Box").unwrap();
        let mesh = body.mesh();
        body.body = solidcraft_engine::kernel::mesh_body(&mesh.positions, &mesh.triangles).unwrap();
        k.world = Arc::new(world);
        assert!(validate(&k).is_none());
    }

    #[test]
    fn occurrence_faces_use_world_placement_and_pending_moves_invalidate_the_cache() {
        let mut s = Session::default();
        let c = s.execute("component.create", &json!({"name":"Part"})).unwrap();
        let component = c["component"].as_u64().unwrap();
        s.execute("solid.box", &json!({"length":40,"width":30,"height":20,"body_name":"Box"})).unwrap();
        s.execute("component.activate", &json!({"component":0})).unwrap();
        let second = s.execute("occurrence.copy", &json!({"component":component,"translate":[50,0,0]})).unwrap()["occurrence"].as_u64().unwrap();
        let world = s.world_state();
        assert_eq!(world.bodies.len(), 2);
        for body in &world.bodies {
            let mesh = body.mesh();
            let (tri, face) = mesh
                .triangles
                .iter()
                .zip(&mesh.tri_face)
                .find(|(t, _)| {
                    let [a, b, c] = mesh.tri(t).unwrap();
                    (a.x - b.x).abs() < 1e-8 && (a.x - c.x).abs() < 1e-8 && (a.x - mesh.bounds().min.x).abs() < 1e-8
                })
                .unwrap();
            let [a, b, c] = mesh.tri(tri).unwrap();
            s.selection.push(Sel::Face { body: body.name.clone(), index: *face as usize, point: (a + b + c) / 3.0 });
        }
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.update(&s, &ctx);
        assert!((state.reading.unwrap().mm - 50.0).abs() < 1e-8);
        let revision = s.revision;
        let mut moved = s.doc.occurrences.iter().find(|o| o.id == second).unwrap().transform;
        moved[3][0] += 10.0;
        s.pending_moves.insert(second, moved);
        state.update(&s, &ctx);
        assert_eq!(s.revision, revision, "pending world placement is part of the key independently of view revision");
        assert_eq!(state.computations, 2);
        assert!((state.reading.unwrap().mm - 60.0).abs() < 1e-8);
    }
}
