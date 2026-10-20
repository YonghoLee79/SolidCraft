use super::*;
use solidcraft_kernel::BooleanStrategy;

#[test]
fn boolean_retry_warning_and_details_survive_cache_undo_and_reopen() {
    let slope = 1f64.to_radians().tan();
    let mut session = Session::default();
    run(&mut session, "sketch.create", json!({"plane":"XZ","name":"Floor section"}));
    run(&mut session, "sketch.line", json!({"points":[[0,0],[100,0],[100,-10.-slope*100.],[0,-10.]],"closed":true}));
    run(&mut session, "sketch.finish", json!({}));
    run(&mut session, "solid.extrude", json!({"sketch":"Floor section","distance":80,"body_name":"Floor"}));
    run(&mut session, "solid.cylinder", json!({"base":[50,40,-5],"radius":10,"height":30,"operation":"cut"}));
    run(&mut session, "sketch.create", json!({"plane":"XY","name":"Ring section"}));
    run(&mut session, "sketch.circle.center", json!({"center":[50,40],"radius":20}));
    run(&mut session, "sketch.circle.center", json!({"center":[50,40],"radius":10}));
    run(&mut session, "sketch.finish", json!({}));
    run(
        &mut session,
        "solid.extrude",
        json!({"sketch":"Ring section","profiles":[{"point":[65,40]}],"start_offset":-5,"distance":7,"body_name":"Ring"}),
    );
    let id = run(&mut session, "solid.combine", json!({"target":"Floor","tools":["Ring"],"operation":"join"}))["feature"].as_u64().unwrap();
    let check = |session: &Session| {
        let result = session.model.result(id).unwrap();
        assert!(result.error.is_none());
        assert!(result.warning.as_ref().is_some_and(|warning| warning.contains("Boolean retry:")), "{result:?}");
        assert!(
            result.boolean_diagnostics.events.iter().any(|event| event.strategy == BooleanStrategy::CommonBore),
            "{:?}",
            result.boolean_diagnostics
        );
        result.boolean_diagnostics.clone()
    };
    let original = check(&session);
    let expected = (10. + slope * 50.) * (8000. - PI * 100.) + PI * 300. * 5.;
    assert!(rel(volume(&mut session), expected) < 2e-5);
    let inspected = run(&mut session, "timeline.inspect", json!({"feature":id}));
    assert_eq!(inspected["boolean_diagnostics"], serde_json::to_value(&original).unwrap());
    session.refresh();
    assert_eq!(check(&session), original);
    run(&mut session, "edit.undo", json!({}));
    run(&mut session, "edit.redo", json!({}));
    assert_eq!(check(&session), original);
    let reopened_doc = solidcraft_doc::Document::from_json(&session.doc.to_json()).unwrap();
    let reopened = Session::new(reopened_doc);
    assert_eq!(check(&reopened), original);
    // A later independent feature must not inherit the earlier retry's warning.
    run(&mut session, "solid.box", json!({"corner":[200,0,0],"length":5,"width":5,"height":5}));
    let last = session.model.results.last().unwrap();
    assert!(last.warning.is_none());
    assert!(last.boolean_diagnostics.events.is_empty());
}

/// A barrel joined under a plate's back edge, its ends flush with the sides of the leaf it hangs
/// from (#827 without its fillets): the join is exact, so the command's reply carries no Boolean
/// retry, and the barrel's ends stay in the leaf's sides (it is not nudged to make the join go).
#[test]
fn a_barrel_joined_between_the_sides_of_a_leaf_needs_no_retry() {
    let mut session = Session::default();
    run(&mut session, "solid.box", json!({"length": 40, "width": 20, "height": 6, "corner": [0, 0, 5], "body_name": "Plate"}));
    run(&mut session, "solid.box", json!({"length": 14, "width": 10, "height": 6, "corner": [10, 10, 0], "operation": "join", "targets": ["Plate"]}));
    let reply = run(
        &mut session,
        "solid.cylinder",
        json!({"radius": 5, "height": 14, "axis": "X", "base": [10, 20, 0], "operation": "join", "targets": ["Plate"]}),
    );
    assert!(reply["warning"].is_null(), "{reply}");
    let result = session.model.result(reply["feature"].as_u64().unwrap()).unwrap();
    assert!(result.boolean_diagnostics.events.is_empty(), "{:?}", result.boolean_diagnostics);
    let state = session.model.state();
    let plate = &state.body("Plate").unwrap().body;
    assert_eq!(plate.shell_count(), 1);
    assert!(plate.validity().is_empty(), "{:?}", plate.validity());
    // Every edge ends on a level along x of the plate, the leaf or the barrel.
    for edge in plate.edges(1e-3).unwrap() {
        for p in [edge.points.first(), edge.points.last()].into_iter().flatten() {
            assert!([0., 10., 24., 40.].iter().any(|x| (p.x - x).abs() < 1e-9), "an edge ends at {p:?}");
        }
    }
    // A quarter of the barrel lies in the leaf.
    let expected = 5500. + 0.75 * PI * 25. * 14.;
    let actual = plate.tessellate(1e-4).unwrap().measure().volume;
    assert!(rel(actual, expected) < 1e-5, "{actual} vs {expected}");
}

