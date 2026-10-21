//! Snapping while a sketch point or a line is dragged: the dragged point snaps to other points
//! (ends, centres, the origin, projected points), to the middle of a line and onto curves, and
//! lines up horizontally or vertically with other points (a dashed guide). Letting go on a point,
//! a midpoint or a curve adds the matching constraint; lining up adds none. The suppress-snap
//! chord is configurable (default Ctrl/Cmd or Alt/Option; see `prefs::suppress_sketch_snap`).

use std::cell::{Cell, RefCell};

use egui::{Align2, FontId, Shape, Stroke};
use serde_json::{Value, json};
use solidcraft_engine::geom::Vec2;
use solidcraft_engine::sketch::{CurveKind, Sketch};

use crate::SolveApp;
use crate::theme::Tokens;
use crate::viewport::Proj;

/// What the dragged point snapped to.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Another point (its id): coincident on release.
    Point(String),
    /// The middle of a line (its id): midpoint on release.
    Mid(String),
    /// On a curve (its id): point on curve on release.
    On(String),
    /// Where two curves cross (their ids): point on both curves on release.
    Cross(String, String),
    /// Lined up with other points: no constraint.
    Align,
}

/// A snap: the moving point (its id) and where it goes, the move added to the cursor's, the
/// target, and the dashed guides (sketch coordinates).
#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub point: String,
    pub at: Vec2,
    pub offset: Vec2,
    pub target: Target,
    pub guides: Vec<(Vec2, Vec2)>,
}

impl Snap {
    pub fn label(&self) -> &'static str {
        match self.target {
            Target::Point(_) => "Pt",
            Target::Mid(_) => "Mid",
            Target::On(_) => "On",
            Target::Cross(..) => "X",
            Target::Align => "",
        }
    }
}

/// Snap distances, in screen pixels.
const POINT_PX: f64 = 8.0;
const CURVE_PX: f64 = 6.0;
const ALIGN_PX: f64 = 6.0;

thread_local! {
    static CURRENT: RefCell<Option<Snap>> = const { RefCell::new(None) };
    static SUPPRESSED: Cell<bool> = const { Cell::new(false) };
}

/// Whether sketch snap is suppressed this frame (set from the viewport via `prefs::suppress_sketch_snap_held`).
pub fn set_suppressed(on: bool) {
    SUPPRESSED.with(|s| s.set(on));
}

pub fn suppressed() -> bool {
    SUPPRESSED.with(Cell::get)
}

pub fn current() -> Option<Snap> {
    CURRENT.with(|c| c.borrow().clone())
}

pub fn clear() {
    CURRENT.with(|c| *c.borrow_mut() = None);
}

fn keep(s: Option<Snap>) -> Option<Snap> {
    CURRENT.with(|c| *c.borrow_mut() = s.clone());
    s
}

