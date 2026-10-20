//! The CPU reference of the Airflow solver: lattice Boltzmann D3Q19 with BGK collision, Guo body
//! forcing and half-way bounce-back walls, in lattice units (cell size 1, time step 1). It is the
//! oracle the GPU kernels are checked against, so it favours plain code and `f64` over speed.
//! Openings (a velocity inlet, a pressure outlet) use Guo's non-equilibrium extrapolation from
//! the cell just inside. Turbulence: the Smagorinsky model raises each cell's relaxation time with
//! its local shear, read from the non-equilibrium momentum flux (Hou et al. 1996).

use crate::{Cell, Grid, Result, SimError};

/// Lattice directions: rest, 6 faces, 12 edges.
pub const C: [[i32; 3]; 19] = [
    [0, 0, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
    [1, 1, 0],
    [-1, -1, 0],
    [1, -1, 0],
    [-1, 1, 0],
    [1, 0, 1],
    [-1, 0, -1],
    [1, 0, -1],
    [-1, 0, 1],
    [0, 1, 1],
    [0, -1, -1],
    [0, 1, -1],
    [0, -1, 1],
];
/// Each direction's opposite (the directions above come in opposite pairs after the rest one).
pub const OPP: [usize; 19] = [0, 2, 1, 4, 3, 6, 5, 8, 7, 10, 9, 12, 11, 14, 13, 16, 15, 18, 17];
pub const W: [f64; 19] = {
    let (r, f, e) = (1.0 / 3.0, 1.0 / 18.0, 1.0 / 36.0);
    [r, f, f, f, f, f, f, e, e, e, e, e, e, e, e, e, e, e, e]
};
const Q: usize = 19;

pub struct Lattice {
    pub dims: [usize; 3],
    /// Relaxation time; the kinematic viscosity is (tau − ½) / 3.
    pub tau: f64,
    /// Body force per unit volume (lattice units), e.g. a pressure gradient driving a channel.
    pub force: [f64; 3],
    /// Which axes wrap around; on the others the grid's outer faces are walls.
    pub periodic: [bool; 3],
    /// The Smagorinsky constant (0: laminar BGK; about 0.1–0.17 for turbulent flow).
    smagorinsky: f64,
    solid: Vec<bool>,
    f: Vec<f64>,
    next: Vec<f64>,
    /// Opening cells: (cell, the fluid cell just inside it, what is imposed there).
    openings: Vec<(usize, usize, Opening)>,
}

/// What an opening imposes on the flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Opening {
    /// A velocity inlet (lattice units).
    Velocity([f64; 3]),
    /// A pressure outlet, as a density (pressure = density / 3).
    Density(f64),
}

fn equilibrium(q: usize, rho: f64, u: [f64; 3]) -> f64 {
    let c = C[q];
    let cu = 3.0 * (c[0] as f64 * u[0] + c[1] as f64 * u[1] + c[2] as f64 * u[2]);
    let uu = 1.5 * (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]);
    W[q] * rho * (1.0 + cu + 0.5 * cu * cu - uu)
}

impl Lattice {
    /// Fluid at rest (density 1) on the grid's fluid cells.
    pub fn new(grid: &Grid, tau: f64, force: [f64; 3], periodic: [bool; 3]) -> Result<Lattice> {
        if !tau.is_finite() || tau <= 0.5 || force.iter().any(|g| !g.is_finite()) {
            return Err(SimError::InvalidRelaxation);
        }
        let n = grid.len();
        let len = n.checked_mul(Q).ok_or(SimError::TooManyCells { cells: n as u64, max: (usize::MAX / Q) as u64 })?;
        let solid: Vec<bool> = grid.cells.iter().map(|c| *c == Cell::Solid).collect();
        let mut f = vec![0.0; len];
        for (cell, dist) in f.chunks_mut(Q).enumerate() {
            if !solid.get(cell).copied().unwrap_or(true) {
                for (q, v) in dist.iter_mut().enumerate() {
                    *v = W[q];
                }
            }
        }
        Ok(Lattice { dims: grid.dims, tau, force, periodic, solid, next: f.clone(), f, openings: Vec::new(), smagorinsky: 0.0 })
    }

    /// Model turbulence with the Smagorinsky constant `cs` (0 turns it off).
    pub fn with_smagorinsky(mut self, cs: f64) -> Result<Lattice> {
        if !cs.is_finite() || !(0.0..=0.5).contains(&cs) {
            return Err(SimError::InvalidRelaxation);
        }
        self.smagorinsky = cs;
        Ok(self)
    }

