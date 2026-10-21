//! Cut a ring of holes on a pitch circle (default: 6× Ø6.5 mm on Ø70 mm, 10 mm plate).
//!
//! `bolt-circle [count] [pcd_mm] [hole_dia_mm]`

use serde_json::json;
use solidcraft_macro_client::App;

fn arg(i: usize, default: &str) -> String {
    std::env::args().nth(i).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count: u32 = arg(1, "6").parse().unwrap_or(6);
    let pcd: f64 = arg(2, "70").parse().unwrap_or(70.0);
    let hole = arg(3, "6.5");
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    app.run("solid.cylinder", json!({"radius": 50, "height": 10, "name": "Flange"}))?;
    let r = pcd / 2.0;
    for i in 0..count.max(1) {
        let a = std::f64::consts::TAU * f64::from(i) / f64::from(count.max(1));
        let x = r * a.cos();
        let y = r * a.sin();
        app.run("solid.hole", json!({"position": [x, y, 10], "diameter": format!("{hole} mm"), "name": format!("Bolt{}", i + 1)}))?;
    }
    Ok(())
}
