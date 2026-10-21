use super::*;

const EPS: f64 = 1e-9;

fn iso_layout(size: PaperSize, orientation: Orientation) -> BorderLayout {
    let sheet = Sheet::standard(size, orientation);
    BorderProfile::iso(&sheet).layout(&sheet).unwrap()
}

fn lengths(fields: &[Field]) -> Vec<f64> {
    fields.iter().map(|f| f.to_mm - f.from_mm).collect()
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < EPS)
}

#[test]
fn every_size_is_its_published_millimetres() {
    use PaperSize::*;
    let expected = [
        (A0, 841.0, 1189.0),
        (A1, 594.0, 841.0),
        (A2, 420.0, 594.0),
        (A3, 297.0, 420.0),
        (A4, 210.0, 297.0),
        (A5, 148.0, 210.0),
        (Letter, 215.9, 279.4),
        (Legal, 215.9, 355.6),
        (AnsiA, 215.9, 279.4),
        (AnsiB, 279.4, 431.8),
        (AnsiC, 431.8, 558.8),
        (AnsiD, 558.8, 863.6),
        (AnsiE, 863.6, 1117.6),
        (AnsiF, 711.2, 1016.0),
        (ArchA, 228.6, 304.8),
        (ArchB, 304.8, 457.2),
        (ArchC, 457.2, 609.6),
        (ArchD, 609.6, 914.4),
        (ArchE, 914.4, 1219.2),
        (ArchE1, 762.0, 1066.8),
    ];
    assert_eq!(expected.len(), PaperSize::ALL.len());
    for (size, short, long) in expected {
        let (s, l) = size.sides_mm();
        assert!((s - short).abs() < EPS && (l - long).abs() < EPS, "{size:?}: {s} x {l}");
    }
}

#[test]
fn landscape_is_the_default_and_either_orientation_turns_every_size() {
    let sheet: Sheet = serde_json::from_value(serde_json::json!({"size": {"standard": "a3"}})).unwrap();
    assert_eq!(sheet.orientation, Orientation::Landscape);
    assert_eq!(sheet.paper_mm().unwrap(), Vec2::new(420.0, 297.0));
    for size in PaperSize::ALL {
        let across = Sheet::standard(size, Orientation::Landscape).paper_mm().unwrap();
        let upright = Sheet::standard(size, Orientation::Portrait).paper_mm().unwrap();
        assert!(across.x > across.y && upright.x < upright.y, "{size:?}");
        assert_eq!(across, Vec2::new(upright.y, upright.x));
    }
    // A custom sheet's sides may come in either order; the orientation lays it.
    for sides_mm in [[500.0, 700.0], [700.0, 500.0]] {
        let sheet = Sheet { size: SheetSize::Custom { sides_mm }, orientation: Orientation::Portrait };
        assert_eq!(sheet.paper_mm().unwrap(), Vec2::new(500.0, 700.0));
    }
}

#[test]
fn a3_landscape_has_a_390_by_277_frame_and_8_by_6_fields() {
    let border = iso_layout(PaperSize::A3, Orientation::Landscape);
    assert_eq!(border.paper, Aabb2 { min: Vec2::ZERO, max: Vec2::new(420.0, 297.0) });
    assert_eq!(border.frame, Aabb2 { min: Vec2::new(20.0, 10.0), max: Vec2::new(410.0, 287.0) });
    assert_eq!(border.frame_line_mm, 0.7);
    // Columns: 50 mm out from the centre line at 210; the left corner takes the 40 mm left over.
    assert!(close(&lengths(&border.columns), &[40.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0]));
    assert_eq!(border.columns.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), ["1", "2", "3", "4", "5", "6", "7", "8"]);
    assert!(border.columns.iter().any(|f| f.to_mm == 210.0));
    // Rows: 148.5 above and below 10 and 287, so two of 50 and a 38.5 mm corner each way.
    assert!(close(&lengths(&border.rows), &[38.5, 50.0, 50.0, 50.0, 50.0, 38.5]));
    assert_eq!(border.rows.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), ["A", "B", "C", "D", "E", "F"]);
    let (top, bottom) = (border.rows.first().unwrap(), border.rows.last().unwrap());
    assert_eq!((top.from_mm, top.to_mm), (248.5, 287.0));
    assert_eq!((bottom.from_mm, bottom.to_mm), (10.0, 48.5));
    assert_eq!(border.label_sides, Sides::ALL);
}

