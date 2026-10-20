//! Commands that recompute the history off the window thread (`deferred`): the marker moved,
//! undo, each computed beside the window and run once in; a marker moved again while one move
//! computes replaces the move still waiting.

use std::time::Duration;

use serde_json::json;

use crate::scenario::Harness;

/// A 60 × 20 × 10 block, a 10 × 10 × 5 boss on it, and a 4 mm hole through the boss, the
/// marker at the end. Every command computes off the window thread (no wait at all).
fn block_boss_hole() -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    for (id, p) in [
        ("solid.box", json!({"length": 60, "width": 20, "height": 10})),
        ("solid.box", json!({"corner": [5, 5, 10], "length": 10, "width": 10, "height": 5, "operation": "join"})),
        ("solid.cylinder", json!({"base": [10, 10, -1], "axis": [0, 0, 1], "radius": 2, "height": 20, "operation": "cut"})),
    ] {
        h.app.run(id, p).unwrap();
    }
    h.app.apply_off_thread = Some(Duration::ZERO);
    h
}

/// Frames until nothing computes or waits (a few seconds at most).
fn until_settled(h: &mut Harness) {
    for _ in 0..500 {
        if h.app.deferred.is_none() && h.app.deferred_queue.is_empty() && h.app.delete_check.is_none() {
            break;
        }
        h.frames(1);
        std::thread::sleep(Duration::from_millis(10));
    }
    h.frames(1);
}

fn marker(h: &Harness) -> Option<usize> {
    h.app.session.doc.marker
}

#[test]
fn the_marker_moved_computes_off_the_window_thread_then_moves() {
    let mut h = block_boss_hole();
    h.app.run_heavy("timeline.roll_to", json!({"position": 1})).unwrap();
    assert!(h.app.computing_since().is_some(), "computing");
    assert_eq!(marker(&h), None, "the design is as it was while it computes");
    until_settled(&mut h);
    assert_eq!(marker(&h), Some(1));
    h.step(&json!({"expect": {"volume": 12000}})).unwrap();
    // And back to the end, the boss and the hole recomputed (or found in the cache).
    h.app.run_heavy("timeline.roll_to", json!({})).unwrap();
    until_settled(&mut h);
    assert_eq!(marker(&h), None);
    let hole = std::f64::consts::PI * 4.0 * 15.0;
    let v = h.app.run("inspect.measure", json!({})).unwrap()["total"]["volume_mm3"].as_f64().unwrap();
    assert!((v - (12500.0 - hole)).abs() < 1.0, "{v}");
}

#[test]
fn playback_waits_for_each_background_rebuild_before_timing_the_next_step() {
    let mut h = block_boss_hole();
    h.app.run("timeline.roll_to", json!({"position": 0})).unwrap();
    crate::timeline::transport_action(&mut h.app, &h.ctx, "last", None);
    h.frames(1);
    for position in 1..=3 {
        assert!(h.app.deferred.is_some(), "step {position} is computed in the background");
        assert!(h.app.deferred_queue.is_empty(), "playback must not queue steps that supersede each other");
        until_settled(&mut h);
        assert_eq!(marker(&h).unwrap_or(3), position);
        h.frames(10);
        assert_eq!(marker(&h).unwrap_or(3), position, "step {position} stays visible after computation");
        if position < 3 {
            // Advance only until the next step starts; never sleep through a whole playback.
            for _ in 0..40 {
                if h.app.deferred.is_some() {
                    break;
                }
                h.frames(1);
            }
        }
    }
    assert!(h.app.timeline_playback.is_none());
    assert!(h.app.deferred_queue.is_empty());
}

#[test]
fn interrupted_playback_never_applies_its_pending_background_step() {
    for action in ["stop", "edit", "marker", "tool", "palette", "new"] {
        let mut h = block_boss_hole();
        h.app.run("timeline.roll_to", json!({"position": 0})).unwrap();
        crate::timeline::transport_action(&mut h.app, &h.ctx, "last", None);
        h.frames(1);
        assert!(h.app.deferred.is_some());
        match action {
            "stop" => crate::timeline::transport_action(&mut h.app, &h.ctx, "last", None),
            "edit" => {
                h.app.run("timeline.rename", json!({"feature": 1, "name": "Renamed"})).unwrap();
            }
            "marker" => {
                h.app.run("timeline.roll_to", json!({"position": 2})).unwrap();
            }
            "tool" => h.app.start("solid.box"),
            "palette" => h.app.start("ui.palette"),
            _ => {
                h.app.run("file.new", json!({})).unwrap();
            }
        }
        assert!(h.app.timeline_playback.is_none(), "{action}");
        assert!(h.app.deferred.is_none(), "{action}: the worker cannot apply the old marker");
        let marker = marker(&h);
        h.frames(90);
        assert_eq!(h.app.session.doc.marker, marker, "{action}");
    }
}

