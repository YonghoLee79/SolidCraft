//! Bounded coefficient caches for small B-spline support grids.
//! Ordered accumulation adapted from truck-geometry 0.5.0 BSplineSurface::subs
//! (Apache-2.0, copyright RICOS Co. Ltd.); see ATTRIBUTION.md.
use truck_modeling::{self as mt, EuclideanSpace};

const GRID_SIDE: usize = 51;
const BASIS_WORDS: usize = 8;

pub(crate) fn batched(surface: &mt::Surface, u: &[f64], v: &[f64]) -> Option<Vec<mt::Point3>> {
    let mt::Surface::BSplineSurface(s) = surface else { return None };
    if u.len() != GRID_SIDE || v.len() != GRID_SIDE {
        return None;
    }
    let rows = s.control_points().len();
    let columns = s.control_points().first()?.len();
    if columns == 0 {
        return None;
    }
    // Match valid vendor degrees; malformed shapes retain the original guarded path.
    let du = s.uknot_vec().len().checked_sub(rows)?.checked_sub(1)?;
    let dv = s.vknot_vec().len().checked_sub(columns)?.checked_sub(1)?;
    if s.uknot_vec().len().checked_sub(1)? > BASIS_WORDS || s.vknot_vec().len().checked_sub(1)? > BASIS_WORDS {
        return None;
    }
    // Fixed caches contain 6,528 bytes. One temporary vendor basis Vec is requested
    // for at most 8 words; allocation metadata/slack is outside that cache payload.
    let mut bu = [[0.; BASIS_WORDS]; GRID_SIDE];
    let mut bv = [[0.; BASIS_WORDS]; GRID_SIDE];
    for (basis, &parameter) in bu.iter_mut().zip(u) {
        let values = s.uknot_vec().try_bspline_basis_functions(du, parameter).ok()?;
        if values.len() != rows || values.capacity() > BASIS_WORDS {
            return None;
        }
        for (slot, value) in basis.iter_mut().zip(&values) {
            *slot = *value
        }
    }
    for (basis, &parameter) in bv.iter_mut().zip(v) {
        let values = s.vknot_vec().try_bspline_basis_functions(dv, parameter).ok()?;
        if values.len() != columns || values.capacity() > BASIS_WORDS {
            return None;
        }
        for (slot, value) in basis.iter_mut().zip(&values) {
            *slot = *value
        }
    }
    let mut points = Vec::with_capacity(GRID_SIDE * GRID_SIDE);
    for basis0 in &bu {
        let basis0 = basis0.get(..rows)?;
        for basis1 in &bv {
            let basis1 = basis1.get(..columns)?;
            points.push(
                s.control_points()
                    .iter()
                    .zip(basis0)
                    .fold(mt::Point3::origin(), |sum, (row, &a)| row.iter().zip(basis1).fold(sum, |sum, (point, &b)| sum + point.to_vec() * (a * b))),
            );
        }
    }
    Some(points)
}

#[cfg(test)]
#[path = "grid_basis_tests.rs"]
mod tests;