#[test]
fn the_iso_rule_gives_the_standard_field_counts_for_a0_to_a4_either_way() {
    // ISO 5457's table, long side by short side.
    for (size, long, short) in
        [(PaperSize::A0, 24, 16), (PaperSize::A1, 16, 12), (PaperSize::A2, 12, 8), (PaperSize::A3, 8, 6), (PaperSize::A4, 6, 4)]
    {
        let across = iso_layout(size, Orientation::Landscape);
        assert_eq!((across.columns.len(), across.rows.len()), (long, short), "{size:?} landscape");
        let upright = iso_layout(size, Orientation::Portrait);
        assert_eq!((upright.columns.len(), upright.rows.len()), (short, long), "{size:?} portrait");
    }
    // A1 across: 400.5 left of the centre line and 410.5 right of it; eight fields each way.
    let a1 = lengths(&iso_layout(PaperSize::A1, Orientation::Landscape).columns);
    assert!((a1.first().unwrap() - 50.5).abs() < EPS && (a1.last().unwrap() - 60.5).abs() < EPS, "{a1:?}");
}

#[test]
fn only_a4_is_labelled_on_its_top_and_right_alone() {
    for orientation in [Orientation::Portrait, Orientation::Landscape] {
        let border = iso_layout(PaperSize::A4, orientation);
        assert_eq!(border.label_sides, Sides { top: true, bottom: false, left: false, right: true });
    }
    for size in PaperSize::ALL.into_iter().filter(|s| *s != PaperSize::A4) {
        assert_eq!(iso_layout(size, Orientation::Landscape).label_sides, Sides::ALL, "{size:?}");
    }
}

#[test]
fn an_equal_division_cuts_the_frame_evenly() {
    let sheet = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let profile = BorderProfile { grid: GridRule::Equal { columns: 8, rows: 6 }, ..BorderProfile::iso(&sheet) };
    let border = profile.layout(&sheet).unwrap();
    assert!(close(&lengths(&border.columns), &[48.75; 8]));
    assert!(close(&lengths(&border.rows), &[277.0 / 6.0; 6]));
    assert!((277.0 / 6.0 - 46.166_666_666_7_f64).abs() < 1e-9);
    let none = BorderProfile { grid: GridRule::None, ..profile }.layout(&sheet).unwrap();
    assert!(none.columns.is_empty() && none.rows.is_empty());
    assert_eq!(none.frame, border.frame);
}

#[test]
fn rows_skip_i_and_o_and_letters_go_on_past_z() {
    let a0 = iso_layout(PaperSize::A0, Orientation::Landscape);
    let rows: String = a0.rows.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(rows, "ABCDEFGHJKLMNPQR");
    assert_eq!(letters(0, true), "A");
    assert_eq!(letters(23, true), "Z");
    assert_eq!(letters(24, true), "AA");
    assert_eq!(letters(25, true), "AB");
    assert_eq!(letters(24 + 24 * 24 - 1, true), "ZZ");
    assert_eq!(letters(24 + 24 * 24, true), "AAA");
    assert_eq!(letters(8, false), "I");
    assert_eq!(letters(25, false), "Z");
    assert_eq!(letters(26, false), "AA");
    assert_eq!(letters(usize::MAX, true).len(), letters(usize::MAX - 1, true).len());
    for i in 0..2000 {
        assert!(!letters(i, true).contains(['I', 'O']), "{i}");
    }
    let sheet = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let every = BorderProfile { skip_i_and_o: false, grid: GridRule::Equal { columns: 1, rows: 10 }, ..BorderProfile::iso(&sheet) };
    let rows: String = every.layout(&sheet).unwrap().rows.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(rows, "ABCDEFGHIJ");
}

