//! 3D sketching in the viewport (the Sketch Palette's 3D Sketch option): the Line, Spline and
//! Point tools take points off the sketch plane by snapping to model vertices, edges and faces
//! and to points already drawn in 3D; segments that stay on the plane are ordinary sketch lines.
//! A drawn 3D point (or curve end) clicked without a tool gets a move triad: drag an arrow to
//! move along X, Y or Z, or the square to move parallel to the sketch plane. So does a point on
//! the plane (a line's end, a spline's fit point, a lone point): moved within the plane it stays
//! an ordinary sketch point (`sketch.move_point`); moved off it, the lines and fit-point splines
//! through it become drawn 3D curves (`sketch.move3d` with `point`). Everything runs
//! `sketch.line3d`, `sketch.spline3d`, `sketch.point3d`, `sketch.move3d`, `sketch.move_point` and
//! `sketch.line`.

use std::cell::RefCell;

use egui::{Pos2, Rect, Shape, Stroke, vec2};
use serde_json::{Value, json};
use solidcraft_engine::geom::{Vec2, Vec3};

use crate::SolveApp;
use crate::theme::Tokens;
use crate::tools::Tool;
use crate::viewport::{Hit, Proj, pick_3d};

thread_local! {
    /// Points of the curve being drawn in 3D (with the tool they belong to).
    static CHAIN: RefCell<(Option<&'static str>, Vec<Vec3>)> = const { RefCell::new((None, Vec::new())) };
    /// The point with the triad.
    static TRIAD: RefCell<Option<Grab>> = const { RefCell::new(None) };
    /// A triad drag in progress.
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    /// Where the cursor would put a point (for the rubber band), and its snap label.
    static HOVER: RefCell<Option<(Vec3, &'static str)>> = const { RefCell::new(None) };
}

/// What the triad holds.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    /// A drawn 3D point: wire id and fit index.
    Wire(String, usize),
    /// A sketch point on the plane, by id.
    Plane(String),
}

/// Triad arrow length (screen px).
const ARROW: f32 = 60.0;

/// Is the sketch being edited a 3D sketch?
pub fn active(app: &SolveApp) -> bool {
    let st = app.session.world_state();
    app.session.active_sketch.and_then(|s| st.sketch(s)).is_some_and(|ss| ss.sketch.view.three_d)
}

fn takes(cmd: &str) -> bool {
    matches!(cmd, "sketch.line" | "sketch.spline.fit_point" | "sketch.point")
}

/// Only 3D point-placing tools broaden sketch-point picking; ordinary selection stays local.
pub fn drawing(app: &SolveApp) -> bool {
    active(app) && app.tool.as_ref().is_some_and(|tool| takes(tool.cmd))
}

/// Points drawn in 3D: (wire id, fit index, point).
fn drawn_points(app: &SolveApp) -> Vec<(String, usize, Vec3)> {
    let st = app.session.world_state();
    let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else { return Vec::new() };
    ss.sketch.wires.iter().flat_map(|w| w.fit.iter().enumerate().map(|(i, p)| (w.id.clone(), i, *p))).collect()
}

/// Points on the plane that can leave it: the ends of lines, the fit points of fit-point splines
/// and lone points (not the origin, not projected geometry): (point id, world point).
fn plane_points(app: &SolveApp) -> Vec<(String, Vec3)> {
    use solidcraft_engine::sketch::CurveKind;
    let st = app.session.world_state();
    let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else { return Vec::new() };
    let sk = &ss.sketch;
    let mut take = vec![false; sk.points.len()];
    let mut used = vec![false; sk.points.len()];
    for c in &sk.curves {
        for i in c.kind.point_ids() {
            if let Some(u) = used.get_mut(i) {
                *u = true;
            }
        }
        if c.link.is_some() {
            continue;
        }
        let ids = match &c.kind {
            CurveKind::Line { a, b } => vec![*a, *b],
            CurveKind::Spline { pts, control: false, periodic: false, .. } => pts.clone(),
            _ => Vec::new(),
        };
        for i in ids {
            if let Some(t) = take.get_mut(i) {
                *t = true;
            }
        }
    }
    sk.points
        .iter()
        .enumerate()
        .filter(|(i, q)| *i != 0 && q.link.is_none() && (take.get(*i).copied().unwrap_or(false) || !used.get(*i).copied().unwrap_or(true)))
        .map(|(_, q)| (q.id.clone(), ss.plane.to_world(q.pos)))
        .collect()
}

/// Closest point of segment a–b to the ray (o, d).
fn seg_ray_closest(a: Vec3, b: Vec3, o: Vec3, d: Vec3) -> Vec3 {
    let u = b - a;
    let w0 = a - o;
    let (aa, bb, cc) = (u.dot(u), u.dot(d), d.dot(d));
    let (dd, ee) = (u.dot(w0), d.dot(w0));
    let den = aa * cc - bb * bb;
    let s = if den.abs() < 1e-12 { 0.0 } else { ((bb * ee - cc * dd) / den).clamp(0.0, 1.0) };
    a + u * s
}

/// The 3D point under the cursor and what it snapped to: a drawn 3D point, a model vertex, a
/// point on a model edge or face, a sketch point, or the sketch plane.
pub fn point_at(app: &SolveApp, proj: &Proj, pos: Pos2) -> Option<(Vec3, &'static str)> {
    for (_, _, p) in drawn_points(app) {
        if proj.to_screen(p).is_some_and(|s| s.distance(pos) < 8.0) {
            return Some((p, "Pt"));
        }
    }
    let st = app.session.world_state();
    let ss = app.session.active_sketch.and_then(|s| st.sketch(s))?;
    let (o, d) = proj.ray(pos);
    for h in pick_3d(app, proj, pos) {
        match h {
            Hit::SketchPoint { sketch, at, .. } => {
                if let Some(source) = st.sketch(sketch) {
                    return Some((source.plane.to_world(at), "Pt"));
                }
            }
            Hit::Vertex { point, .. } => return Some((point, "Vertex")),
            Hit::Edge { body, index, .. } => {
                let ws = app.session.world_state();
                let m = ws.body(&body)?.mesh();
                let e = m.edges.get(index)?;
                let best = e
                    .windows(2)
                    .map(|w| seg_ray_closest(w[0], w[1], o, d))
                    .min_by(|p, q| (*p - o).cross(d).len().total_cmp(&(*q - o).cross(d).len()))?;
                return Some((best, "Edge"));
            }
            Hit::Face { point, .. } => return Some((point, "Face")),
            _ => {}
        }
    }
    ss.plane.intersect_ray(o, d).map(|w| (w, "Plane"))
}

/// The sketch-plane coordinates of `p` when it lies on the plane.
fn on_plane(app: &SolveApp, p: Vec3) -> Option<Vec2> {
    let st = app.session.world_state();
    let ss = app.session.active_sketch.and_then(|s| st.sketch(s))?;
    let q = ss.plane.to_local(p);
    (ss.plane.to_world(q).dist(p) < 1e-6).then_some(q)
}

fn chain() -> Vec<Vec3> {
    CHAIN.with(|c| c.borrow().1.clone())
}

fn set_chain(cmd: Option<&'static str>, pts: Vec<Vec3>) {
    CHAIN.with(|c| *c.borrow_mut() = (cmd, pts));
}

/// The pointer moved: remember where a click would put the point.
pub fn on_hover(app: &SolveApp, proj: &Proj, pos: Pos2) {
    let h = if app.tool.as_ref().is_some_and(|t| takes(t.cmd)) && active(app) { point_at(app, proj, pos) } else { None };
    HOVER.with(|x| *x.borrow_mut() = h);
}

/// A click while a drawing tool is active in a 3D sketch (true: handled here).
pub fn on_click(app: &mut SolveApp, tool: &mut Tool, proj: &Proj, pos: Pos2) -> bool {
    if !takes(tool.cmd) || !active(app) {
        return false;
    }
    let Some((p, _)) = point_at(app, proj, pos) else { return true };
    if CHAIN.with(|c| c.borrow().0) != Some(tool.cmd) {
        set_chain(Some(tool.cmd), Vec::new());
    }
    match tool.cmd {
        "sketch.point" => {
            let r = match on_plane(app, p) {
                Some(q) => app.run("sketch.point", json!({"point": [q.x, q.y]})),
                None => app.run("sketch.point3d", json!({"point": [p.x, p.y, p.z]})),
            };
            report(app, r);
        }
        "sketch.line" => {
            if let Some(a) = chain().last().copied() {
                let r = match (on_plane(app, a), on_plane(app, p)) {
                    (Some(qa), Some(qb)) => app.run("sketch.line", json!({"points": [[qa.x, qa.y], [qb.x, qb.y]], "infer": true})),
                    _ => app.run("sketch.line3d", json!({"points": [[a.x, a.y, a.z], [p.x, p.y, p.z]]})),
                };
                report(app, r);
            }
            let mut c = chain();
            c.push(p);
            set_chain(Some(tool.cmd), c);
        }
        _ => {
            let mut c = chain();
            c.push(p);
            set_chain(Some(tool.cmd), c);
        }
    }
    true
}

fn report(app: &mut SolveApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.set_status(e, true);
    }
}

