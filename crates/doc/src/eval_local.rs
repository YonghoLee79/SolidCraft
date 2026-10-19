//! Results reused across the model. Many features read only a few bodies (a cut of the bodies it
//! names; a new primitive, or a sketch on a fixed plane, reads none), so they give the same result
//! whatever the rest of the model holds. Keyed by what they read alone, an edit in one part of an
//! assembly does not recompute the costly features of another part after it in the timeline:
//! their results are laid over the other bodies as they are now.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use solidcraft_kernel as kernel;

use super::{ModelBody, ModelState, SolvedSketch, StepDigests, feature_op, fingerprint, placement_digest};
use crate::expr::Value;
use crate::{Document, Feature, FeatureKind, Operation, PatternKind, PlaneRef};

/// The feature a pattern or mirror replays under `name`: the one that makes geometry when a
/// sketch shares its name.
pub(super) fn replayed<'a>(doc: &'a Document, name: &str) -> Option<&'a Feature> {
    doc.features
        .iter()
        .find(|x| x.name == name && !matches!(x.kind, FeatureKind::Sketch { .. } | FeatureKind::ConstructionPlane { .. }))
        .or_else(|| doc.find_feature(name))
}

/// What a feature reads of the model besides every body's name (new names avoid them): bodies by
/// index and sketches by feature id.
struct Reads {
    bodies: Vec<usize>,
    sketches: Vec<u64>,
}

/// What `f` reads, when that is known to be all it reads: a primitive or a coil (or a
/// rectangular or circular pattern of them) cutting or joining the bodies it names or its
/// component's bodies, a move or pattern of named bodies, a sketch on a fixed plane with no
/// projections, an extrude or revolve making new bodies from a sketch of its component, with no
/// To / From Object, through all or taper. No face or edge picked by name and no sheet metal
/// around. Everything else may read anything (faces, To Object, every body with `*`) and is keyed
/// by the whole model.
fn reads(doc: &Document, f: &Feature, st: &ModelState) -> Option<Reads> {
    if !st.sheets.is_empty() || !plain_picks(f) {
        return None;
    }
    let named = |names: &[String]| -> Option<Vec<usize>> {
        if names.iter().any(|n| n == "*") {
            return None;
        }
        names.iter().map(|n| st.bodies.iter().position(|b| &b.name == n)).collect()
    };
    // The bodies a tool operates on (as `apply_traced` picks them).
    let operated = |op: Operation, targets: Vec<String>| -> Option<Vec<usize>> {
        if op == Operation::NewBody {
            return Some(Vec::new());
        }
        let targets = if targets.is_empty() { &f.participants } else { &targets };
        if targets.is_empty() {
            Some((0..st.bodies.len()).filter(|i| st.bodies.get(*i).is_some_and(|b| b.component == f.component || b.feature == f.id)).collect())
        } else {
            named(targets)
        }
    };
    let sketch_of = |sketch: &crate::SketchId| doc.feature(sketch.0).is_some_and(|s| s.component == f.component).then_some(sketch.0);
    let bodies = |bodies: Vec<usize>| Some(Reads { bodies, sketches: Vec::new() });
    match &f.kind {
        FeatureKind::Box { .. }
        | FeatureKind::Cylinder { .. }
        | FeatureKind::Sphere { .. }
        | FeatureKind::Torus { .. }
        | FeatureKind::Coil { .. } => {
            let (op, targets) = feature_op(f);
            bodies(operated(op, targets)?)
        }
        FeatureKind::Pattern { features, pattern: PatternKind::Rectangular { .. } | PatternKind::Circular { .. }, bodies: copied } => {
            if !copied.is_empty() {
                return bodies(named(&crate::plain::<String, _>(copied))?);
            }
            if features.is_empty() {
                return None;
            }
            let mut read = Vec::new();
            for n in crate::plain::<String, _>(features) {
                let src = replayed(doc, &n)?;
                let primitive = matches!(
                    src.kind,
                    FeatureKind::Box { .. }
                        | FeatureKind::Cylinder { .. }
                        | FeatureKind::Sphere { .. }
                        | FeatureKind::Torus { .. }
                        | FeatureKind::Coil { .. }
                );
                if !primitive || !plain_picks(src) {
                    return None;
                }
                let (op, targets) = feature_op(src);
                read.extend(operated(op, targets)?);
            }
            read.sort_unstable();
            read.dedup();
            bodies(read)
        }
        FeatureKind::Move { bodies: moved, .. } => bodies(named(&crate::plain::<String, _>(moved))?),
        FeatureKind::Sketch { plane, sketch } => {
            (fixed_plane(plane) && sketch.links.is_empty()).then(|| Reads { bodies: Vec::new(), sketches: Vec::new() })
        }
        FeatureKind::Extrude { sketch, extent, operation: Operation::NewBody, .. }
            if extent.to.is_none() && extent.from.is_none() && !extent.through_all && extent.taper.is_none() =>
        {
            Some(Reads { bodies: Vec::new(), sketches: vec![sketch_of(sketch)?] })
        }
        FeatureKind::Revolve { sketch, to: None, operation: Operation::NewBody, .. } => {
            Some(Reads { bodies: Vec::new(), sketches: vec![sketch_of(sketch)?] })
        }
        _ => None,
    }
}

