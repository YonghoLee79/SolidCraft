//! Cut one hole on the top face of the open solid and pattern it on a pitch circle.
//!
//! Needs an open part (`examples/macros/flange-blank.start.json`).
//! `bolt-circle [count] [pcd_mm] [hole_dia_mm]`

use serde_json::json;
use solidcraft_macro_client::App;

fn arg(i: usize, default: &str) -> String {
    std::env::args().nth(i).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
}

fn coord(v: &serde_json::Value, i: usize) -> Option<f64> {
    v.as_array()?.get(i)?.as_f64()
}

fn top_face_z(faces: &[serde_json::Value]) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for f in faces {
        let Some(n) = f.get("plane_normal") else { continue };
        let Some(nz) = coord(n, 2) else { continue };
        if nz < 0.9 {
            continue;
        }
        let Some(area) = f.get("area").and_then(serde_json::Value::as_f64) else { continue };
        let z = f.get("centroid").and_then(|c| coord(c, 2)).or_else(|| f.get("point").and_then(|p| coord(p, 2)));
        let Some(z) = z else { continue };
        if best.is_none_or(|(a, _)| area > a) {
            best = Some((area, z));
        }
    }
    best.map(|(_, z)| z)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count: u32 = arg(1, "6").parse().unwrap_or(6);
    let pcd: f64 = arg(2, "70").parse().unwrap_or(70.0);
    let hole = arg(3, "6.5");
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    let info = app.inspect()?;
    let bodies = info.get("bodies").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let name = bodies.first().and_then(|b| b.get("name")).and_then(|n| n.as_str()).filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Err("open a solid first (examples/macros/flange-blank.start.json)".into());
    };
    let faces = app.run("model.faces", json!({"body": name}))?;
    let list = faces.get("faces").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let z = top_face_z(&list).ok_or("no top face on the open solid")?;
    let r = pcd / 2.0;
    app.run("solid.hole", json!({"position": [r, 0.0, z], "diameter": format!("{hole} mm"), "name": "BoltHole"}))?;
    app.run("solid.pattern.circular", json!({"features": ["BoltHole"], "axis": "Z", "count": count.max(1)}))?;
    Ok(())
}
