//! Chamfer the edges currently selected in the live app (1 mm). Requires `SOLIDCRAFT_CONTROL_PORT`.

use serde_json::json;
use solidcraft_macro_client::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::connect()?;
    let info = app.inspect()?;
    let mut edges = Vec::new();
    if let Some(sel) = info.get("selection").and_then(|v| v.as_array()) {
        for item in sel {
            if item.get("type").and_then(|t| t.as_str()) == Some("edge")
                && let Some(pt) = item.get("point")
            {
                edges.push(pt.clone());
            }
        }
    }
    app.run("solid.chamfer", json!({"edges": edges, "distance": "1 mm"}))?;
    Ok(())
}