/// A plane given by numbers alone (no face, edge or construction to find).
fn fixed_plane(plane: &PlaneRef) -> bool {
    match plane {
        PlaneRef::Origin { .. } | PlaneRef::Custom { .. } => true,
        PlaneRef::Offset { base, .. } | PlaneRef::AtAngle { base, .. } | PlaneRef::Flipped { base, .. } => fixed_plane(base),
        _ => false,
    }
}

/// No face or edge picked by name or reference (those are found on any body).
fn plain_picks(f: &Feature) -> bool {
    f.face_names.iter().all(String::is_empty) && f.edge_names.iter().all(String::is_empty) && f.edge_refs.is_empty()
}

/// The cache key of `f` from what it reads alone (see [`reads`]), with the bodies it reads by
/// index; `None` when it may read more.
pub(super) fn key(
    doc: &Document,
    vals: &BTreeMap<String, Value>,
    f: &Feature,
    ctx: u64,
    st: &ModelState,
    steps: &StepDigests,
) -> Option<(u64, Arc<[usize]>)> {
    let read = reads(doc, f, st)?;
    let mut h = DefaultHasher::new();
    "reads".hash(&mut h);
    (fingerprint(0, f, vals, false, steps) ^ placement_digest(doc, f)).hash(&mut h);
    ctx.hash(&mut h);
    f.id.hash(&mut h);
    // A pattern replays its features' definitions.
    if let FeatureKind::Pattern { features, .. } = &f.kind {
        for n in crate::plain::<String, _>(features) {
            replayed(doc, &n).map(|src| fingerprint(0, src, vals, false, steps)).hash(&mut h);
        }
    }
    // Every body's name and component (new names avoid them, a cut's pieces are numbered past
    // them, the component's bodies are a tool's default targets), the bodies read by content.
    for b in &st.bodies {
        b.name.hash(&mut h);
        b.component.hash(&mut h);
        b.transferred.hash(&mut h);
    }
    for &i in &read.bodies {
        i.hash(&mut h);
        if let Some(b) = st.bodies.get(i) {
            b.key.hash(&mut h);
            b.feature.hash(&mut h);
        }
    }
    for id in &read.sketches {
        id.hash(&mut h);
        st.sketch_keys.get(id).hash(&mut h);
    }
    st.body_counter.hash(&mut h);
    // How deep the naming history runs (names are not traced past a depth).
    st.bodies.iter().filter_map(|b| b.names.origin.as_ref().map(|o| o.depth)).max().hash(&mut h);
    // A cut drops the threads of bodies it removes.
    serde_json::to_string(&st.threads).unwrap_or_default().hash(&mut h);
    Some((h.finish(), read.bodies.into()))
}