    /// The relaxation time of a cell with density `rho` whose populations are `dist` around the
    /// equilibrium `feq`: the molecular one plus the Smagorinsky eddy viscosity's share.
    fn local_tau(&self, dist: &[f64], feq: &[f64; Q], rho: f64) -> f64 {
        if self.smagorinsky == 0.0 {
            return self.tau;
        }
        let mut pi = [[0.0f64; 3]; 3];
        for q in 0..Q {
            let neq = dist.get(q).copied().unwrap_or(0.0) - feq[q];
            for (a, row) in pi.iter_mut().enumerate() {
                for (b, v) in row.iter_mut().enumerate() {
                    *v += f64::from(C[q][a] * C[q][b]) * neq;
                }
            }
        }
        let norm = pi.iter().flatten().map(|v| v * v).sum::<f64>().sqrt();
        let cs2 = self.smagorinsky * self.smagorinsky;
        0.5 * (self.tau + (self.tau * self.tau + 18.0 * std::f64::consts::SQRT_2 * cs2 * norm / rho).sqrt())
    }

    /// Make fluid `cells` an opening; `inward` points from each of them to the fluid cell just
    /// inside, which must be fluid too.
    pub fn add_opening(&mut self, cells: &[[usize; 3]], inward: [i32; 3], kind: Opening) -> Result<()> {
        let valid = match kind {
            Opening::Velocity(u) => u.iter().all(|v| v.is_finite() && v.abs() < 0.3),
            Opening::Density(rho) => rho.is_finite() && rho > 0.0,
        };
        if !valid {
            return Err(SimError::BadOpening);
        }
        for at in cells {
            let cell = self.index(at[0], at[1], at[2]).ok_or(SimError::BadOpening)?;
            let inside = self.neighbour(*at, inward).ok_or(SimError::BadOpening)?;
            if self.solid.get(cell).copied().unwrap_or(true) || self.solid.get(inside).copied().unwrap_or(true) {
                return Err(SimError::BadOpening);
            }
            self.openings.push((cell, inside, kind));
        }
        Ok(())
    }

    /// Rewrite the opening cells: the imposed density or velocity, the other moment and the
    /// non-equilibrium part taken from the cell just inside.
    fn apply_openings(&mut self) {
        for &(cell, inside, kind) in &self.openings {
            let Some(src) = self.f.get(inside * Q..(inside + 1) * Q) else { continue };
            let (rho_in, u_in) = Self::moments_of(src, self.force);
            let (rho, u) = match kind {
                Opening::Velocity(u) => (rho_in, u),
                Opening::Density(rho) => (rho, u_in),
            };
            let mut out = [0.0; Q];
            for (q, v) in out.iter_mut().enumerate() {
                let fq = src.get(q).copied().unwrap_or(0.0);
                *v = equilibrium(q, rho, u) + fq - equilibrium(q, rho_in, u_in);
            }
            if let Some(dst) = self.f.get_mut(cell * Q..(cell + 1) * Q) {
                dst.copy_from_slice(&out);
            }
        }
    }

    pub fn viscosity(&self) -> f64 {
        (self.tau - 0.5) / 3.0
    }

    /// The Smagorinsky constant (0: laminar).
    pub fn smagorinsky(&self) -> f64 {
        self.smagorinsky
    }

    /// Which cells are solid, in grid order.
    pub fn solid(&self) -> &[bool] {
        &self.solid
    }

    /// The opening cells: (cell, the fluid cell just inside it, what is imposed there).
    pub fn openings(&self) -> &[(usize, usize, Opening)] {
        &self.openings
    }

    /// Every cell's populations, cell after cell: direction `q` of cell `c` is at `c * 19 + q`
    /// (solid cells hold zeros).
    pub fn populations(&self) -> &[f64] {
        &self.f
    }

    /// Replace the populations, for example with a GPU run's: as many finite values as
    /// [`Lattice::populations`] has. Solid cells keep their zeros.
    pub fn set_populations(&mut self, f: &[f64]) -> Result<()> {
        if f.len() != self.f.len() || f.iter().any(|v| !v.is_finite()) {
            return Err(SimError::BadPopulations);
        }
        for ((dst, src), solid) in self.f.chunks_mut(Q).zip(f.chunks(Q)).zip(&self.solid) {
            if !solid {
                dst.copy_from_slice(src);
            }
        }
        Ok(())
    }

    fn index(&self, i: usize, j: usize, k: usize) -> Option<usize> {
        let [nx, ny, nz] = self.dims;
        (i < nx && j < ny && k < nz).then_some(i + nx * (j + ny * k))
    }