#[test]
fn a_marker_moved_again_replaces_the_move_still_waiting() {
    let mut h = block_boss_hole();
    h.app.run_heavy("timeline.roll_to", json!({"position": 1})).unwrap();
    assert!(h.app.computing_since().is_some());
    // A drag: two more slots while the first computes. Only the last one waits.
    h.app.run_heavy("timeline.roll_to", json!({"position": 2})).unwrap();
    h.app.run_heavy("timeline.roll_to", json!({"position": 3})).unwrap();
    assert_eq!(h.app.deferred_queue.len(), 1, "{:?}", h.app.deferred_queue);
    until_settled(&mut h);
    assert_eq!(marker(&h), None, "where the drag ended: the end of the history");
}

#[test]
fn undo_computes_off_the_window_thread() {
    let mut h = block_boss_hole();
    let features = h.app.session.doc.features.len();
    h.app.start("edit.undo");
    assert!(h.app.computing_since().is_some(), "undo computing");
    assert_eq!(h.app.session.doc.features.len(), features, "nothing undone yet");
    until_settled(&mut h);
    assert_eq!(h.app.session.doc.features.len(), features - 1, "the hole undone");
    h.step(&json!({"expect": {"volume": 12500}})).unwrap();
    h.app.start("edit.redo");
    until_settled(&mut h);
    assert_eq!(h.app.session.doc.features.len(), features, "and redone");
}

#[test]
fn a_design_changed_while_a_move_computes_is_computed_again() {
    let mut h = block_boss_hole();
    h.app.run_heavy("timeline.roll_to", json!({"position": 2})).unwrap();
    assert!(h.app.computing_since().is_some());
    // Something else changes the design meanwhile (a command that just runs).
    h.app.run("solid.box", json!({"corner": [40, 5, 10], "length": 5, "width": 5, "height": 5, "operation": "join"})).unwrap();
    let revision = h.app.session.revision;
    until_settled(&mut h);
    assert!(h.app.session.revision > revision, "the move ran after the change");
    assert_eq!(marker(&h), Some(2));
}

fn feature_ids(h: &Harness) -> Vec<u64> {
    h.app.session.doc.features.iter().map(|f| f.id).collect()
}

/// Volume of the block with the hole through it, once the boss is deleted.
fn block_with_hole() -> f64 {
    12000.0 - std::f64::consts::PI * 4.0 * 10.0
}

fn volume(h: &mut Harness) -> f64 {
    h.app.run("inspect.measure", json!({})).unwrap()["total"]["volume_mm3"].as_f64().unwrap()
}

/// The timeline's Delete checks what the delete breaks off the window thread (#2678): the design
/// is as it was while it checks, and the feature goes once the check is in.
#[test]
fn the_timelines_delete_checks_off_the_window_thread() {
    let mut h = block_boss_hole();
    let boss = feature_ids(&h)[1];
    crate::timeline::delete_features(&mut h.app, vec![boss]);
    assert!(h.app.computing_since().is_some(), "checking");
    assert!(h.app.session.doc.feature(boss).is_some(), "the design is as it was while it checks");
    until_settled(&mut h);
    assert!(h.app.session.doc.feature(boss).is_none(), "deleted once the check is in");
    assert!(h.app.dialog.is_none(), "nothing breaks, so nothing to confirm");
    assert!((volume(&mut h) - block_with_hole()).abs() < 1.0);
}

/// Delete on the selection (the Delete key) runs its dry run off the window thread too.
#[test]
fn delete_on_the_selection_checks_off_the_window_thread() {
    let mut h = block_boss_hole();
    let boss = feature_ids(&h)[1];
    h.app.session.selection = vec![solidcraft_engine::Sel::Feature { id: boss }];
    let params = crate::delete::selection_params(&h.app);
    crate::delete::request(&mut h.app, params);
    assert!(h.app.computing_since().is_some(), "checking");
    assert!(h.app.session.doc.feature(boss).is_some());
    until_settled(&mut h);
    assert!(h.app.session.doc.feature(boss).is_none());
    assert!(h.app.menu.confirm.is_none());
    assert!((volume(&mut h) - block_with_hole()).abs() < 1.0);
}

/// A design changed while its delete is checked is checked again before anything is deleted, and
/// a second Delete meanwhile is left out.
#[test]
fn a_design_changed_while_a_delete_is_checked_is_checked_again() {
    let mut h = block_boss_hole();
    let ids = feature_ids(&h);
    crate::timeline::delete_features(&mut h.app, vec![ids[1]]);
    crate::timeline::delete_features(&mut h.app, vec![ids[2]]);
    h.app.run("solid.box", json!({"corner": [100, 0, 0], "length": 10, "width": 10, "height": 10})).unwrap();
    until_settled(&mut h);
    assert!(h.app.session.doc.feature(ids[1]).is_none(), "the boss is deleted");
    assert!(h.app.session.doc.feature(ids[2]).is_some(), "the second Delete was left out");
    assert!((volume(&mut h) - (block_with_hole() + 1000.0)).abs() < 1.0, "the box added meanwhile stays");
}

/// Without a wait (headless use, tests) the check runs in place, as before.
#[test]
fn without_a_wait_the_delete_checks_in_place() {
    let mut h = block_boss_hole();
    h.app.apply_off_thread = None;
    let boss = feature_ids(&h)[1];
    crate::timeline::delete_features(&mut h.app, vec![boss]);
    assert!(h.app.delete_check.is_none());
    assert!(h.app.session.doc.feature(boss).is_none());
}
