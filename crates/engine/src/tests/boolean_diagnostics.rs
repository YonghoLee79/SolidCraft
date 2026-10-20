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

/// A face of the tool a sketch is on, before and after the boolean that consumes it (#1865).
/// Two cylinders on one axis whose round sides lie in each other have no exact route for their
/// intersection; the faceted route answers it, and the result is flat where those sides curve.
/// The cap the sketch sits on is flat either way, and it comes back as that cap and not as the
/// triangles it was meshed into: the name the sketch stored still names a face of the result,
/// and names it exactly, so the attachment follows the body instead of going missing. The retry
/// reports that it kept the references.
#[test]
fn a_face_of_the_tool_a_sketch_is_on_survives_a_faceted_boolean() {
    let (r, meet) = (25.6, 22.0);
    let mut session = Session::default();
    run(&mut session, "solid.cylinder", json!({"base":[0,0,0],"radius":r,"height":30,"body_name":"Lower"}));
    run(&mut session, "solid.cylinder", json!({"base":[0,0,meet],"radius":r,"height":24,"operation":"new","body_name":"Upper"}));
    // The tool's near cap: inside the other body, so it is where their shared part ends.
    let upper = session.model.state().body("Upper").unwrap().clone();
    let cap = (0..upper.body.face_count())
        .find(|i| upper.body.planar_face(*i).is_some_and(|p| (p.origin.z - meet).abs() < 1e-9))
        .expect("the tool's near cap");
    let name = solidcraft_doc::naming::face_names(&upper).get(cap).cloned().expect("its name");
    assert!(solidcraft_doc::naming::face_name_exact(&upper, cap), "the cap of a cylinder is named exactly to begin with");
    // A sketch on it: what it stores to find the face again is that name.
    let sketch =
        run(&mut session, "sketch.create", json!({"plane":{"face":[0,0,meet]},"name":"Badge","project_edges":false}))["sketch"].as_u64().unwrap();
    run(&mut session, "sketch.circle.center", json!({"center":[0,0],"radius":4}));
    run(&mut session, "sketch.finish", json!({}));
    let id = run(&mut session, "solid.combine", json!({"target":"Lower","tools":["Upper"],"operation":"intersect"}))["feature"].as_u64().unwrap();
    let result = session.model.result(id).unwrap();
    assert!(result.error.is_none(), "{result:?}");
    // The route that answered it, and that it carried the operands' faces through.
    assert!(
        result.boolean_diagnostics.events.iter().any(|e| e.strategy == BooleanStrategy::FacetedFallback && e.references_kept),
        "{:?}",
        result.boolean_diagnostics
    );
    // The sketch's own feature still stands, and its face is still there under its own name.
    assert!(session.model.result(sketch).is_some_and(|r| r.error.is_none()), "{:?}", session.model.result(sketch));
    let bodies = session.model.state().bodies.clone();
    assert_eq!(bodies.len(), 1, "the intersection leaves one body: {:?}", bodies.iter().map(|b| &b.name).collect::<Vec<_>>());
    let body = bodies.first().cloned().expect("that body");
    let names = solidcraft_doc::naming::face_names(&body);
    let at = names.iter().position(|n| *n == name).unwrap_or_else(|| {
        panic!("the face the sketch is on is gone after the boolean: {name} not among {names:?}");
    });
    assert!(solidcraft_doc::naming::face_name_exact(&body, at), "the face the sketch is on is no longer named exactly: {name}");
    // It is the same flat cap, at the height the two bodies meet.
    assert!(body.body.planar_face(at).is_some_and(|p| (p.origin.z - meet).abs() < 1e-6), "{name} is not the cap any more");
}

/// A shell whose cavity cut the boolean cannot take keeps the shell's own exact answer, and does
/// not take a shell of facets instead (#1865, robustness seed 48).
///
/// `polyhedron::shell_opened` cuts a cavity out of the body, and where that cut fails it builds
/// the same shell another way and exactly (`shell_by_opening`). The faceted route is the last
/// resort of a boolean, not of its caller: answered on facets, the cut stops failing, the shell
/// never reaches its own route, and an exact shell of 28 faces becomes one of 116 — 98 mm³
/// heavier, because the cavity it subtracts is now inscribed in the walls instead of lying on
/// them. So the shell asks for that cut without the last resort, and asks again with it only
/// when its own route has failed too.
///
/// The body is a block 32.5 square and 22.5 tall whose fourth side is an arc bulging 12.5 out,
/// with a ø4 hole 15 deep in one face and a 5 × 5 × 20 post joined on top, shelled 1.5 thick
/// through its top face.
#[test]
fn a_shell_whose_cavity_cut_fails_keeps_its_own_exact_answer() {
    let mut session = Session::default();
    run(&mut session, "sketch.create", json!({"plane":"XY"}));
    run(&mut session, "sketch.line", json!({"points":[[0,0],[32.5,0],[32.5,32.5],[0,32.5]]}));
    run(&mut session, "sketch.arc.three_point", json!({"start":[0,32.5],"through":[-12.5,16.25],"end":[0,0]}));
    run(&mut session, "sketch.finish", json!({}));
    run(&mut session, "solid.extrude", json!({"distance":22.5}));
    run(&mut session, "solid.hole", json!({"diameter":4,"depth":15,"position":[16.25,0,11.25]}));
    run(&mut session, "solid.box", json!({"corner":[-7.5,22.5,22.5],"length":5,"width":5,"height":20,"operation":"join"}));
    let before = volume(&mut session);
    let id = run(&mut session, "solid.shell", json!({"faces":[[16.25,16.25,22.5]],"thickness":1.5}))["feature"].as_u64().unwrap();
    let result = session.model.result(id).unwrap();
    assert!(result.error.is_none(), "{result:?}");
    // The route that must not have answered it, and the body that says the same thing: an
    // exact shell carries the walls it was cut from, tens of faces, not hundreds of facets.
    assert!(
        !result.boolean_diagnostics.events.iter().any(|e| e.strategy == BooleanStrategy::FacetedFallback),
        "the shell took a cavity cut answered on facets instead of its own exact route: {:?}",
        result.boolean_diagnostics
    );
    let body = session.model.state().bodies.first().cloned().expect("the shelled body");
    let m = solidcraft_kernel::measure(&body.body).unwrap();
    assert!(m.faces < 40, "the shell came back on facets: {} faces", m.faces);
    // Hollow, and by about what a 1.5 wall leaves: far less than the solid it was cut from.
    let after = volume(&mut session);
    assert!(after < before * 0.3, "the shell did not hollow the body: {after} of {before}");
}
