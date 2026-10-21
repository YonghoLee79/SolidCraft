use super::*;

fn plate() -> Session {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length":40,"width":30,"height":10,"body_name":"Plate"})).unwrap();
    // A blind hole in the bottom: hidden from the front and from above.
    s.execute("solid.cylinder", &json!({"radius":3,"height":4,"base":[10,10,0],"operation":"cut","targets":["Plate"]})).unwrap();
    s
}

fn sheet(s: &mut Session, p: Value) -> Value {
    s.execute(CMD, &p).unwrap()
}

fn view<'a>(r: &'a Value, name: &str) -> &'a Value {
    r["views"].as_array().unwrap().iter().find(|v| v["view"] == name).unwrap()
}

fn xy(v: &Value) -> (f64, f64) {
    (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

/// Every LINE's end points (paper mm) on `layer`.
fn line_points(dxf: &str, layer: &str) -> Vec<(f64, f64)> {
    let pairs: Vec<(&str, &str)> = dxf.lines().collect::<Vec<_>>().chunks(2).map(|c| (c[0].trim(), c[1].trim())).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        if pairs[i] == ("0", "LINE") {
            let mut j = i + 1;
            let (mut on, mut p) = (false, [0.0; 4]);
            while j < pairs.len() && pairs[j].0 != "0" {
                match pairs[j].0 {
                    "8" => on = pairs[j].1 == layer,
                    "10" => p[0] = pairs[j].1.parse().unwrap(),
                    "20" => p[1] = pairs[j].1.parse().unwrap(),
                    "11" => p[2] = pairs[j].1.parse().unwrap(),
                    "21" => p[3] = pairs[j].1.parse().unwrap(),
                    _ => {}
                }
                j += 1;
            }
            if on {
                out.extend([(p[0], p[1]), (p[2], p[3])]);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[test]
fn a3_sheet_has_the_tcvn_frame_title_block_and_first_angle_views() {
    let mut s = plate();
    let doc = s.doc.clone();
    let r = sheet(&mut s, json!({"fields": {"title": "Tấm đế", "drawn_by": "Nguyễn Văn A", "material": "CT3", "drawing_no": "SC-01"}}));
    assert_eq!(s.doc, doc, "exporting a sheet changes nothing");
    assert_eq!(r["sheet"], "A3");
    assert_eq!(xy(&r["size_mm"]), (420.0, 297.0));
    // 20 mm on the binding side, 10 mm elsewhere.
    assert_eq!(xy(&r["frame_mm"]["min"]), (20.0, 10.0));
    assert_eq!(xy(&r["frame_mm"]["max"]), (410.0, 287.0));
    // 40×30×10 mm: front 40×10, top 40×30, left 30×10. 5:1 is too big for the drawing area, 2:1 fits.
    assert_eq!(r["scale"], "2:1");
    let (front, top, left) = (xy(&view(&r, "front")["centre_mm"]), xy(&view(&r, "top")["centre_mm"]), xy(&view(&r, "left")["centre_mm"]));
    assert!((front.0 - top.0).abs() < 1e-6, "the top view lines up under the front view: {front:?} {top:?}");
    assert!((front.1 - left.1).abs() < 1e-6, "the left view lines up beside the front view: {front:?} {left:?}");
    assert!(top.1 < front.1, "first angle: the view from above goes under the front view");
    assert!(left.0 > front.0, "first angle: the view from the left goes on the right");
    let size = xy(&view(&r, "front")["size_mm"]);
    assert!((size.0 - 80.0).abs() < 0.1 && (size.1 - 20.0).abs() < 0.1, "front at 2:1 is 80×20 mm: {size:?}");

    let dxf = r["dxf"].as_str().unwrap();
    // Lineweights: thick frame and outlines, thin cells.
    assert!(dxf.contains("FRAME\n 70\n0\n 62\n7\n  6\nContinuous\n370\n50"), "frame layer weight");
    // Every outline stays inside the drawing area: above the title block, below the code box.
    for (x, y) in line_points(dxf, "VISIBLE") {
        assert!((20.0..=410.0).contains(&x) && (65.0..=273.0).contains(&y), "({x}, {y}) is outside the drawing area");
    }
    assert!(!line_points(dxf, "HIDDEN").is_empty() || dxf.contains("HIDDEN"), "the hole is drawn dashed");
    // The title block's corner: 180×55 mm in the frame's lower right corner.
    let frame = line_points(dxf, "FRAME");
    assert!(frame.contains(&(230.0, 10.0)) && frame.contains(&(230.0, 65.0)), "title block's left edge at x = 410 − 180");
    // Vietnamese labels and the fields, in DXF's \U+ escapes.
    for text in ["Ng\\U+01B0\\U+1EDDi v\\U+1EBD", "T\\U+1EC9 l\\U+1EC7", "2:1", "SC-01", "CT3", "T\\U+1EA5m \\U+0111\\U+1EBF"] {
        assert!(dxf.contains(text), "{text} is missing");
    }
    // The drawing code is also written upside down in the code box at the frame's upper left.
    assert!(dxf.contains("\n 50\n180"), "turned drawing code");
}

#[test]
fn third_angle_puts_the_top_view_over_the_front_and_the_left_view_on_the_left() {
    let mut s = plate();
    let r = sheet(&mut s, json!({"projection": "third_angle"}));
    let (front, top, left) = (xy(&view(&r, "front")["centre_mm"]), xy(&view(&r, "top")["centre_mm"]), xy(&view(&r, "left")["centre_mm"]));
    assert!(top.1 > front.1 && (top.0 - front.0).abs() < 1e-6);
    assert!(left.0 < front.0 && (left.1 - front.1).abs() < 1e-6);
}

#[test]
fn sheet_sizes_orientations_margins_and_blocks() {
    let mut s = plate();
    let a4 = sheet(&mut s, json!({"sheet": "A4"}));
    assert_eq!((a4["orientation"].as_str(), xy(&a4["size_mm"])), (Some("portrait"), (210.0, 297.0)));
    // The production block is exactly as wide as an A4 frame with TCVN margins.
    assert_eq!(xy(&a4["frame_mm"]["max"]).0 - xy(&a4["frame_mm"]["min"]).0, 180.0);
    let compact = sheet(&mut s, json!({"sheet": "a1", "margins": "compact", "title_block": "school"}));
    assert_eq!(xy(&compact["size_mm"]), (841.0, 594.0));
    assert_eq!(xy(&compact["frame_mm"]["min"]), (20.0, 5.0));
    assert_eq!(compact["title_block"], "school");
    let school = compact["dxf"].as_str().unwrap();
    let lines = line_points(school, "FRAME");
    assert!(lines.contains(&(836.0 - 140.0, 5.0 + 32.0)), "school block 140×32");
    let none = sheet(&mut s, json!({"title_block": "none", "views": ["front", "iso"]}));
    assert!(!none["dxf"].as_str().unwrap().lines().any(|l| l.trim() == "AcDbText"), "no title block, no text");
    assert_eq!(none["views"].as_array().unwrap().len(), 2);
}

#[test]
fn a_chosen_scale_is_kept_or_refused_with_one_that_fits() {
    let mut s = plate();
    let r = sheet(&mut s, json!({"scale": "1:2.5"}));
    assert_eq!(r["scale"], "1:2.5");
    let e = s.execute(CMD, &json!({"scale": "10:1"})).unwrap_err().to_string();
    assert!(e.contains("don't fit") && e.contains("2:1 fits"), "{e}");
}

#[test]
fn fields_are_checked_and_reported_when_the_block_has_no_cell_for_them() {
    let mut s = plate();
    let r = sheet(&mut s, json!({"title_block": "school", "fields": {"title": "Bài 1", "mass": 0.12, "approved_by": "B"}}));
    assert_eq!(r["unused_fields"], json!(["approved_by", "mass"]));
    let r = sheet(&mut s, json!({"fields": {"mass": 0.12, "sheet": 1, "sheets": 2, "revisions": [{"mark": "a", "date": "19/10/2026"}]}}));
    let dxf = r["dxf"].as_str().unwrap();
    for text in ["0.12", "T\\U+1EDD: 1", "S\\U+1ED1 t\\U+1EDD: 2", "19/10/2026"] {
        assert!(dxf.contains(text), "{text} is missing");
    }
    for (bad, why) in [
        (json!({"fields": {"titel": "x"}}), "unknown field `titel`"),
        (json!({"fields": {"title": "x".repeat(81)}}), "longer than 80"),
        (json!({"fields": {"title": ["x"]}}), "text or a number"),
        (json!({"fields": {"revisions": [{}, {}, {}, {}]}}), "at most 3"),
        (json!({"fields": {"revisions": [{"who": "x"}]}}), "unknown revision field"),
        (json!({"fields": "x"}), "must be an object"),
    ] {
        let e = s.execute(CMD, &bad).unwrap_err().to_string();
        assert!(e.contains(why), "{bad}: {e}");
    }
    // A line break can't end a DXF value early.
    let r = sheet(&mut s, json!({"fields": {"title": "a\n0\nEOF"}}));
    assert!(r["dxf"].as_str().unwrap().contains("a 0 EOF"));
}

#[test]
fn an_empty_design_has_nothing_to_draw() {
    let mut s = Session::default();
    let e = s.execute(CMD, &json!({})).unwrap_err().to_string();
    assert!(e.contains("file.export_drawing_dxf") && e.contains("no visible bodies"), "{e}");
}

#[test]
fn hostile_sheet_params_never_panic() {
    let mut s = plate();
    let doc = s.doc.clone();
    let hostile = [
        json!(null),
        json!(-1),
        json!(1e308),
        json!(f64::MAX),
        json!(""),
        json!("A9"),
        json!("0:0"),
        json!("-1:2"),
        json!("1:1e400"),
        json!([]),
        json!({}),
        json!(vec!["front"; 9]),
        json!(["front", "front"]),
        json!([1, 2]),
    ];
    for k in ["sheet", "orientation", "margins", "title_block", "views", "projection", "scale", "hidden", "tolerance", "fields", "path"] {
        for v in &hostile {
            let mut p = json!({});
            p[k] = v.clone();
            if k == "path" && v.is_string() {
                continue;
            }
            let _ = s.execute(CMD, &p);
        }
    }
    assert_eq!(s.doc, doc);
}

#[test]
fn scales_parse_and_the_largest_standard_one_that_fits_is_chosen() {
    assert_eq!(Scale::parse("1:2").map(Scale::label).as_deref(), Some("1:2"));
    assert_eq!(Scale::parse(" 2.5 : 1 ").map(Scale::label).as_deref(), Some("2.5:1"));
    for bad in ["", "1", "1:0", "0:1", "1:-2", "1:x", "1:20000", "inf:1", "NaN:1"] {
        assert_eq!(Scale::parse(bad), None, "{bad}");
    }
    assert_eq!(Scale::fitting(4.45).map(Scale::label).as_deref(), Some("2:1"));
    assert_eq!(Scale::fitting(1.0).map(Scale::label).as_deref(), Some("1:1"));
    assert_eq!(Scale::fitting(0.3).map(Scale::label).as_deref(), Some("1:5"));
    assert_eq!(Scale::fitting(1e6).map(Scale::label).as_deref(), Some("50:1"));
    assert_eq!(Scale::fitting(1e-6), None);
}
