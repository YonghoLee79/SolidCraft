//! A closed triangle mesh to solid cells: rays along +x through every (y, z) column of cell
//! centres cross the surface an even number of times, and cells whose centres lie between an
//! odd crossing and the next one are inside the body.

use solidcraft_geom::Mesh;

use crate::{Cell, Grid, Result, SimError};

/// Ray offsets in cell sizes. Cell centres often lie exactly on a triangle edge (a box's
/// diagonal) or vertex, where a ray would count one crossing twice or not at all; shifting every
/// ray by the same small, irrational-looking amount keeps it off those lines for the meshes a
/// CAD kernel makes.
const NUDGE: (f64, f64) = (1.234_567e-6, 2.718_281e-6);

/// Mark the cells inside `mesh` as solid (fluid cells elsewhere are left as they are).
pub fn voxelize(mesh: &Mesh, grid: &mut Grid) -> Result<()> {
    if mesh.triangles.is_empty() {
        return Err(SimError::EmptyMesh);
    }
    let [nx, ny, nz] = grid.dims;
    let s = grid.spacing;
    let (oy, oz) = (grid.origin.y + (0.5 + NUDGE.0) * s, grid.origin.z + (0.5 + NUDGE.1) * s);
    // Crossing x per column (j + ny * k).
    let mut crossings: Vec<Vec<f64>> = vec![Vec::new(); ny * nz];
    for t in &mesh.triangles {
        let [a, b, c] = mesh.tri(t).ok_or(SimError::BadMesh)?;
        // Twice the signed area of the triangle seen along x; zero for triangles parallel to x.
        let area = (b.y - a.y) * (c.z - a.z) - (c.y - a.y) * (b.z - a.z);
        if !area.is_finite() || area.abs() < 1e-18 {
            continue;
        }
        let column = |v: f64, o: f64, n: usize| ((v - o) / s).clamp(-1.0, n as f64);
        let (j0, j1) = (column(a.y.min(b.y).min(c.y), oy, ny).ceil(), column(a.y.max(b.y).max(c.y), oy, ny).floor());
        let (k0, k1) = (column(a.z.min(b.z).min(c.z), oz, nz).ceil(), column(a.z.max(b.z).max(c.z), oz, nz).floor());
        if j1 < j0 || k1 < k0 {
            continue;
        }
        for k in (k0.max(0.0) as usize)..=(k1.min(nz as f64 - 1.0) as usize) {
            let z = oz + k as f64 * s;
            for j in (j0.max(0.0) as usize)..=(j1.min(ny as f64 - 1.0) as usize) {
                let y = oy + j as f64 * s;
                // Barycentric weights of (y, z) in the projected triangle.
                let wb = ((y - a.y) * (c.z - a.z) - (c.y - a.y) * (z - a.z)) / area;
                let wc = ((b.y - a.y) * (z - a.z) - (y - a.y) * (b.z - a.z)) / area;
                let wa = 1.0 - wb - wc;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                if let Some(col) = crossings.get_mut(j + ny * k) {
                    col.push(wa * a.x + wb * b.x + wc * c.x);
                }
            }
        }
    }
    for k in 0..nz {
        for j in 0..ny {
            let Some(col) = crossings.get_mut(j + ny * k) else { continue };
            if col.len() % 2 == 1 {
                return Err(SimError::NotClosed);
            }
            col.sort_by(f64::total_cmp);
            for &[x0, x1] in col.as_chunks::<2>().0 {
                let first = (((x0 - grid.origin.x) / s) - 0.5).ceil().max(0.0) as usize;
                let last = (((x1 - grid.origin.x) / s) - 0.5).floor();
                if last < 0.0 {
                    continue;
                }
                for i in first..=(last as usize).min(nx.saturating_sub(1)) {
                    if let Some(cell) = grid.index(i, j, k).and_then(|idx| grid.cells.get_mut(idx)) {
                        *cell = Cell::Solid;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use solidcraft_geom::Vec3;
    use solidcraft_geom::{Aabb3, Mesh};

    use super::*;

    /// A closed axis-aligned box, triangles counter-clockwise from outside.
    pub(crate) fn add_box(mesh: &mut Mesh, min: [f64; 3], max: [f64; 3]) {
        let base = mesh.positions.len() as u32;
        for k in 0..8 {
            let pick = |bit: usize, axis: usize| if k & bit == 0 { min[axis] } else { max[axis] };
            mesh.positions.push(Vec3 { x: pick(1, 0), y: pick(2, 1), z: pick(4, 2) });
        }
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        for [a, b, c, d] in quads {
            mesh.triangles.push([base + a, base + b, base + c]);
            mesh.triangles.push([base + a, base + c, base + d]);
        }
    }

    /// A square duct along x, 40 long, 20 × 20 outside with 4-thick walls (four boxes that meet
    /// only along faces parallel to x).
    pub(crate) fn square_duct() -> Mesh {
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [40.0, 20.0, 4.0]);
        add_box(&mut m, [0.0, 0.0, 16.0], [40.0, 20.0, 20.0]);
        add_box(&mut m, [0.0, 0.0, 4.0], [40.0, 4.0, 16.0]);
        add_box(&mut m, [0.0, 16.0, 4.0], [40.0, 20.0, 16.0]);
        m
    }

    fn bounds(m: &Mesh) -> Aabb3 {
        m.bounds()
    }

    #[test]
    fn a_cube_fills_its_volume_in_cells() {
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let mut g = Grid::around(bounds(&m), 1.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        assert_eq!(g.dims, [14, 14, 14]);
        assert_eq!(g.count(Cell::Solid), 1000);
        assert_eq!(g.get(2, 2, 2), Some(Cell::Solid));
        assert_eq!(g.get(1, 2, 2), Some(Cell::Fluid));
    }

    #[test]
    fn a_duct_wall_is_solid_and_its_bore_stays_fluid() {
        let m = square_duct();
        let mut g = Grid::around(bounds(&m), 2.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        // (20 × 20 − 12 × 12) × 40 mm³ of wall in 8 mm³ cells.
        assert_eq!(g.count(Cell::Solid), 1280);
        let mid = |x: f64, y: f64, z: f64| {
            let c = |v: f64, o: f64| ((v - o) / 2.0).floor() as usize;
            g.get(c(x, g.origin.x), c(y, g.origin.y), c(z, g.origin.z))
        };
        assert_eq!(mid(20.0, 10.0, 10.0), Some(Cell::Fluid), "the bore");
        assert_eq!(mid(20.0, 2.0, 10.0), Some(Cell::Solid), "a side wall");
    }

    #[test]
    fn an_open_surface_and_hostile_sizes_are_refused() {
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        m.triangles.truncate(10);
        let mut g = Grid::around(bounds(&m), 1.0, 1, 1_000_000).unwrap();
        assert_eq!(voxelize(&m, &mut g), Err(SimError::NotClosed));
        for spacing in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Grid::around(bounds(&m), spacing, 1, 1_000).err(), Some(SimError::InvalidSpacing), "{spacing}");
        }
        assert!(matches!(Grid::around(bounds(&m), 1e-6, 1, 1_000_000), Err(SimError::TooManyCells { .. })));
        m.triangles.push([0, 1, 999]);
        assert_eq!(voxelize(&m, &mut g), Err(SimError::BadMesh));
        assert_eq!(voxelize(&Mesh::default(), &mut g), Err(SimError::EmptyMesh));
    }
}
