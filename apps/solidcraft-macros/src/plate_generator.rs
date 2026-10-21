//! Build a rectangular plate with a hole in each corner from five values (args or defaults).
//!
//! `plate-generator [length_mm] [width_mm] [height_mm] [hole_dia_mm] [offset_mm]`

use serde_json::json;
use solidcraft_macro_client::App;

fn arg(i: usize, default: &str) -> String {
    std::env::args().nth(i).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let length = arg(1, "120");
    let width = arg(2, "80");
    let height = arg(3, "8");
    let hole = arg(4, "6.5");
    let offset = arg(5, "10");
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    app.run(
        "solid.box",
        json!({"length": format!("{length} mm"), "width": format!("{width} mm"), "height": format!("{height} mm"), "name": "Plate"}),
    )?;
    let corners = [
        (offset.clone(), offset.clone()),
        (format!("{length} - {offset}"), offset.clone()),
        (offset.clone(), format!("{width} - {offset}")),
        (format!("{length} - {offset}"), format!("{width} - {offset}")),
    ];
    for (i, (x, y)) in corners.iter().enumerate() {
        app.run(
            "solid.hole",
            json!({"position": [x, y, format!("{height} mm")], "diameter": format!("{hole} mm"), "name": format!("Hole{}", i + 1)}),
        )?;
    }
    Ok(())
}
