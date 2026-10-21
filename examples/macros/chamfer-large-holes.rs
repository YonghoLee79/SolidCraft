//! Chamfer both ends of every cylindrical hole wider than 2 mm on the open design.
//!
//! Start part: `examples/macros/enclosure.start.json` (shelled box, lip, snap, two Ø2.5 bosses).
//! Live: `solidcraft --control 7878` then `SOLIDCRAFT_CONTROL_PORT=7878 cargo run -p solidcraft-macros --bin chamfer-large-holes`
//! Headless: `cargo run -p solidcraft-macros --bin chamfer-large-holes` (needs `solidcraft-cli` on PATH).

use serde_json::json;
use solidcraft_macro_client::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    let holes = app.run("inspect.find_holes", json!({"min_diameter": "2 mm"}))?;
    let list = holes.get("holes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    eprintln!("{} hole(s) wider than 2 mm", list.len());
    for h in list {
        let edges = h.get("edges").cloned().unwrap_or(json!([]));
        match app.run("solid.chamfer", json!({"edges": edges, "distance": "1 mm"})) {
            Ok(r) if r.get("skipped").and_then(|v| v.as_bool()) == Some(true) => {}
            Ok(_) => {}
            Err(e) => eprintln!("skip diameter={}: {e}", h.get("diameter").unwrap_or(&json!(null))),
        }
    }
    Ok(())
}