/// A profile revolved ±90° about an axis lying in a plate's top face, joined to the plate or cut from it (#3156): both end
/// faces of the revolve lie flush in that face. The join and the cut are exact, with no Boolean retry warning (the tool is
/// not nudged). Volumes are analytic: the plate, plus or minus the part of the revolve inside it.
#[test]
fn a_revolve_ended_flush_with_the_face_its_axis_lies_in_needs_no_retry() {
    // Simpson's rule over [30, 37], the profile's distances from the axis.
    let integral = |f: &dyn Fn(f64) -> f64| {
        let (n, h) = (2000, 7. / 2000.);
        let inner: f64 = (1..n).map(|i| f(30. + h * f64::from(i)) * if i % 2 == 1 { 4. } else { 2. }).sum();
        (f(30.) + f(37.) + inner) * h / 3.
    };
    // The rectangle is 4 wide along x; the trapezoid is 4 wide 30 from the axis and 2 wide 37 from it.
    let profiles: [(&str, [[f64; 2]; 4], &dyn Fn(f64) -> f64); 2] = [
        ("rectangle", [[-47., -10.], [-43., -10.], [-43., -17.], [-47., -17.]], &|_| 4.),
        ("trapezoid", [[-47., -17.], [-43., -17.], [-44., -10.], [-46., -10.]], &|r| 4. - 2. * (r - 30.) / 7.),
    ];
    for (name, outline, width) in profiles {
        let turn = PI * integral(&|r| width(r) * r);
        // On each side of the axis, the ring r from it lies in the plate (6 thick) for asin(6 / r) below its top face.
        let inside = 2. * integral(&|r| width(r) * r * (6. / r).asin());
        for (operation, expected) in [("join", 6000. + turn - inside), ("cut", 6000. - inside)] {
            let mut session = Session::default();
            run(&mut session, "solid.box", json!({"length":10,"width":100,"height":6,"corner":[-50,-50,41],"body_name":"Plate"}));
            run(&mut session, "sketch.create", json!({"plane":"XZ"}));
            run(&mut session, "sketch.line", json!({"points":outline,"closed":true}));
            run(&mut session, "sketch.line", json!({"points":[[-60,-47],[-30,-47]],"construction":true,"ids":["ax"]}));
            run(&mut session, "sketch.finish", json!({}));
            let reply = run(
                &mut session,
                "solid.revolve",
                json!({"axis":"ax","angle":"90 deg","angle2":"90 deg","operation":operation,"profiles":[{"point":[-45,-13.5]}]}),
            );
            assert!(reply["warning"].is_null(), "{name} {operation}: {reply}");
            let result = session.model.result(reply["feature"].as_u64().unwrap()).unwrap();
            assert!(
                !result.boolean_diagnostics.events.iter().any(|event| event.strategy.alters_result()),
                "{name} {operation}: {:?}",
                result.boolean_diagnostics
            );
            let state = session.model.state();
            let plate = &state.body("Plate").unwrap().body;
            assert!(plate.validity().is_empty(), "{name} {operation}: {:?}", plate.validity());
            let actual = plate.tessellate(2e-3).unwrap().measure().volume;
            assert!(rel(actual, expected) < 2e-5, "{name} {operation}: {actual} vs {expected}");
        }
    }
}
