//! Unfold the current sheet-metal body and write a DXF.
//!
//! `flatten-export-dxf [out.dxf]`

use serde_json::json;
use solidcraft_macro_client::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "flat.dxf".into());
    let mut app = if std::env::var("SOLIDCRAFT_CONTROL_PORT").is_ok() { App::connect()? } else { App::headless()? };
    app.run("sheet.flat_pattern", json!({}))?;
    app.run("sheet.export_dxf", json!({"path": path}))?;
    Ok(())
}
