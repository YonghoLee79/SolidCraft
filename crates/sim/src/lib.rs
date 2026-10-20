//! Flow simulation setup, without GPU code: a body's triangle mesh becomes a Cartesian voxel
//! grid of solid and fluid cells ([`voxelize`]), and a flood fill from a seed finds the fluid
//! region a study runs in ([`fluid_region`]), reporting whether it leaks to the grid's edge.
//! The solvers (wgpu compute) sit above this crate, beside the app's GPU code.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod fluid;
mod grid;
pub mod lbm;
mod voxelize;

pub use fluid::{Region, fluid_region};
pub use grid::{Cell, Grid};
pub use lbm::{Lattice, Opening};
pub use voxelize::voxelize;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SimError {
    #[error("the cell size must be a positive length")]
    InvalidSpacing,
    #[error("the grid would have {cells} cells, more than the limit of {max}; use larger cells")]
    TooManyCells { cells: u64, max: u64 },
    #[error("the body has no triangles to voxelize")]
    EmptyMesh,
    #[error("the body's mesh refers to a vertex it does not have")]
    BadMesh,
    #[error("the body's surface is not closed, so its inside cannot be found")]
    NotClosed,
    #[error("the relaxation time must be above ½ and the force finite")]
    InvalidRelaxation,
    #[error("an opening must be fluid cells with fluid just inside, a finite speed under 0.3 or a positive density")]
    BadOpening,
    #[error("the seed cell is outside the grid or not fluid")]
    BadSeed,
}

pub type Result<T> = std::result::Result<T, SimError>;
