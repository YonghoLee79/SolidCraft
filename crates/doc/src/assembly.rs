//! Components and occurrences: the design as a tree.
//!
//! The root component (id 0) is the design. Every other component lives in a parent component
//! through one or more occurrences, each with a rigid transform (instances share the component's
//! contents; a "Paste New" copy is a separate component). A component's sketches, construction
//! geometry and features are authored in its own frame (its origin); the model is evaluated in
//! those frames and placed in the world through the occurrence transforms when bodies are
//! gathered for display, measuring and export (`world_bodies`). Commands take world coordinates
//! and the engine maps a new feature's geometry into the active component's frame.

use serde::{Deserialize, Serialize};
use solidcraft_geom::{Transform, Vec3};

use crate::{DocError, Document, FeatureKind, PlaneRef, Result};

/// A placement of a component inside a parent component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Occurrence {
    pub id: u64,
    pub component: u64,
    /// The component it sits in (0 = the root).
    #[serde(default)]
    pub parent: u64,
    pub name: String,
    #[serde(default)]
    pub transform: Transform,
    /// Grounded occurrences don't move.
    #[serde(default)]
    pub grounded: bool,
}

/// How deep and how many placements [`Document::placements`] walks.
pub const MAX_PLACEMENT_DEPTH: usize = 64;
pub const MAX_PLACEMENTS: usize = 100_000;

fn map_plane_ref(m: &Transform, p: &mut PlaneRef) {
    match p {
        PlaneRef::Custom { plane } => *plane = m.plane(plane),
        PlaneRef::Placed { origin, x_dir, y_dir } => {
            map_point_expr(m, origin);
            *x_dir = m.vector(*x_dir);
            *y_dir = m.vector(*y_dir);
        }
        PlaneRef::Face { plane, at, .. } => {
            *plane = m.plane(plane);
            *at = m.point(*at);
        }
        PlaneRef::Offset { base, .. } | PlaneRef::Flipped { base, .. } => map_plane_ref(m, base),
        PlaneRef::AtAngle { base, axis_origin, axis_dir, .. } => {
            map_plane_ref(m, base);
            *axis_origin = m.point(*axis_origin);
            *axis_dir = m.vector(*axis_dir);
        }
        // Named origin planes and construction planes are the component's own.
        PlaneRef::Origin { .. } | PlaneRef::Construction { .. } => {}
        PlaneRef::Tangent { face, at } => {
            *at = m.point(*at);
            map_geo_ref(m, face);
        }
        PlaneRef::Midplane { .. }
        | PlaneRef::TwoEdges { .. }
        | PlaneRef::ThreePoints { .. }
        | PlaneRef::AlongPath { .. }
        | PlaneRef::Perpendicular { .. } => {
            for r in p.refs_mut() {
                map_geo_ref(m, r);
            }
        }
    }
}

fn map_geo_ref(m: &Transform, g: &mut crate::construct::GeoRef) {
    g.map_points(&mut |p: &mut Vec3| *p = m.point(*p), &mut |pl: &mut PlaneRef| map_plane_ref(m, pl));
}

/// Map a point given as expressions by `m`: numbers stay numbers; a coordinate that is an
/// expression stays one (`m · (x, y, z) + t`, written out), so it still follows its parameters.
pub fn map_point_expr(m: &Transform, p: &mut crate::PointExpr) {
    let num = |e: &crate::Expr| e.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    if let [Some(x), Some(y), Some(z)] = [num(&p[0]), num(&p[1]), num(&p[2])] {
        *p = crate::point_expr(m.point(Vec3::new(x, y, z)));
        return;
    }
    let old = p.clone();
    for (row, out) in p.iter_mut().enumerate() {
        let mut constant = m[3][row];
        let mut terms: Vec<String> = Vec::new();
        for (col, e) in old.iter().enumerate() {
            let c = m[col][row];
            if c.abs() < 1e-12 {
                continue;
            }
            match num(e) {
                Some(v) => constant += c * v,
                None if (c - 1.0).abs() < 1e-12 => terms.push(format!("({e})")),
                None => terms.push(format!("{c} * ({e})")),
            }
        }
        let mut e = terms.join(" + ");
        if terms.is_empty() {
            e = format!("{constant} mm");
        } else if constant.abs() >= 1e-12 {
            e = format!("{e} {} {} mm", if constant < 0.0 { '-' } else { '+' }, constant.abs());
        }
        *out = e.into();
    }
}