#[test]
fn hostile_sizes_and_profiles_are_refused() {
    let custom = |a: f64, b: f64| Sheet { size: SheetSize::Custom { sides_mm: [a, b] }, orientation: Orientation::Landscape };
    for (a, b) in [(0.0, 100.0), (-1.0, 100.0), (f64::NAN, 100.0), (100.0, f64::INFINITY), (5_000.000_001, 100.0), (f64::MIN_POSITIVE, f64::MAX)] {
        assert!(custom(a, b).paper_mm().is_err(), "{a} x {b}");
    }
    assert_eq!(custom(5000.0, 5000.0).paper_mm().unwrap(), Vec2::new(5000.0, 5000.0));
    let a3 = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let iso = BorderProfile::iso(&a3);
    let with_margins = |left_mm, right_mm, top_mm, bottom_mm| BorderProfile { margins: Margins { left_mm, right_mm, top_mm, bottom_mm }, ..iso };
    for profile in [
        with_margins(-1.0, 10.0, 10.0, 10.0),
        with_margins(f64::NAN, 10.0, 10.0, 10.0),
        with_margins(20.0, f64::INFINITY, 10.0, 10.0),
        with_margins(20.0, 10.0, 5000.1, 10.0),
        with_margins(210.0, 210.0, 10.0, 10.0),
        with_margins(20.0, 10.0, 148.0, 148.5),
        // A millimetre of drawing space, but the centre line at 210 lies outside it.
        with_margins(200.0, 219.0, 10.0, 10.0),
        BorderProfile { frame_line_mm: 0.0, ..iso },
        BorderProfile { frame_line_mm: -0.7, ..iso },
        BorderProfile { frame_line_mm: f64::NAN, ..iso },
        BorderProfile { frame_line_mm: 10.000_001, ..iso },
        BorderProfile { grid: GridRule::FromCentre { field_mm: 0.999 }, ..iso },
        BorderProfile { grid: GridRule::FromCentre { field_mm: 1000.1 }, ..iso },
        BorderProfile { grid: GridRule::FromCentre { field_mm: f64::NAN }, ..iso },
        BorderProfile { grid: GridRule::FromCentre { field_mm: f64::INFINITY }, ..iso },
        BorderProfile { grid: GridRule::Equal { columns: 0, rows: 6 }, ..iso },
        BorderProfile { grid: GridRule::Equal { columns: 8, rows: 0 }, ..iso },
        BorderProfile { grid: GridRule::Equal { columns: 201, rows: 1 }, ..iso },
        BorderProfile { grid: GridRule::Equal { columns: u32::MAX, rows: u32::MAX }, ..iso },
    ] {
        assert!(profile.layout(&a3).is_err(), "accepted {profile:?}");
    }
    // The same space with an equal division is fine, and so are the limits themselves.
    let off_centre = BorderProfile { grid: GridRule::Equal { columns: 2, rows: 2 }, ..with_margins(200.0, 219.0, 10.0, 10.0) };
    assert!((off_centre.layout(&a3).unwrap().frame.max.x - 201.0).abs() < EPS);
    for profile in [
        BorderProfile { frame_line_mm: 10.0, ..iso },
        BorderProfile { grid: GridRule::FromCentre { field_mm: 1.0 }, ..iso },
        BorderProfile { grid: GridRule::FromCentre { field_mm: 1000.0 }, ..iso },
        BorderProfile { grid: GridRule::Equal { columns: 200, rows: 200 }, ..iso },
        with_margins(0.0, 0.0, 0.0, 0.0),
    ] {
        profile.layout(&a3).unwrap();
    }
}

#[test]
fn names_round_trip_and_unknown_fields_are_refused() {
    for (size, name) in
        [(PaperSize::A0, "a0"), (PaperSize::A5, "a5"), (PaperSize::Letter, "letter"), (PaperSize::AnsiA, "ansi_a"), (PaperSize::ArchE1, "arch_e1")]
    {
        assert_eq!(serde_json::to_value(size).unwrap(), serde_json::json!(name));
    }
    for size in PaperSize::ALL {
        let back: PaperSize = serde_json::from_value(serde_json::to_value(size).unwrap()).unwrap();
        assert_eq!(back, size);
    }
    let sheet = Sheet { size: SheetSize::Custom { sides_mm: [500.0, 700.0] }, orientation: Orientation::Portrait };
    let value = serde_json::to_value(sheet).unwrap();
    assert_eq!(value, serde_json::json!({"size": {"custom": {"sides_mm": [500.0, 700.0]}}, "orientation": "portrait"}));
    assert_eq!(serde_json::from_value::<Sheet>(value).unwrap(), sheet);
    let a3 = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let profile = BorderProfile::iso(&a3);
    let value = serde_json::to_value(profile).unwrap();
    assert_eq!(value["grid"], serde_json::json!({"rule": "from_centre", "field_mm": 50.0}));
    assert_eq!(serde_json::from_value::<BorderProfile>(value.clone()).unwrap(), profile);
    for bad in [
        serde_json::json!({"size": {"standard": "a6"}}),
        serde_json::json!({"size": {"standard": "a3"}, "scale": 1}),
        serde_json::json!({"size": {"custom": {"sides_mm": [1, 2], "units": "in"}}}),
        serde_json::json!({"size": {"custom": {"sides_mm": [1, 2, 3]}}}),
        serde_json::json!({"size": {"standard": "a3"}, "orientation": "upside_down"}),
    ] {
        assert!(serde_json::from_value::<Sheet>(bad.clone()).is_err(), "{bad}");
    }
    let mut extra = value.clone();
    extra["title_block"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BorderProfile>(extra).is_err());
    let mut extra = value;
    extra["grid"]["columns"] = serde_json::json!(8);
    assert!(serde_json::from_value::<BorderProfile>(extra).is_err());
}