/// A feature's result keyed by what it reads (see [`key`]): its output before it was stamped,
/// the bodies it passed through left out (they are taken from the state it is laid over), and
/// the sketch it solved.
#[derive(Clone, Debug)]
pub(super) struct Partial {
    bodies: Arc<[Out]>,
    sketch: Option<Arc<SolvedSketch>>,
    body_counter: usize,
    pub(super) warning: Option<String>,
    pub(super) boolean_diagnostics: kernel::BooleanDiagnostics,
    pub(super) ms: f64,
    /// What the bodies it made cost.
    bytes: usize,
}

#[derive(Clone, Debug)]
enum Out {
    /// The body at this index in the state before, unchanged.
    Passed(usize),
    /// A body the feature made or changed.
    Made(Box<ModelBody>),
}

impl Partial {
    /// The result of feature `f`, which read the bodies `read` and went from `before` to `after`
    /// (before [`ModelState::stamp`]); `None` unless it changed nothing else: every other body
    /// passed through, the sketches as they were but for its own, the construction, threads and
    /// blend records as they were.
    pub(super) fn of(
        f: &Feature,
        read: &[usize],
        before: &ModelState,
        after: &ModelState,
        warning: Option<String>,
        boolean_diagnostics: kernel::BooleanDiagnostics,
        ms: f64,
    ) -> Option<Partial> {
        let others: Vec<&Arc<SolvedSketch>> = before.sketches.iter().filter(|s| s.feature != f.id).collect();
        let (kept, sketch) = match after.sketches.split_last() {
            Some((last, rest)) if last.feature == f.id => (rest, Some(last.clone())),
            _ => (after.sketches.as_slice(), None),
        };
        let same_rest = after.sheets.is_empty()
            && kept.len() == others.len()
            && kept.iter().zip(&others).all(|(a, b)| Arc::ptr_eq(a, b))
            && (sketch.is_some() || kept.len() == before.sketches.len())
            && after.sketch_keys == before.sketch_keys
            && after.construct.len() == before.construct.len()
            && after.blend_edges == before.blend_edges
            && after.fitted == before.fitted
            && after.tapered.len() == before.tapered.len()
            && serde_json::to_string(&after.threads).ok() == serde_json::to_string(&before.threads).ok();
        if !same_rest {
            return None;
        }
        let mut from = 0;
        let mut passed = vec![false; before.bodies.len()];
        let mut bodies = Vec::with_capacity(after.bodies.len());
        for b in &after.bodies {
            let at = before.bodies.iter().skip(from).position(|x| x.name == b.name).map(|k| k + from);
            match at.filter(|&j| before.bodies.get(j).is_some_and(|x| x.key == b.key && x.body.same(&b.body))) {
                Some(j) => {
                    bodies.push(Out::Passed(j));
                    if let Some(p) = passed.get_mut(j) {
                        *p = true;
                    }
                    from = j + 1;
                }
                None => bodies.push(Out::Made(Box::new(b.clone()))),
            }
        }
        // Every body it did not read went through untouched.
        if !passed.iter().enumerate().all(|(j, p)| *p || read.contains(&j)) {
            return None;
        }
        let bytes = bodies
            .iter()
            .map(|o| match o {
                Out::Made(b) => b.body.retained_bytes(),
                Out::Passed(_) => 0,
            })
            .sum::<usize>()
            + bodies.len() * 64
            + sketch.as_ref().map_or(0, |s| s.retained_bytes());
        Some(Partial { bodies: bodies.into(), sketch, body_counter: after.body_counter, warning, boolean_diagnostics, ms, bytes })
    }
}