impl FeatureKind {
    /// Map the feature's world geometry (points, directions, explicit planes) by `m`.
    pub fn transform_geometry(&mut self, m: &Transform) {
        use crate::feature_geometry::Geo;
        use crate::walk::Walk;
        self.walk_geometry_mut(&mut |g| match g {
            Geo::Pick(p) | Geo::Pos(p) => *p = m.point(*p),
            Geo::Dir(v) => *v = m.vector(*v),
            Geo::Plane(p) => map_plane_ref(m, p),
            Geo::Construct(r) => map_geo_ref(m, r),
            Geo::Placement(e) => map_point_expr(m, e),
            // An Emboss cylinder is picked in its component's frame (`EmbossFace`).
            Geo::Pinned(_) | Geo::Profiles(..) => {}
        });
    }
}

impl Document {
    /// The first occurrence of a component (its placement for authoring).
    pub fn occurrence_of(&self, component: u64) -> Option<&crate::Occurrence> {
        self.occurrences.iter().find(|o| o.component == component)
    }

    /// World transform of a component's frame (through its first occurrence and its parents').
    pub fn component_transform(&self, component: u64) -> Transform {
        let mut m = Transform::IDENTITY;
        let mut c = component;
        for _ in 0..1000 {
            if c == 0 {
                break;
            }
            let Some(o) = self.occurrence_of(c) else { break };
            m = o.transform * m;
            c = o.parent;
        }
        m
    }

    /// World transform of one placement of `component`: the occurrence `path` from the root
    /// (empty for the root itself). `None` when the path no longer leads to that component.
    pub fn path_transform(&self, component: u64, path: &[u64]) -> Option<Transform> {
        if path.len() > MAX_PLACEMENT_DEPTH + 1 {
            return None;
        }
        let (mut m, mut c) = (Transform::IDENTITY, 0);
        for id in path {
            let o = self.occurrences.iter().find(|o| o.id == *id && o.parent == c)?;
            m = m * o.transform;
            c = o.component;
        }
        (c == component).then_some(m)
    }

    /// Every placement of each component: (component, occurrence path ids, world transform).
    /// A branch stops past [`MAX_PLACEMENT_DEPTH`] levels, the walk past [`MAX_PLACEMENTS`] placements.
    pub fn placements(&self) -> Vec<(u64, Vec<u64>, Transform)> {
        let mut out = vec![(0, Vec::new(), Transform::IDENTITY)];
        let mut stack: Vec<(u64, Vec<u64>, Transform)> = vec![(0, Vec::new(), Transform::IDENTITY)];
        while let Some((comp, path, m)) = stack.pop() {
            if out.len() > MAX_PLACEMENTS {
                break;
            }
            // Only this branch stops past the depth limit; the others are still walked.
            if path.len() > MAX_PLACEMENT_DEPTH {
                continue;
            }
            for o in self.occurrences.iter().filter(|o| o.parent == comp) {
                let mut p = path.clone();
                p.push(o.id);
                let w = m * o.transform;
                out.push((o.component, p.clone(), w));
                stack.push((o.component, p, w));
            }
        }
        out
    }

    /// Every body as the world shows it: its world name ("Body1", or "Body1 (Occ:2)" for a
    /// further instance), its component, the occurrence path, and the transform from the frame
    /// its geometry is in to the world.
    pub fn body_frames(&self, st: &crate::ModelState) -> Vec<(String, u64, Vec<u64>, Transform)> {
        self.body_frames_with_source(st).into_iter().map(|(world, _, component, path, matrix)| (world, component, path, matrix)).collect()
    }