    /// The neighbour of (i, j, k) along `c`, wrapped on periodic axes; none past a wall face.
    fn neighbour(&self, at: [usize; 3], c: [i32; 3]) -> Option<usize> {
        let mut p = [0usize; 3];
        for a in 0..3 {
            let n = self.dims[a] as i64;
            let mut v = at[a] as i64 + i64::from(c[a]);
            if v < 0 || v >= n {
                if !self.periodic[a] {
                    return None;
                }
                v = v.rem_euclid(n);
            }
            p[a] = v as usize;
        }
        self.index(p[0], p[1], p[2])
    }

    /// Density and velocity of a fluid cell (the velocity includes half the force, as Guo's
    /// scheme defines it).
    pub fn moments(&self, i: usize, j: usize, k: usize) -> Option<(f64, [f64; 3])> {
        let cell = self.index(i, j, k)?;
        if self.solid.get(cell).copied().unwrap_or(true) {
            return None;
        }
        let dist = self.f.get(cell * Q..(cell + 1) * Q)?;
        Some(Self::moments_of(dist, self.force))
    }

    fn moments_of(dist: &[f64], force: [f64; 3]) -> (f64, [f64; 3]) {
        let rho: f64 = dist.iter().sum();
        let mut m = [0.0; 3];
        for (q, v) in dist.iter().enumerate() {
            for a in 0..3 {
                m[a] += v * C[q][a] as f64;
            }
        }
        let u = [(m[0] + 0.5 * force[0]) / rho, (m[1] + 0.5 * force[1]) / rho, (m[2] + 0.5 * force[2]) / rho];
        (rho, u)
    }

    /// One time step: collide every fluid cell, then stream to its neighbours; a population
    /// heading into a wall comes back the opposite way (half-way bounce-back).
    pub fn step(&mut self) {
        let [nx, ny, nz] = self.dims;
        let g = self.force;
        let (mut post, mut feq) = ([0.0; Q], [0.0; Q]);
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let Some(cell) = self.index(i, j, k) else { continue };
                    if self.solid.get(cell).copied().unwrap_or(true) {
                        continue;
                    }
                    let Some(dist) = self.f.get(cell * Q..(cell + 1) * Q) else { continue };
                    let (rho, u) = Self::moments_of(dist, g);
                    for (q, e) in feq.iter_mut().enumerate() {
                        *e = equilibrium(q, rho, u);
                    }
                    let tau = self.local_tau(dist, &feq, rho);
                    for q in 0..Q {
                        let c = C[q].map(f64::from);
                        let cu = c[0] * u[0] + c[1] * u[1] + c[2] * u[2];
                        let mut source = 0.0;
                        for a in 0..3 {
                            source += (3.0 * (c[a] - u[a]) + 9.0 * cu * c[a]) * g[a];
                        }
                        let fq = dist.get(q).copied().unwrap_or(0.0);
                        post[q] = fq - (fq - feq[q]) / tau + (1.0 - 0.5 / tau) * W[q] * source;
                    }
                    for (q, value) in post.iter().enumerate() {
                        let to = match self.neighbour([i, j, k], C[q]) {
                            Some(n) if !self.solid.get(n).copied().unwrap_or(true) => n * Q + q,
                            _ => cell * Q + OPP[q],
                        };
                        if let Some(slot) = self.next.get_mut(to) {
                            *slot = *value;
                        }
                    }
                }
            }
        }
        std::mem::swap(&mut self.f, &mut self.next);
        self.apply_openings();
    }
}

#[cfg(test)]
mod tests {
    use solidcraft_geom::{Aabb3, Vec3};

    use super::*;

    /// A channel between two plates (z walls), periodic along x and y, driven by a body force:
    /// plane Poiseuille flow, u(z) = g / (2ν) · (z − z0)(z1 − z), with the walls half a cell
    /// beyond the last fluid cells. BGK with half-way bounce-back reproduces it closely.
    #[test]
    fn plane_poiseuille_flow_matches_the_parabola() {
        let (nz, h) = (18usize, 16.0);
        let bounds = Aabb3 { min: Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: Vec3 { x: 1.0, y: 1.0, z: nz as f64 } };
        let mut grid = Grid::around(bounds, 1.0, 0, 1_000).unwrap();
        assert_eq!(grid.dims, [1, 1, nz]);
        for k in [0, nz - 1] {
            let idx = grid.index(0, 0, k).unwrap();
            grid.cells[idx] = Cell::Solid;
        }
        let g = 1e-6;
        let mut lattice = Lattice::new(&grid, 0.8, [g, 0.0, 0.0], [true, true, false]).unwrap();
        for _ in 0..20_000 {
            lattice.step();
        }
        let nu = lattice.viscosity();
        let mut worst = 0.0f64;
        for k in 1..nz - 1 {
            let z = k as f64 - 0.5; // from the lower wall, which sits at k = ½
            let exact = g / (2.0 * nu) * z * (h - z);
            let (rho, u) = lattice.moments(0, 0, k).unwrap();
            assert!((rho - 1.0).abs() < 1e-9, "mass is kept: {rho}");
            assert!(u[1].abs() < 1e-12 && u[2].abs() < 1e-12, "{u:?}");
            worst = worst.max((u[0] - exact).abs() / (g * h * h / (8.0 * nu)));
        }
        assert!(worst < 5e-3, "worst error {worst} of the peak speed");
    }

