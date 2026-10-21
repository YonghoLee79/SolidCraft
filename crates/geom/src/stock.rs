//! Stock size (#137): the tightest box around a part along its own directions, as the board or
//! sheet it is cut from. Sizes are longest first (length ≥ width ≥ thickness), never world X/Y/Z.
//!
//! Not built yet: the tests come first (PR #3995), and until then both functions return `None`.

use crate::Vec3;

/// The box a part is cut from: its sizes longest first, the unit direction of each (length,
/// width, thickness) and the box's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StockBox {
    pub length: f64,
    pub width: f64,
    pub thickness: f64,
    pub axes: [Vec3; 3],
    pub center: Vec3,
}

/// Eigenvalues (largest first) and unit eigenvectors of a symmetric 3 × 3 matrix, such as an
/// inertia tensor. `None` for non-finite input.
pub fn eigen_sym3(m: [[f64; 3]; 3]) -> Option<([f64; 3], [Vec3; 3])> {
    let _ = m;
    None
}

/// The box of `points` along three orthonormal `axes`, its sizes sorted longest first with their
/// axes. `None` for no points, non-finite values or axes that are not orthonormal.
pub fn box_along(points: &[Vec3], axes: [Vec3; 3]) -> Option<StockBox> {
    let _ = (points, axes);
    None
}

#[cfg(test)]
#[path = "stock_tests.rs"]
mod tests;