/// `part`, the result of feature `feature`, laid over `now`: its bodies, with those it passed
/// through taken from `now` (as the rest of the model is now), its sketch, and the rest of
/// `now`. `None` when `now` does not line up with it.
pub(super) fn splice(part: &Partial, feature: u64, now: &ModelState) -> Option<ModelState> {
    let mut bodies = Vec::with_capacity(part.bodies.len());
    for o in part.bodies.iter() {
        bodies.push(match o {
            Out::Passed(j) => {
                let mut b = now.bodies.get(*j)?.clone();
                b.made = crate::lineage::Made::Before;
                b
            }
            Out::Made(b) => (**b).clone(),
        });
    }
    let mut sketches = now.sketches.clone();
    if let Some(s) = &part.sketch {
        sketches.retain(|s| s.feature != feature);
        sketches.push(s.clone());
    }
    Some(ModelState {
        threads: now.threads.clone(),
        bodies,
        sketches,
        body_counter: part.body_counter,
        sheets: now.sheets.clone(),
        construct: now.construct.clone(),
        sketch_keys: now.sketch_keys.clone(),
        blend_edges: now.blend_edges.clone(),
        fitted: now.fitted.clone(),
        tapered: now.tapered.clone(),
    })
}

/// Results keyed by what they read (oldest dropped first), apart from the whole-model results:
/// each of those counts every body of the model, so in a large design one of them fills the
/// budget.
#[derive(Debug, Default)]
pub(super) struct PartialCache {
    map: HashMap<u64, Partial>,
    order: VecDeque<u64>,
    bytes: usize,
}

/// Bytes of partial results kept: the bodies they made.
const PARTIAL_BYTES: usize = 256 << 20;

impl PartialCache {
    pub(super) fn get(&self, key: u64) -> Option<Partial> {
        self.map.get(&key).cloned()
    }