/// For every accepted sheet and profile, the fields tile the frame exactly, top to bottom and
/// left to right, every field is longer than nothing and nothing is NaN.
#[test]
fn fields_tile_every_frame_exactly() {
    let mut sheets: Vec<Sheet> =
        PaperSize::ALL.into_iter().flat_map(|size| [Orientation::Landscape, Orientation::Portrait].map(|o| Sheet::standard(size, o))).collect();
    for sides_mm in [[1.5, 1.5], [5000.0, 5000.0], [5000.0, 1.0], [17.3, 4999.9], [100.0, 101.0], [123.456, 789.012]] {
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            sheets.push(Sheet { size: SheetSize::Custom { sides_mm }, orientation });
        }
    }
    let margins = [
        Margins { left_mm: 20.0, right_mm: 10.0, top_mm: 10.0, bottom_mm: 10.0 },
        Margins { left_mm: 0.0, right_mm: 0.0, top_mm: 0.0, bottom_mm: 0.0 },
        Margins { left_mm: 5.0, right_mm: 5.0, top_mm: 5.0, bottom_mm: 5.0 },
        Margins { left_mm: 33.3, right_mm: 0.1, top_mm: 7.7, bottom_mm: 19.0 },
    ];
    let grids = [
        GridRule::FromCentre { field_mm: 50.0 },
        GridRule::FromCentre { field_mm: 1.0 },
        GridRule::FromCentre { field_mm: 7.5 },
        GridRule::FromCentre { field_mm: 33.3 },
        GridRule::FromCentre { field_mm: 1000.0 },
        GridRule::Equal { columns: 1, rows: 1 },
        GridRule::Equal { columns: 3, rows: 7 },
        GridRule::Equal { columns: 200, rows: 200 },
    ];
    let mut accepted = 0;
    for sheet in &sheets {
        for margins in margins {
            for grid in grids {
                let profile = BorderProfile { margins, frame_line_mm: 0.7, grid, skip_i_and_o: true, label_sides: Sides::ALL };
                let Ok(border) = profile.layout(sheet) else { continue };
                accepted += 1;
                let frame = border.frame;
                assert!(frame.min.x >= 0.0 && frame.min.y >= 0.0 && frame.max.x <= border.paper.max.x && frame.max.y <= border.paper.max.y);
                let check = |fields: &[Field], lo: f64, hi: f64, falling: bool| {
                    let (first, last) = (fields.first().unwrap(), fields.last().unwrap());
                    let (start, end) = if falling { (last.from_mm, first.to_mm) } else { (first.from_mm, last.to_mm) };
                    assert_eq!((start, end), (lo, hi), "{sheet:?} {profile:?}");
                    for f in fields {
                        assert!(f.from_mm.is_finite() && f.to_mm.is_finite() && f.to_mm - f.from_mm > EPS, "{f:?} {sheet:?} {profile:?}");
                    }
                    for w in fields.windows(2) {
                        let (a, b) = (&w[0], &w[1]);
                        if falling { assert_eq!(a.from_mm, b.to_mm) } else { assert_eq!(a.to_mm, b.from_mm) }
                    }
                    let total: f64 = fields.iter().map(|f| f.to_mm - f.from_mm).sum();
                    assert!((total - (hi - lo)).abs() < 1e-6, "{total} vs {}", hi - lo);
                    let mut labels: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
                    labels.sort_unstable();
                    labels.dedup();
                    assert_eq!(labels.len(), fields.len());
                };
                check(&border.columns, frame.min.x, frame.max.x, false);
                check(&border.rows, frame.min.y, frame.max.y, true);
                if let GridRule::FromCentre { field_mm } = grid {
                    // Inner fields are exactly the field length, and the centre lines are edges.
                    let centre = border.paper.max.x / 2.0;
                    assert!(border.columns.iter().any(|f| f.to_mm == centre));
                    let inner = border.columns.len().saturating_sub(2);
                    for f in border.columns.iter().skip(1).take(inner) {
                        assert!((f.to_mm - f.from_mm - field_mm).abs() < 1e-9, "{f:?}");
                    }
                }
            }
        }
    }
    assert!(accepted > 1000, "{accepted}");
}

#[test]
fn rounding_past_a_whole_number_of_fields_leaves_no_sliver() {
    // 420 mm across with the centre line at 210 and the frame to 410: the right half is four fields
    // of 50. The left margin puts the left half at 150 mm, three fields, give or take rounding noise.
    let sheet = Sheet { size: SheetSize::Custom { sides_mm: [420.0, 297.0] }, orientation: Orientation::Landscape };
    for hair in [0.0, 1e-12, 1e-10, -1e-10] {
        let margins = Margins { left_mm: 60.0 - hair, right_mm: 10.0, top_mm: 10.0, bottom_mm: 10.0 };
        let border = BorderProfile { margins, ..BorderProfile::iso(&sheet) }.layout(&sheet).unwrap();
        let widths = lengths(&border.columns);
        // The noise joins the corner field: no field of a hair's width before it.
        assert_eq!(widths.len(), 7, "{hair}: {widths:?}");
        assert!(close(&widths, &[50.0; 7]), "{hair}: {widths:?}");
        assert_eq!(border.columns.first().unwrap().from_mm, 60.0 - hair);
    }
}