    /// The whole path: a square duct as a closed mesh, voxelized, then driven along its bore.
    /// Laminar flow in a square duct of side a carries Q = g a⁴ / (12 ν) · (1 − 192/π⁵ Σ
    /// tanh(nπ/2)/n⁵, n odd) ≈ 0.03514 g a⁴ / ν.
    #[test]
    fn square_duct_flow_rate_matches_the_series_solution() {
        let mut mesh = solidcraft_geom::Mesh::default();
        let (outer, wall, len) = (20.0, 4.0, 2.0);
        let add = crate::voxelize::tests::add_box;
        add(&mut mesh, [0.0, 0.0, 0.0], [len, outer, wall]);
        add(&mut mesh, [0.0, 0.0, outer - wall], [len, outer, outer]);
        add(&mut mesh, [0.0, 0.0, wall], [len, wall, outer - wall]);
        add(&mut mesh, [0.0, outer - wall, wall], [len, outer, outer - wall]);
        let mut grid = Grid::around(mesh.bounds(), 1.0, 0, 10_000).unwrap();
        crate::voxelize(&mesh, &mut grid).unwrap();
        let a = outer - 2.0 * wall;
        assert_eq!(grid.count(Cell::Fluid), (len * a * a) as usize);
        let g = 1e-6;
        let mut lattice = Lattice::new(&grid, 0.8, [g, 0.0, 0.0], [true, false, false]).unwrap();
        for _ in 0..12_000 {
            lattice.step();
        }
        let flow: f64 = (0..grid.dims[1])
            .flat_map(|j| (0..grid.dims[2]).map(move |k| (j, k)))
            .filter_map(|(j, k)| lattice.moments(0, j, k))
            .map(|(_, u)| u[0])
            .sum();
        let sum: f64 = [1.0f64, 3.0, 5.0, 7.0, 9.0].iter().map(|n| (n * std::f64::consts::PI / 2.0).tanh() / n.powi(5)).sum();
        let exact = g * a.powi(4) / (12.0 * lattice.viscosity()) * (1.0 - 192.0 / std::f64::consts::PI.powi(5) * sum);
        let error = (flow - exact).abs() / exact;
        assert!(error < 0.02, "flow {flow:e}, exact {exact:e}, error {error}");
    }