    pub(super) fn put(&mut self, key: u64, r: Partial) {
        let bytes = r.bytes;
        match self.map.insert(key, r) {
            Some(old) => self.bytes = self.bytes.saturating_sub(old.bytes),
            None => self.order.push_back(key),
        }
        self.bytes = self.bytes.saturating_add(bytes);
        while self.order.len() > super::CACHE_ENTRIES || (self.bytes > PARTIAL_BYTES && self.order.len() > 1) {
            if let Some(k) = self.order.pop_front()
                && let Some(old) = self.map.remove(&k)
            {
                self.bytes = self.bytes.saturating_sub(old.bytes);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::*;

    fn feature(doc: &mut Document, component: u64, name: &str, kind: serde_json::Value) {
        let kind: FeatureKind = serde_json::from_value(kind).unwrap();
        let id = doc.add_feature(kind, Some(name)).unwrap();
        doc.feature_mut(id).unwrap().component = component;
    }

    /// A plate in one component, then a rod in another, slotted by a box cut patterned round it.
    fn two_parts(slot_targets: &[&str]) -> Document {
        let mut doc = Document::new("two parts");
        doc.set_param("w", "40 mm", None, None).unwrap();
        doc.set_param("slot", "1 mm", None, None).unwrap();
        let plate = doc.add_component(Some("Plate"), 0).unwrap();
        let rod = doc.add_component(Some("Rod"), 0).unwrap();
        feature(
            &mut doc,
            plate,
            "Plate",
            json!({"type": "box", "corner": ["0", "0", "0"], "length": "w", "width": "30", "height": "5", "operation": "new_body"}),
        );
        feature(
            &mut doc,
            rod,
            "Rod",
            json!({"type": "cylinder", "base": ["0", "0", "0"], "axis": [0.0, 0.0, 1.0], "radius": "4", "height": "20", "operation": "new_body"}),
        );
        feature(
            &mut doc,
            rod,
            "Slot",
            json!({"type": "box", "corner": ["3", "-0.5", "2"], "length": "2", "width": "slot", "height": "20", "operation": "cut"}),
        );
        for f in doc.features.iter_mut().filter(|f| f.name == "Plate" || f.name == "Rod") {
            f.body_names = vec![f.name.clone()];
        }
        if let Some(f) = doc.features.iter_mut().find(|f| f.name == "Slot") {
            f.participants = slot_targets.iter().map(|t| t.to_string()).collect();
        }
        feature(
            &mut doc,
            rod,
            "Slots",
            json!({"type": "pattern", "features": ["Slot"], "pattern": {"type": "circular", "origin": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "count": "4", "angle": "360 deg"}}),
        );
        doc
    }

    /// The bodies as an evaluation from scratch makes them: names, volumes, face names.
    fn bodies(st: &ModelState) -> Vec<(String, u64, i64, Vec<String>)> {
        st.bodies
            .iter()
            .map(|b| {
                let volume = solidcraft_kernel::measure(&b.body).unwrap().volume;
                (b.name.clone(), b.component, (volume * 1e6).round() as i64, crate::naming::face_names(b).to_vec())
            })
            .collect()
    }

    fn fresh(doc: &Document) -> Vec<(String, u64, i64, Vec<String>)> {
        let mut m = Model::new();
        m.no_cache = true;
        m.evaluate(doc);
        bodies(&m.state())
    }

    /// An edit in one component leaves another component's features after it as they were: they
    /// are restored, not recomputed, and give what evaluating from scratch gives.
    #[test]
    fn edit_in_one_component_keeps_the_others_results() {
        let mut doc = two_parts(&[]);
        let mut m = Model::new();
        m.evaluate(&doc);
        assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
        assert_eq!(m.last_recomputed, 4);
        doc.set_param("w", "50 mm", None, None).unwrap();
        m.evaluate(&doc);
        assert_eq!((m.last_recomputed, m.last_restored), (1, 3));
        assert_eq!(bodies(&m.state()), fresh(&doc));
        // An edit in the rod's own component recomputes its features.
        doc.set_param("slot", "1.5 mm", None, None).unwrap();
        m.evaluate(&doc);
        assert_eq!((m.last_recomputed, m.last_restored), (2, 0));
        assert_eq!(bodies(&m.state()), fresh(&doc));
        doc.set_param("w", "60 mm", None, None).unwrap();
        m.evaluate(&doc);
        assert_eq!((m.last_recomputed, m.last_restored), (1, 3));
        assert_eq!(bodies(&m.state()), fresh(&doc));
    }

    /// A sketch on a fixed plane, the new body extruded from it and that body moved read nothing
    /// of another component either.
    #[test]
    fn sketch_extrude_and_move_keep_their_results() {
        let mut doc = two_parts(&[]);
        let rod = doc.find_component("Rod").unwrap();
        let mut sk = solidcraft_sketch::Sketch::new();
        sk.add_circle(solidcraft_geom::Vec2::new(20.0, 0.0), 2.0, None, None).unwrap();
        let id = doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }, Some("Peg sketch")).unwrap();
        doc.feature_mut(id).unwrap().component = rod;
        feature(
            &mut doc,
            rod,
            "Peg",
            json!({"type": "extrude", "sketch": id, "profiles": {"type": "all"}, "extent": {"convention": "total_distance", "direction": "positive", "distance": "8"}, "operation": "new_body"}),
        );
        if let Some(f) = doc.features.iter_mut().find(|f| f.name == "Peg") {
            f.body_names = vec!["Peg".into()];
        }
        feature(&mut doc, rod, "Peg up", json!({"type": "move", "bodies": ["Peg"], "translate": ["0", "0", "slot * 3"]}));
        let mut m = Model::new();
        m.evaluate(&doc);
        assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
        doc.set_param("w", "50 mm", None, None).unwrap();
        m.evaluate(&doc);
        assert_eq!((m.last_recomputed, m.last_restored), (1, 6));
        assert_eq!(bodies(&m.state()), fresh(&doc));
        // The move follows its own parameter.
        doc.set_param("slot", "2 mm", None, None).unwrap();
        m.evaluate(&doc);
        assert_eq!((m.last_recomputed, m.last_restored), (3, 2));
        assert_eq!(bodies(&m.state()), fresh(&doc));
    }

    /// A feature that cuts a body of another component reads it: it is recomputed when that body
    /// changes, and so is what follows it in its own component.
    #[test]
    fn a_cut_of_another_components_body_follows_it() {
        let mut doc = two_parts(&["Plate", "Rod"]);
        let mut m = Model::new();
        m.evaluate(&doc);
        assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
        doc.set_param("w", "50 mm", None, None).unwrap();
        m.evaluate(&doc);
        assert_eq!((m.last_recomputed, m.last_restored), (3, 1));
        assert_eq!(bodies(&m.state()), fresh(&doc));
    }
}
