//! A drawing of the visible bodies from a supplied viewport camera. No screen pixels, grid,
//! selection highlights or tessellation diagonals are exported.

#[path = "view_dxf_meshes.rs"]
mod meshes;
#[path = "view_dxf_progress.rs"]
mod progress;
#[path = "view_dxf_projection.rs"]
mod projection;

use serde_json::{Value, json};
use solidcraft_geom::{Aabb3, Vec3};
use solidcraft_io::{DxfEntity, DxfLayer, DxfLinetype, DxfShape};
use solidcraft_render::{Camera, StandardView};

use super::CommandSpec;
use crate::params::{bad, str_};
use crate::{Result, Session};

const CMD: &str = "file.export_view_dxf";
pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(CMD, "Export Current View as DXF", export)
    .icon("export")
    .noundo()
    .params("camera: Camera (current view); hidden?: omit|dashed (default omit); tolerance?: 0.0001..1 mm (curve chord tolerance, default 0.01); path?: .dxf file (else returns dxf text). Visible placed bodies only; 2D LINE/LWPOLYLINE entities on VISIBLE/HIDDEN layers, mm; perspective projected onto the camera target plane; no viewport crop or overlays.")];

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cam: Camera =
        serde_json::from_value(p.get("camera").cloned().ok_or_else(|| bad(CMD, "camera is required"))?).map_err(|_| bad(CMD, "invalid camera"))?;
    if !cam.is_valid()
        || !(0.0..std::f64::consts::PI).contains(&cam.fov)
        || cam.pitch.abs() > std::f64::consts::FRAC_PI_2
        || cam.yaw.abs() > 1e6
        || cam.roll.abs() > 1e6
    {
        return Err(bad(CMD, "invalid camera"));
    }
    let dashed = match p.get("hidden") {
        None => false,
        Some(Value::String(v)) if v == "omit" => false,
        Some(Value::String(v)) if v == "dashed" => true,
        _ => return Err(bad(CMD, "hidden must be omit or dashed")),
    };
    let tolerance = match p.get("tolerance") {
        None => 0.01,
        Some(v) => v.as_f64().filter(|t| (0.0001..=1.0).contains(t)).ok_or_else(|| bad(CMD, "tolerance must be 0.0001..1 mm"))?,
    };
    let path = match p.get("path") {
        None => None,
        Some(_) => Some(str_(p, "path").filter(|v| !v.trim().is_empty() && v.len() < 4096).ok_or_else(|| bad(CMD, "invalid path"))?),
    };
    if let Some(path) = path {
        solidcraft_io::vfs::check(path).map_err(|e| crate::EngineError::Other(e.to_string()))?;
    }
    let started = std::time::Instant::now();
    progress::Stage::new("dxf_meshes", 1).check().map_err(|e| bad(CMD, e))?;
    let meshes = meshes::prepare(s, tolerance)?;
    let prepare_ms = started.elapsed().as_secs_f64() * 1000.;
    let drawing = projection::drawing(&meshes, &cam).map_err(|e| bad(CMD, e))?;
    let writing = progress::Stage::new("dxf_write", 1);
    writing.check().map_err(|e| bad(CMD, e))?;
    let mut visible = 0;
    let mut hidden = 0;
    let mut entities = Vec::new();
    for line in drawing.lines {
        if line.hidden && !dashed {
            continue;
        }
        let layer = if line.hidden {
            hidden += 1;
            "HIDDEN"
        } else {
            visible += 1;
            "VISIBLE"
        };
        entities.push(DxfEntity { layer: layer.into(), shape: DxfShape::Line(line.a, line.b) });
    }
    let entities = projection::connect(entities);
    let mut layers = vec![DxfLayer::new("VISIBLE", 7)];
    if dashed {
        layers.push(DxfLayer { linetype: DxfLinetype::Hidden, ..DxfLayer::new("HIDDEN", 8) });
    }
    let text = solidcraft_io::write_dxf_drawing(&layers, &entities);
    writing.check().map_err(|e| bad(CMD, e))?;
    let mut result = json!({"visible":visible,"hidden":hidden,"units":"mm","tolerance":tolerance,"projection":if cam.fov > 0.0 {"perspective"} else {"orthographic"}});
    result["stats"] = json!({"bodies":meshes.placements.len(),"shared_meshes":meshes.meshes.len(),"triangles":meshes.triangles,
        "mesh_bytes":meshes.bytes,"worker_limit":meshes::WORKERS,"segments":drawing.segments,"occlusion_work":drawing.work,
        "prepare_ms":prepare_ms,"projection_ms":drawing.projection_ms,"occlusion_ms":drawing.occlusion_ms});
    if let Some(path) = path {
        solidcraft_io::vfs::check(path).map_err(|e| crate::EngineError::Other(e.to_string()))?;
        solidcraft_io::write_atomic(std::path::Path::new(path), text.as_bytes(), false).map_err(|e| crate::EngineError::Other(e.to_string()))?;
        result["path"] = json!(path);
    } else {
        result["dxf"] = json!(text);
    }
    Ok(result)
}

/// Parallel views of the visible bodies, one list of entities each (layers VISIBLE and, when
/// `dashed`, HIDDEN), in millimetres along each view's right and up about the centre of the
/// bodies' bounding box, so views that share an axis line up. The meshes are prepared once.
pub(super) fn standard_views(s: &Session, cmd: &str, views: &[StandardView], tolerance: f64, dashed: bool) -> Result<Vec<Vec<DxfEntity>>> {
    progress::Stage::new("dxf_meshes", 1).check().map_err(|e| bad(cmd, e))?;
    // Its errors name this command, not the one-view export's.
    let meshes = meshes::prepare(s, tolerance).map_err(|e| match e {
        crate::EngineError::BadParams { msg, .. } => bad(cmd, msg),
        e => e,
    })?;
    let bounds = meshes
        .placements
        .iter()
        .filter_map(|p| meshes.meshes.get(p.mesh).map(|m| meshes::transformed_bounds(m, &p.transform)))
        .fold(Aabb3::EMPTY, |b, p| b.union(&p));
    if bounds.is_empty() || !bounds.diagonal().is_finite() {
        return Err(bad(cmd, "there are no shown bodies to draw"));
    }
    let target: Vec3 = bounds.center();
    // Outside the bodies; a parallel view looks the same from any distance.
    let distance = 10.0 + 3.0 * bounds.diagonal();
    let mut out = Vec::with_capacity(views.len());
    for view in views {
        let mut cam = Camera { target, fov: 0.0, distance, ..Default::default() };
        cam.set_view(*view);
        let drawing = projection::drawing(&meshes, &cam).map_err(|e| bad(cmd, e))?;
        let entities = drawing
            .lines
            .into_iter()
            .filter(|l| dashed || !l.hidden)
            .map(|l| DxfEntity { layer: if l.hidden { "HIDDEN" } else { "VISIBLE" }.into(), shape: DxfShape::Line(l.a, l.b) })
            .collect();
        out.push(projection::connect(entities));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "view_dxf_tests.rs"]
mod tests;
