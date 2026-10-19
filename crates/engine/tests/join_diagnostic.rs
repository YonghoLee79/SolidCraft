use serde_json::json;
use solidcraft_engine::Session;

#[test]
fn reported_two_profile_extrusion_and_separate_combine() {
    let text = std::fs::read_to_string(r"E:\FreeCad\TESTE.1.json").unwrap();
    for separate in [false, true] {
        let mut session = Session::new(solidcraft_doc::Document::from_json(&text).unwrap());
        let start = std::time::Instant::now();
        let result = session.execute(
            "solid.extrude",
            &json!({"sketch":7,"profiles":[2,3],"distance":"25","direction":"positive",
                "convention":"total_distance","operation":if separate {"new"} else {"join"}}),
        );
        eprintln!("reported profiles, separate={separate}, elapsed={:?}: {result:?}", start.elapsed());
        result.unwrap();
        if separate {
            let tools: Vec<String> = session.model.state().bodies.iter().filter(|body| body.name != "Body1").map(|body| body.name.clone()).collect();
            let result = session.execute("solid.combine", &json!({"target":"Body1","tools":tools,"operation":"join"}));
            eprintln!("combine: {result:?}");
            result.unwrap();
        }
        assert_eq!(session.model.state().bodies.len(), 1);
        for result in &session.model.results {
            assert!(result.error.is_none(), "feature {}: {:?}", result.id, result.error);
        }
        for body in &session.model.state().bodies {
            assert!(body.body.validity().is_empty(), "{:?}", body.body.validity());
        }
        if !separate {
            std::fs::write(r"E:\FreeCad\solidcraft\target\join-fixed.solidcraft", session.doc.to_json()).unwrap();
        }
    }
}

#[test]
fn join_diagnostic() {
    let text = std::fs::read_to_string(r"E:\FreeCad\TESTE.1.json").unwrap();
    let doc = solidcraft_doc::Document::from_json(&text).unwrap();
    let start = std::time::Instant::now();
    let mut session = Session::new(doc);
    eprintln!("loaded in {:?}", start.elapsed());
    for result in &session.model.results {
        assert!(result.error.is_none(), "feature {}: {:?}", result.id, result.error);
    }
    let state = session.model.state();
    let sketch = state.sketch(7).unwrap();
    eprintln!("plane: {:?}", sketch.plane);
    for (i, profile) in sketch.profiles.iter().enumerate() {
        eprintln!("profile {i}: area {} curves {:?} holes {:?}", profile.area, profile.outer_curves, profile.hole_curves);
    }
    if std::env::var_os("SOLIDCRAFT_JOIN_OPERANDS").is_some() {
        session.execute("solid.extrude", &json!({"sketch":7,"profiles":[{"curves":["c2"]}],"distance":10,"operation":"new"})).unwrap();
        let state = session.model.state();
        for (i, body) in state.bodies.iter().enumerate() {
            std::fs::write(
                format!(r"E:\FreeCad\solidcraft\target\join-operand-{i}.step"),
                solidcraft_kernel::step_export(&[&body.body], "join diagnostic").unwrap(),
            )
            .unwrap();
        }
        return;
    }
    let start = std::time::Instant::now();
    let (result, diagnostics) = solidcraft_kernel::capture_boolean_diagnostics(|| {
        session.execute("solid.extrude", &json!({"sketch":7,"profiles":[{"curves":["c2"]}],"distance":10,"operation":"join"}))
    });
    eprintln!("join in {:?}: {result:?}\n{diagnostics:#?}", start.elapsed());
    assert!(result.is_ok(), "{result:?}");
    std::fs::write(r"E:\FreeCad\solidcraft\target\join-fixed.solidcraft", session.doc.to_json()).unwrap();
}