/// Right-click / Esc while drawing in 3D: a spline is made from its points, a line chain ends
/// (true: handled here; the tool stays for the next curve).
pub fn finish(app: &mut SolveApp) -> bool {
    let (cmd, pts) = CHAIN.with(|c| c.borrow().clone());
    let Some(cmd) = cmd.filter(|c| app.tool.as_ref().is_some_and(|t| t.cmd == *c)) else {
        set_chain(None, Vec::new());
        return false;
    };
    if pts.is_empty() {
        set_chain(None, Vec::new());
        return false;
    }
    if cmd == "sketch.spline.fit_point" && pts.len() >= 2 {
        let r = if pts.iter().all(|p| on_plane(app, *p).is_some()) {
            let q: Vec<Value> = pts.iter().filter_map(|p| on_plane(app, *p)).map(|q| json!([q.x, q.y])).collect();
            app.run("sketch.spline.fit_point", json!({ "points": q }))
        } else {
            let q: Vec<Value> = pts.iter().map(|p| json!([p.x, p.y, p.z])).collect();
            app.run("sketch.spline3d", json!({ "points": q }))
        };
        report(app, r);
    }
    set_chain(Some(cmd), Vec::new());
    true
}

/// Rubber band and snap marker of the curve being drawn in 3D (true: drawn here).
pub fn preview(app: &SolveApp, t: &Tool, painter: &egui::Painter, proj: &Proj) -> bool {
    if !takes(t.cmd) || !active(app) {
        return false;
    }
    let tk = Tokens::get();
    let stroke = Stroke::new(1.5, tk.sketch_accent);
    let pts = if CHAIN.with(|c| c.borrow().0) == Some(t.cmd) { chain() } else { Vec::new() };
    let hover = HOVER.with(|h| *h.borrow());
    let mut all: Vec<Pos2> = pts.iter().filter_map(|p| proj.to_screen(*p)).collect();
    if let Some((h, label)) = hover
        && let Some(s) = proj.to_screen(h)
    {
        if t.cmd != "sketch.point" && !pts.is_empty() {
            all.push(s);
        }
        painter.circle_stroke(s, if label == "Plane" { 3.0 } else { 6.0 }, stroke);
        if label != "Plane" {
            painter.text(
                s + vec2(10.0, -14.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(crate::theme::Tokens::get().font_micro),
                tk.text,
            );
        }
        painter.text(
            s + vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!("{}, {}, {}", crate::prefs::show_mm(app, h.x, 1), crate::prefs::show_mm(app, h.y, 1), crate::prefs::show_mm(app, h.z, 1)),
            egui::FontId::proportional(crate::theme::Tokens::get().font_small),
            tk.text_dim,
        );
    }
    if t.cmd == "sketch.spline.fit_point" && all.len() >= 2 {
        let mut w: Vec<Vec3> = pts.clone();
        if let Some((h, _)) = hover {
            w.push(h);
        }
        let poly: Vec<Pos2> = solidcraft_engine::sketch::fit_polyline(&w).into_iter().filter_map(|p| proj.to_screen(p)).collect();
        painter.add(Shape::line(poly, stroke));
    } else {
        painter.add(Shape::line(all, stroke));
    }
    true
}

// ---------------------------------------------------------------------------------------------
// The move triad

fn triad_point(app: &SolveApp) -> Option<(Grab, Vec3)> {
    let grab = TRIAD.with(|t| t.borrow().clone())?;
    let at = match &grab {
        Grab::Wire(w, i) => drawn_points(app).into_iter().find(|(id, k, _)| id == w && k == i).map(|(_, _, p)| p),
        Grab::Plane(id) => plane_points(app).into_iter().find(|(q, _)| q == id).map(|(_, p)| p),
    }?;
    Some((grab, at))
}

fn unit(v: egui::Vec2) -> Option<egui::Vec2> {
    (v.length() > 1e-3).then(|| v / v.length())
}

/// Where an arrow points on screen when its axis points (nearly) at the viewer, as Z does in the
/// sketch's own top view: up and to the left, so all three arrows can always be grabbed.
const HEAD_ON_DIR: egui::Vec2 = egui::Vec2::new(-std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);

/// Each axis on screen at `p`: (axis, screen direction, head-on?) and the view's scale there in
/// pixels per mm. An axis drawn shorter than a sixth of the longest is head-on: it gets
/// [`HEAD_ON_DIR`], and dragging it moves the point by the drag's length at that scale.
fn axes_on_screen(proj: &Proj, p: Vec3) -> Option<(Vec<(Vec3, egui::Vec2, bool)>, f32)> {
    let base = proj.to_screen(p)?;
    let drawn: Vec<(Vec3, egui::Vec2)> = [Vec3::X, Vec3::Y, Vec3::Z].into_iter().filter_map(|a| Some((a, proj.to_screen(p + a)? - base))).collect();
    let longest = drawn.iter().map(|(_, v)| v.length()).fold(0.0f32, f32::max);
    if longest < 1e-6 {
        return None;
    }
    let axes = drawn
        .into_iter()
        .map(|(a, v)| match unit(v).filter(|_| v.length() >= longest / 6.0) {
            Some(dir) => (a, dir, false),
            None => (a, HEAD_ON_DIR, true),
        })
        .collect();
    Some((axes, longest))
}

/// The triad's handles on screen: (axis or None for the plane square, tip, base).
fn handles(app: &SolveApp, proj: &Proj) -> Vec<(Option<Vec3>, Pos2, Pos2)> {
    let Some((_, p)) = triad_point(app) else { return Vec::new() };
    let Some(base) = proj.to_screen(p) else { return Vec::new() };
    let Some((axes, _)) = axes_on_screen(proj, p) else { return Vec::new() };
    let mut out: Vec<_> = axes.into_iter().map(|(axis, dir, _)| (Some(axis), base + dir * ARROW, base)).collect();
    out.push((None, base + vec2(14.0, -14.0), base));
    out
}

/// A click without a tool: a drawn 3D point gets the triad (true: handled).
pub fn click(app: &mut SolveApp, proj: &Proj, pos: Pos2) -> bool {
    if !active(app) {
        TRIAD.with(|t| *t.borrow_mut() = None);
        return false;
    }
    let hit = drawn_points(app).into_iter().find(|(_, _, p)| proj.to_screen(*p).is_some_and(|s| s.distance(pos) < 8.0));
    let on_handle = handles(app, proj).iter().any(|(_, tip, _)| tip.distance(pos) < 9.0);
    if on_handle {
        return true;
    }
    if hit.is_none() {
        // A point on the plane gets the triad too, and the click still selects it as usual.
        let on_plane = plane_points(app).into_iter().find(|(_, p)| proj.to_screen(*p).is_some_and(|s| s.distance(pos) < 8.0));
        TRIAD.with(|t| *t.borrow_mut() = on_plane.map(|(id, _)| Grab::Plane(id)));
        return false;
    }
    TRIAD.with(|t| *t.borrow_mut() = hit.as_ref().map(|(w, i, _)| Grab::Wire(w.clone(), *i)));
    if let Some((w, _, p)) = hit {
        app.set_status(
            format!(
                "{w} at {}, {}, {}: drag an arrow (X, Y, Z) or the square (in the sketch plane); Delete removes it",
                crate::prefs::show_mm(app, p.x, 1),
                crate::prefs::show_mm(app, p.y, 1),
                crate::prefs::show_mm(app, p.z, 1)
            ),
            false,
        );
        return true;
    }
    false
}

/// The drawn curve or point with the triad, for Delete.
pub fn delete_selected(app: &mut SolveApp) -> bool {
    let Some(Grab::Wire(w, _)) = TRIAD.with(|t| t.borrow().clone()) else { return false };
    TRIAD.with(|t| *t.borrow_mut() = None);
    let r = app.run("sketch.delete", json!({ "entities": [w] }));
    report(app, r);
    true
}

/// A drag starting on a triad handle (true: the triad took it).
pub fn drag_start(app: &SolveApp, proj: &Proj, press: Pos2) -> bool {
    let Some((axis, _, _)) = handles(app, proj).into_iter().find(|(_, tip, _)| tip.distance(press) < 9.0) else { return false };
    let Some((_, p)) = triad_point(app) else { return false };
    let head_on = axis.and_then(|a| {
        let (axes, px_per_mm) = axes_on_screen(proj, p)?;
        axes.into_iter().find(|(b, _, head_on)| *b == a && *head_on).map(|(_, dir, _)| HeadOn { press, from: p, dir, px_per_mm })
    });
    let grab = target(app, proj, press, axis, p).map(|g| p - g).unwrap_or(Vec3::ZERO);
    DRAG.with(|d| *d.borrow_mut() = Some(Drag { axis, depth: app.session.undo.len(), grab, head_on }));
    true
}

/// A drag on an arrow whose axis points at the viewer: where it began, and how the screen maps
/// onto the axis (along `dir`, `px_per_mm`).
#[derive(Clone, Copy)]
struct HeadOn {
    press: Pos2,
    from: Vec3,
    dir: egui::Vec2,
    px_per_mm: f32,
}

/// The triad drag in progress: the axis (None: the plane square), the undo depth when it began
/// (the whole drag is one undo step), the pointer's offset from the point, and the head-on
/// mapping when the axis points at the viewer.
#[derive(Clone, Copy)]
struct Drag {
    axis: Option<Vec3>,
    depth: usize,
    grab: Vec3,
    head_on: Option<HeadOn>,
}

/// Where the pointer puts the point: along `axis` through `p`, or in the plane through `p`
/// parallel to the sketch plane.
fn target(app: &SolveApp, proj: &Proj, pos: Pos2, axis: Option<Vec3>, p: Vec3) -> Option<Vec3> {
    let (o, d) = proj.ray(pos);
    match axis {
        Some(u) => {
            let w0 = p - o;
            let (a, b, c) = (u.dot(u), u.dot(d), d.dot(d));
            let den = a * c - b * b;
            if den.abs() < 1e-12 {
                return None;
            }
            Some(p + u * ((b * d.dot(w0) - c * u.dot(w0)) / den))
        }
        None => {
            let st = app.session.world_state();
            let n = app.session.active_sketch.and_then(|s| st.sketch(s))?.plane.normal();
            let den = d.dot(n);
            (den.abs() > 1e-12).then(|| o + d * ((p - o).dot(n) / den))
        }
    }
}

pub fn dragging() -> bool {
    DRAG.with(|d| d.borrow().is_some())
}

pub fn drag_to(app: &mut SolveApp, proj: &Proj, pos: Pos2) {
    let Some(Drag { axis, depth, grab, head_on }) = DRAG.with(|d| *d.borrow()) else { return };
    let Some((held, p)) = triad_point(app) else { return };
    let to = match (head_on, axis) {
        // The axis points at the viewer, so no ray meets it: the drag's length along the arrow
        // is the distance moved.
        (Some(h), Some(a)) => Some(h.from + a * f64::from((pos - h.press).dot(h.dir) / h.px_per_mm)),
        _ => target(app, proj, pos, axis, p).map(|t| t + grab),
    };
    let Some(to) = to else { return };
    match held {
        Grab::Wire(w, i) => {
            if app.session.execute("sketch.move3d", &json!({"wire": w, "index": i, "to": [to.x, to.y, to.z]})).is_ok() {
                app.session.undo.truncate(depth + 1);
            }
        }
        Grab::Plane(id) => {
            let st = app.session.world_state();
            let Some(plane) = app.session.active_sketch.and_then(|s| st.sketch(s)).map(|ss| ss.plane) else { return };
            let local = plane.to_local(to);
            if plane.to_world(local).dist(to) < 1e-6 {
                // Still on the plane: an ordinary sketch point drag.
                if app.session.execute("sketch.move_point", &json!({"point": id, "to": [local.x, local.y]})).is_ok() {
                    app.session.undo.truncate(depth + 1);
                }
            } else if let Ok(r) = app.session.execute("sketch.move3d", &json!({"point": id, "to": [to.x, to.y, to.z]})) {
                // Off the plane: its curves are drawn 3D curves now, and the triad follows the point.
                app.session.undo.truncate(depth + 1);
                if let (Some(w), Some(k)) = (r.get("wire").and_then(Value::as_str), r.get("index").and_then(Value::as_u64)) {
                    TRIAD.with(|t| *t.borrow_mut() = Some(Grab::Wire(w.to_string(), k as usize)));
                }
            }
        }
    }
}

pub fn drag_end() {
    DRAG.with(|d| *d.borrow_mut() = None);
}

/// Draw the drawn 3D points and the triad.
pub fn show(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    if app.session.active_sketch.is_none() {
        return;
    }
    let tk = Tokens::get();
    let sel = TRIAD.with(|t| t.borrow().clone());
    for (w, i, p) in drawn_points(app) {
        if let Some(s) = proj.to_screen(p) {
            let on = sel.as_ref().is_some_and(|g| *g == Grab::Wire(w.clone(), i));
            painter.rect_filled(Rect::from_center_size(s, vec2(6.0, 6.0)), 0.0, if on { tk.accent } else { tk.sketch_point_free });
        }
    }
    if !active(app) {
        return;
    }
    for (name, tip) in handle_points(app, proj) {
        crate::scenario::publish_handle(&format!("triad_{name}"), tip);
    }
    for (axis, tip, base) in handles(app, proj) {
        match axis {
            Some(a) => {
                let col = if a == Vec3::X {
                    tk.axis_x
                } else if a == Vec3::Y {
                    tk.axis_y
                } else {
                    tk.axis_z
                };
                painter.line_segment([base, tip], Stroke::new(2.0, col));
                if let Some(dir) = unit(tip - base) {
                    let n = vec2(-dir.y, dir.x);
                    painter.add(Shape::convex_polygon(vec![tip + dir * 8.0, tip + n * 4.0, tip - n * 4.0], col, Stroke::NONE));
                }
            }
            None => {
                painter.rect_stroke(Rect::from_center_size(tip, vec2(10.0, 10.0)), 0.0, Stroke::new(1.5, tk.accent), egui::StrokeKind::Inside);
            }
        }
    }
}

/// Test hook: screen positions of the triad handles (axis name or "plane", tip).
pub fn handle_points(app: &SolveApp, proj: &Proj) -> Vec<(&'static str, Pos2)> {
    handles(app, proj)
        .into_iter()
        .map(|(a, tip, _)| {
            let name = match a {
                Some(v) if v == Vec3::X => "x",
                Some(v) if v == Vec3::Y => "y",
                Some(_) => "z",
                None => "plane",
            };
            (name, tip)
        })
        .collect()
}
