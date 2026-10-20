//! World-frame metadata at a timeline body ownership boundary.
use solidcraft_geom::{Plane, Transform};

use crate::{DocError, ModelState, Result};

pub(crate) fn remap_sheet_metadata(st: &mut ModelState, names: &[String], transform: &Transform) -> Result<()> {
    if names.len() > 1000 || st.sheets.len().saturating_add(st.threads.len()) > 100_000 || !transform.is_finite() {
        return Err(DocError::Invalid("body transfer metadata exceeds its finite processing budget".into()));
    }
    let names: std::collections::BTreeSet<&str> = names.iter().map(String::as_str).collect();
    if !st.sheets.iter().any(|s| names.contains(s.body.as_str())) && !st.threads.iter().any(|t| names.contains(t.body.as_str())) {
        return Ok(());
    }
    if !crate::valid_transfer_matrix(transform) {
        return Err(DocError::Invalid("sheet and thread transfer requires a rigid placement".into()));
    }
    let frames: Vec<_> = st
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| names.contains(s.body.as_str()))
        .map(|(i, s)| {
            let frame = Plane::new(transform.point(s.frame.origin), transform.vector(s.frame.x), transform.vector(s.frame.y))
                .ok_or_else(|| DocError::Invalid("invalid transferred sheet frame".into()))?;
            Ok((i, frame))
        })
        .collect::<Result<_>>()?;
    let threads: Vec<_> = st
        .threads
        .iter()
        .enumerate()
        .filter(|(_, t)| names.contains(t.body.as_str()))
        .map(|(i, t)| {
            let point = transform.point(t.axis_point);
            let axis = transform.vector(*t.axis).unit().ok_or_else(|| DocError::Invalid("invalid transferred thread axis".into()))?;
            if !point.is_finite() {
                return Err(DocError::Invalid("invalid transferred thread placement".into()));
            }
            Ok((i, point, axis))
        })
        .collect::<Result<_>>()?;
    for (i, frame) in frames {
        if let Some(s) = st.sheets.get_mut(i) {
            s.frame = frame;
        }
    }
    for (i, point, axis) in threads {
        if let Some(t) = st.threads.get_mut(i) {
            t.axis_point = point;
            t.axis = axis;
        }
    }
    Ok(())
}