/// The best snap for points moving to `moving` (point index, position after the cursor's move),
/// `mm_px` millimetres per pixel. Points in `exclude` (the ones moving with the drag) are not
/// targets, nor are curves using them.
pub fn find(sk: &Sketch, moving: &[(usize, Vec2)], exclude: &[usize], mm_px: f64) -> Option<Snap> {
    if suppressed() || moving.is_empty() || !(mm_px > 0.0 && mm_px.is_finite()) {
        return keep(None);
    }
    let id = |i: usize| sk.points.get(i).map(|p| p.id.clone()).unwrap_or_default();
    let visible = solidcraft_engine::view::SketchPointVisibility::new(sk);
    let targets: Vec<(usize, Vec2)> =
        sk.points.iter().enumerate().filter(|(i, p)| !exclude.contains(i) && visible.takes(p)).map(|(i, p)| (i, p.pos)).collect();
    let free_curves: Vec<usize> =
        (0..sk.curves.len()).filter(|&c| sk.curves.get(c).is_some_and(|cu| !exclude.iter().any(|&p| cu.kind.uses(p)))).collect();
    let mut best: Option<(f64, Snap)> = None;
    let offer = |best: &mut Option<(f64, Snap)>, d: f64, s: Snap| {
        if best.as_ref().is_none_or(|b| d < b.0) {
            *best = Some((d, s));
        }
    };
    // Points, then midpoints, then curves.
    for &(mi, p) in moving {
        for &(ti, t) in &targets {
            let d = p.dist(t);
            if d < POINT_PX * mm_px {
                offer(&mut best, d, Snap { point: id(mi), at: t, offset: t - p, target: Target::Point(id(ti)), guides: Vec::new() });
            }
        }
    }
    // A crossing of two curves that stay put beats a midpoint or a curve (when the sketch
    // snaps to crossings).
    if best.is_none() && sk.view.snaps_crossings() {
        for &(mi, p) in moving {
            let Some(x) = solidcraft_engine::crossing_near(sk, p, CURVE_PX * mm_px, |c| free_curves.contains(&c)) else { continue };
            let (Some(a), Some(b)) = (sk.curves.get(x.a), sk.curves.get(x.b)) else { continue };
            let t = Target::Cross(a.id.clone(), b.id.clone());
            offer(&mut best, p.dist(x.at), Snap { point: id(mi), at: x.at, offset: x.at - p, target: t, guides: Vec::new() });
        }
    }
    if best.is_none() {
        for &(mi, p) in moving {
            for &c in &free_curves {
                let Some(cu) = sk.curves.get(c) else { continue };
                let CurveKind::Line { a, b } = cu.kind else { continue };
                let (Some(a), Some(b)) = (sk.point(a), sk.point(b)) else { continue };
                let m = (a + b) * 0.5;
                let d = p.dist(m);
                if d < POINT_PX * mm_px {
                    offer(&mut best, d, Snap { point: id(mi), at: m, offset: m - p, target: Target::Mid(cu.id.clone()), guides: Vec::new() });
                }
            }
        }
    }
    if best.is_none() {
        for &(mi, p) in moving {
            for &c in &free_curves {
                let (Some(cu), Some(sh)) = (sk.curves.get(c), sk.shape(c)) else { continue };
                let d = sh.dist(p);
                if d < CURVE_PX * mm_px {
                    let q = sh.project(p);
                    offer(&mut best, d, Snap { point: id(mi), at: q, offset: q - p, target: Target::On(cu.id.clone()), guides: Vec::new() });
                }
            }
        }
    }
    if let Some((_, s)) = best {
        return keep(Some(s));
    }
    // Lining up: the nearest horizontal and the nearest vertical alignment, each on its own axis.
    let tol = ALIGN_PX * mm_px;
    let (mut hx, mut vy): (Option<(f64, usize, Vec2, Vec2)>, Option<(f64, usize, Vec2, Vec2)>) = (None, None);
    for &(mi, p) in moving {
        for &(_, t) in &targets {
            let (dy, dx) = ((p.y - t.y).abs(), (p.x - t.x).abs());
            if dy < tol && dx > tol && hx.is_none_or(|h| dy < h.0) {
                hx = Some((dy, mi, p, t));
            }
            if dx < tol && dy > tol && vy.is_none_or(|v| dx < v.0) {
                vy = Some((dx, mi, p, t));
            }
        }
    }
    let mut offset = Vec2::ZERO;
    let mut guides = Vec::new();
    let mut point = None;
    if let Some((_, mi, p, t)) = hx {
        offset.y = t.y - p.y;
        point = Some(mi);
        guides.push((t, Vec2::new(p.x, t.y)));
    }
    if let Some((_, mi, p, t)) = vy {
        offset.x = t.x - p.x;
        point = point.or(Some(mi));
        guides.push((t, Vec2::new(t.x, p.y)));
    }
    // The guides end at the snapped point (both alignments may move it).
    let Some(mi) = point else { return keep(None) };
    let at = moving.iter().find(|m| m.0 == mi).map_or(Vec2::ZERO, |m| m.1) + offset;
    for g in &mut guides {
        g.1 = if (g.0.y - g.1.y).abs() < 1e-12 { Vec2::new(at.x, g.0.y) } else { Vec2::new(g.0.x, at.y) };
    }
    keep(Some(Snap { point: id(mi), at, offset, target: Target::Align, guides }))
}

/// Points the drag of `entity` (a point or curve id) moves, and the points that move with them.
pub fn moving_points(sk: &Sketch, entity: &str) -> (Vec<usize>, Vec<usize>) {
    let own: Vec<usize> = if let Some(p) = sk.point_index(entity) {
        vec![p]
    } else {
        match sk.curve_index(entity).and_then(|c| sk.curves.get(c)) {
            // Only lines slide whole; a circle or arc grabbed on its edge changes its radius.
            Some(c) if matches!(c.kind, CurveKind::Line { .. }) => c.kind.point_ids(),
            _ => Vec::new(),
        }
    };
    let mut ex = own.clone();
    for c in &sk.curves {
        if own.iter().any(|&p| c.kind.uses(p)) {
            ex.extend(c.kind.point_ids());
        }
    }
    ex.sort_unstable();
    ex.dedup();
    (own, ex)
}