    /// Air blown into a square duct at a set speed and let out at a set pressure: what goes in
    /// comes out, and once the flow has developed the pressure falls at the exact laminar rate,
    /// dp/dx = ν Q / (0.03514 a⁴) for flow rate Q.
    #[test]
    fn a_duct_with_an_inlet_and_an_outlet_keeps_mass_and_the_laminar_pressure_drop() {
        let mut mesh = solidcraft_geom::Mesh::default();
        let (outer, wall, len) = (18.0, 3.0, 36.0);
        let add = crate::voxelize::tests::add_box;
        add(&mut mesh, [0.0, 0.0, 0.0], [len, outer, wall]);
        add(&mut mesh, [0.0, 0.0, outer - wall], [len, outer, outer]);
        add(&mut mesh, [0.0, 0.0, wall], [len, wall, outer - wall]);
        add(&mut mesh, [0.0, outer - wall, wall], [len, outer, outer - wall]);
        let mut grid = Grid::around(mesh.bounds(), 1.0, 0, 100_000).unwrap();
        crate::voxelize(&mesh, &mut grid).unwrap();
        let [nx, ny, nz] = grid.dims;
        let bore: Vec<(usize, usize)> =
            (0..ny).flat_map(|j| (0..nz).map(move |k| (j, k))).filter(|&(j, k)| grid.get(0, j, k) == Some(Cell::Fluid)).collect();
        let a = outer - 2.0 * wall;
        assert_eq!(bore.len(), (a * a) as usize);
        let u0 = 0.02;
        let mut lattice = Lattice::new(&grid, 0.8, [0.0; 3], [false; 3]).unwrap();
        let inlet: Vec<[usize; 3]> = bore.iter().map(|&(j, k)| [0, j, k]).collect();
        let outlet: Vec<[usize; 3]> = bore.iter().map(|&(j, k)| [nx - 1, j, k]).collect();
        lattice.add_opening(&inlet, [1, 0, 0], Opening::Velocity([u0, 0.0, 0.0])).unwrap();
        lattice.add_opening(&outlet, [-1, 0, 0], Opening::Density(1.0)).unwrap();
        for _ in 0..8_000 {
            lattice.step();
        }
        let section = |i: usize| -> (f64, f64) {
            let (mut flow, mut rho) = (0.0, 0.0);
            for &(j, k) in &bore {
                let (r, u) = lattice.moments(i, j, k).unwrap();
                flow += r * u[0];
                rho += r;
            }
            (flow, rho / bore.len() as f64)
        };
        // A uniform inlet speed also lands on the cells against the walls, where no-slip fights
        // it: on a 12-cell bore the flow delivered is a few percent under u0 · a². Known; a
        // flow-rate inlet that corrects its speed to the target comes later, and results report
        // the measured flow.
        let nominal = u0 * a * a;
        let (q_mid, rho_a) = section(12);
        let (q_out, rho_b) = section(28);
        assert!((q_mid - nominal).abs() / nominal < 0.06, "flow {q_mid} vs {nominal} nominal");
        assert!((q_out - q_mid).abs() / q_mid < 0.002, "mass kept: {q_out} out vs {q_mid}");
        let gradient = (rho_a - rho_b) / 3.0 / 16.0;
        let exact = lattice.viscosity() * q_mid / (0.035_144 * a.powi(4));
        let error = (gradient - exact).abs() / exact;
        assert!(error < 0.05, "dp/dx {gradient:e}, exact {exact:e}, error {error}");
        // Openings are checked like any input.
        assert!(lattice.add_opening(&inlet, [1, 0, 0], Opening::Velocity([1.0, 0.0, 0.0])).is_err());
        assert!(lattice.add_opening(&[[0, 0, 0]], [1, 0, 0], Opening::Density(1.0)).is_err(), "a wall cell");
        assert!(lattice.add_opening(&outlet, [1, 0, 0], Opening::Density(-1.0)).is_err());
    }

    /// The Smagorinsky model adds eddy viscosity only where there is shear: a duct's flow slows
    /// a little, never speeds up, and constant 0 is the laminar solver exactly.
    #[test]
    fn smagorinsky_adds_viscosity_with_shear_and_zero_is_laminar() {
        let bounds = Aabb3 { min: Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: Vec3 { x: 1.0, y: 1.0, z: 18.0 } };
        let mut grid = Grid::around(bounds, 1.0, 0, 1_000).unwrap();
        for k in [0, 17] {
            let idx = grid.index(0, 0, k).unwrap();
            grid.cells[idx] = Cell::Solid;
        }
        let run = |cs: f64| {
            let mut l = Lattice::new(&grid, 0.55, [1e-5, 0.0, 0.0], [true, true, false]).unwrap().with_smagorinsky(cs).unwrap();
            for _ in 0..30_000 {
                l.step();
            }
            (1..17).map(|k| l.moments(0, 0, k).unwrap().1[0]).sum::<f64>()
        };
        let (laminar, plain, turbulent) = (
            run(0.0),
            {
                let mut l = Lattice::new(&grid, 0.55, [1e-5, 0.0, 0.0], [true, true, false]).unwrap();
                for _ in 0..30_000 {
                    l.step();
                }
                (1..17).map(|k| l.moments(0, 0, k).unwrap().1[0]).sum::<f64>()
            },
            run(0.17),
        );
        assert_eq!(laminar, plain, "constant 0 is plain BGK");
        assert!(turbulent < laminar && turbulent > 0.5 * laminar, "{turbulent} vs {laminar}");
        for cs in [-0.1, 0.9, f64::NAN] {
            assert!(Lattice::new(&grid, 0.8, [0.0; 3], [true; 3]).unwrap().with_smagorinsky(cs).is_err(), "{cs}");
        }
    }

    #[test]
    fn unstable_relaxation_is_refused() {
        let bounds = Aabb3 { min: Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: Vec3 { x: 2.0, y: 2.0, z: 2.0 } };
        let grid = Grid::around(bounds, 1.0, 0, 1_000).unwrap();
        for tau in [0.5, 0.1, f64::NAN] {
            assert!(Lattice::new(&grid, tau, [0.0; 3], [true; 3]).is_err(), "{tau}");
        }
        assert!(Lattice::new(&grid, 0.8, [f64::INFINITY, 0.0, 0.0], [true; 3]).is_err());
    }
}