    /// Body frames with explicit definition names; callers must not infer them from labels.
    pub fn body_frames_with_source(&self, st: &crate::ModelState) -> Vec<(String, String, u64, Vec<u64>, Transform)> {
        let mut out = Vec::new();
        for (comp, path, m) in self.placements() {
            for b in st.bodies.iter().filter(|b| self.model_body_component(b) == comp) {
                let name = self.world_body_name(st, &b.name, comp, &path);
                let w = match self.model_body_offset(b) {
                    Some(o) => m * o,
                    None => m,
                };
                out.push((name, b.definition_name().to_string(), comp, path.clone(), w));
            }
        }
        out
    }

    /// Computed placement name. Source names retain priority; ambiguous labels use full IDs.
    /// Resolves label ambiguity with bounded dynamic counts instead of expanding all placements.
    pub fn world_body_name(&self, st: &crate::ModelState, source: &str, component: u64, path: &[u64]) -> String {
        if path == self.first_path(component) {
            return source.to_string();
        }
        let names: Vec<&str> = path.iter().filter_map(|id| self.occurrences.iter().find(|o| o.id == *id)).map(|o| o.name.as_str()).collect();
        let label = names.join("/");
        let candidate = format!("{source} ({label})");
        let mut counts = std::collections::BTreeMap::from([(0, 1usize)]);
        for name in &names {
            let mut next = std::collections::BTreeMap::<u64, usize>::new();
            for o in self.occurrences.iter().filter(|o| o.name == *name) {
                if let Some(count) = counts.get(&o.parent) {
                    let n = next.entry(o.component).or_default();
                    *n = (*n + count).min(2);
                }
            }
            counts = next;
        }
        let ambiguous = counts.get(&component).copied().unwrap_or(0) > 1;
        // Slash and reserved-marker names could alias concatenated labels or fallback keys.
        let reserved =
            self.occurrences.iter().any(|o| o.name.contains('/') || o.name.contains("@occ[")) || st.bodies.iter().any(|b| b.name.contains("@occ["));
        if !ambiguous && !reserved && st.body(&candidate).is_none() {
            return candidate;
        }
        let ids = path.iter().map(u64::to_string).collect::<Vec<_>>().join("/");
        let encoded = serde_json::to_string(source).unwrap_or_else(|_| source.to_string());
        let mut key = format!("{encoded}@occ[{ids}]");
        while st.body(&key).is_some() {
            key.push('#');
        }
        key
    }

    /// The occurrence path of a component's first placement.
    pub fn first_path(&self, component: u64) -> Vec<u64> {
        let mut path = Vec::new();
        let mut c = component;
        for _ in 0..1000 {
            if c == 0 {
                break;
            }
            let Some(o) = self.occurrence_of(c) else { break };
            path.insert(0, o.id);
            c = o.parent;
        }
        path
    }

    /// Is `a` inside `b` (or `b` itself)?
    pub fn component_within(&self, a: u64, b: u64) -> bool {
        let mut c = a;
        for _ in 0..1000 {
            if c == b {
                return true;
            }
            if c == 0 {
                return false;
            }
            match self.components.iter().find(|x| x.id == c) {
                Some(x) => c = x.parent,
                None => return false,
            }
        }
        false
    }

    /// Add an occurrence of an existing component into `parent`.
    pub fn add_occurrence(&mut self, component: u64, parent: u64, transform: Transform) -> Result<u64> {
        if component == 0 || !self.components.iter().any(|c| c.id == component) {
            return Err(DocError::Unknown(format!("component {component}")));
        }
        if self.component_within(parent, component) {
            return Err(DocError::Invalid("a component can't contain itself".into()));
        }
        if self.occurrences.len() >= 100_000 {
            return Err(DocError::Invalid("too many occurrences".into()));
        }
        let id = self.next_id.max(1);
        self.next_id = id.checked_add(1).ok_or_else(|| DocError::Invalid("too many items in the design".into()))?;
        let cname = self.components.iter().find(|c| c.id == component).map(|c| c.name.clone()).unwrap_or_default();
        let k = self.occurrences.iter().filter(|o| o.component == component).count() + 1;
        self.occurrences.push(crate::Occurrence { id, component, parent, name: format!("{cname}:{k}"), transform, grounded: false });
        Ok(id)
    }
}