/// The drag let go: the constraint(s) of the snap it ended on (none for lining up), as
/// `sketch.drag_end` takes them (`constraint`, or `constraints` for a crossing).
pub fn release(sk: &Sketch) -> Value {
    let Some(s) = current() else { return json!({}) };
    clear();
    if let Target::Cross(a, b) = &s.target {
        // Both curves or none; a curve the point is an end of already holds it.
        let end_of = |c: &String| {
            let (p, c) = (sk.point_index(&s.point), sk.curve_index(c).and_then(|c| sk.curves.get(c)));
            p.zip(c).is_some_and(|(p, c)| c.kind.uses(p))
        };
        let cs: Vec<Value> = [a, b]
            .into_iter()
            .filter(|c| !end_of(c))
            .map(|c| json!({"command": "sketch.constraint.coincident", "params": {"a": s.point, "b": c}}))
            .collect();
        return json!({"constraints": cs});
    }
    let (cmd, params) = match &s.target {
        Target::Point(t) => ("sketch.constraint.coincident", json!({"a": s.point, "b": t})),
        Target::Mid(l) => ("sketch.constraint.midpoint", json!({"point": s.point, "line": l})),
        Target::On(c) => ("sketch.constraint.coincident", json!({"a": s.point, "b": c})),
        Target::Cross(..) | Target::Align => return json!({}),
    };
    json!({"constraint": {"command": cmd, "params": params}})
}

/// The snap of the drag in progress: its dashed guides, a ring on the target and its label.
pub fn show(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    if app.viewport.point_drag.is_none() {
        return;
    }
    let Some(s) = current() else { return };
    let st = app.session.world_state();
    let Some(ss) = app.session.active_sketch.and_then(|i| st.sketch(i)) else { return };
    let to = |p: Vec2| proj.to_screen(ss.plane.to_world(p));
    let tk = Tokens::get();
    for (a, b) in &s.guides {
        if let (Some(a), Some(b)) = (to(*a), to(*b)) {
            painter.add(Shape::dashed_line(&[a, b], Stroke::new(1.0, tk.rubber_band), 5.0, 4.0));
        }
    }
    let Some(sp) = to(s.at) else { return };
    painter.circle_stroke(sp, 6.0, Stroke::new(1.5, tk.accent));
    let label = s.label();
    if !label.is_empty() {
        let r = egui::Rect::from_min_size(sp + egui::vec2(12.0, -24.0), egui::vec2(24.0, 14.0));
        painter.rect_filled(r, 3.0, tk.accent_soft);
        painter.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(crate::theme::Tokens::get().font_micro), tk.text);
    }
    crate::scenario::publish_count("drag_guides", s.guides.len() as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sk() -> Sketch {
        let mut s = Sketch::new();
        let _ = s.add_line(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), None, None, Some("l1"));
        let _ = s.add_line(Vec2::new(20.0, 5.0), Vec2::new(30.0, 15.0), None, None, Some("l2"));
        s
    }

    fn drag(s: &Sketch, entity: &str, p: Vec2) -> Option<Snap> {
        let (own, ex) = moving_points(s, entity);
        find(s, &[(own[0], p)], &ex, 0.1)
    }

    #[test]
    fn a_dragged_end_snaps_to_points_midpoints_and_curves() {
        let s = sk();
        let end = s.resolve_point("l2.start").map(|i| s.points[i].id.clone()).unwrap_or_default();
        let at = drag(&s, &end, Vec2::new(10.3, 0.4)).expect("point");
        assert_eq!(at.target, Target::Point(s.points[s.resolve_point("l1.end").unwrap_or(0)].id.clone()));
        assert_eq!(at.at, Vec2::new(10.0, 0.0));
        let mid = drag(&s, &end, Vec2::new(5.2, 0.3)).expect("mid");
        assert_eq!(mid.target, Target::Mid("l1".into()));
        let on = drag(&s, &end, Vec2::new(7.5, 0.3)).expect("on");
        assert_eq!(on.target, Target::On("l1".into()));
        assert!(on.at.y.abs() < 1e-12);
    }

    fn crossing_sk() -> Sketch {
        let mut s = Sketch::new();
        let _ = s.add_line(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0), None, None, Some("l1"));
        let _ = s.add_line(Vec2::new(0.0, 20.0), Vec2::new(20.0, 0.0), None, None, Some("l2"));
        let _ = s.add_line(Vec2::new(40.0, 5.0), Vec2::new(50.0, 15.0), None, None, Some("l3"));
        s.view.snap_crossings = true;
        s
    }

    #[test]
    fn a_crossing_is_no_target_until_the_option_is_on() {
        let mut s = crossing_sk();
        s.view.snap_crossings = false;
        let end = s.points[s.resolve_point("l3.start").unwrap_or(0)].id.clone();
        let x = drag(&s, &end, Vec2::new(10.3, 9.8)).expect("a curve");
        assert!(!matches!(x.target, Target::Cross(..)), "{:?}", x.target);
    }

    #[test]
    fn snap_off_turns_the_crossing_off_too() {
        let mut s = crossing_sk();
        s.view.no_snap = true;
        let end = s.points[s.resolve_point("l3.start").unwrap_or(0)].id.clone();
        let x = drag(&s, &end, Vec2::new(10.3, 9.8)).expect("a curve");
        assert!(!matches!(x.target, Target::Cross(..)), "{:?}", x.target);
    }

    #[test]
    fn a_dragged_end_snaps_to_a_crossing() {
        let s = crossing_sk();
        let end = s.points[s.resolve_point("l3.start").unwrap_or(0)].id.clone();
        let x = drag(&s, &end, Vec2::new(10.3, 9.8)).expect("crossing");
        assert_eq!(x.target, Target::Cross("l1".into(), "l2".into()));
        assert!(x.at.dist(Vec2::new(10.0, 10.0)) < 1e-6);
        assert_eq!(x.label(), "X");
        let both = release(&s);
        assert_eq!(both["constraints"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn a_dragged_line_resting_on_a_curve_still_finds_the_crossing() {
        // The dragged line is drawn first and its end rests on l1 near the l1×l2 crossing: its
        // own touch of l1 is nearer, but it moves with the drag, so it is no target.
        let mut s = Sketch::new();
        let _ = s.add_line(Vec2::new(40.0, 5.0), Vec2::new(10.3, 10.3), None, None, Some("l0"));
        let _ = s.add_line(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0), None, None, Some("l1"));
        let _ = s.add_line(Vec2::new(0.0, 20.0), Vec2::new(20.0, 0.0), None, None, Some("l2"));
        s.view.snap_crossings = true;
        let end = s.points[s.resolve_point("l0.end").unwrap_or(0)].id.clone();
        let x = drag(&s, &end, Vec2::new(10.35, 10.25)).expect("crossing");
        assert_eq!(x.target, Target::Cross("l1".into(), "l2".into()));
        assert!(x.at.dist(Vec2::new(10.0, 10.0)) < 1e-6);
    }

    #[test]
    fn a_point_beats_a_crossing() {
        let mut s = crossing_sk();
        let _ = s.add_point(Vec2::new(10.0, 10.0), None);
        let end = s.points[s.resolve_point("l3.start").unwrap_or(0)].id.clone();
        let x = drag(&s, &end, Vec2::new(10.3, 9.8)).expect("point");
        assert!(matches!(x.target, Target::Point(_)), "{:?}", x.target);
    }

    #[test]
    fn lining_up_moves_the_point_but_names_no_constraint() {
        let s = sk();
        let end = s.points[s.resolve_point("l2.end").unwrap_or(0)].id.clone();
        // Level with l1's end (y 0), far from it in x.
        let a = drag(&s, &end, Vec2::new(40.0, 0.35)).expect("aligned");
        assert_eq!(a.target, Target::Align);
        assert_eq!(a.at.y, 0.0);
        assert_eq!(a.guides.len(), 1);
        assert!(drag(&s, &end, Vec2::new(40.0, 3.0)).is_none());
        set_suppressed(true);
        assert!(drag(&s, &end, Vec2::new(10.1, 0.1)).is_none());
        set_suppressed(false);
    }
}
