// The Airflow solver's lattice Boltzmann step on the GPU: the f32 twin of
// solidcraft_sim::Lattice (D3Q19, BGK with the Smagorinsky model, Guo forcing, half-way
// bounce-back, openings by non-equilibrium extrapolation). See flow.rs.
//
// Populations are stored as their offset from the rest state (f - w), direction by direction:
// direction q of cell c is at q * cells + c. The offsets keep a small flow's digits in f32.

struct Params {
    nx: u32,
    ny: u32,
    nz: u32,
    cells: u32,
    // Which axes wrap around (1) or end in walls (0).
    px: u32,
    py: u32,
    pz: u32,
    openings: u32,
    // Body force per unit volume.
    gx: f32,
    gy: f32,
    gz: f32,
    tau: f32,
    // The Smagorinsky constant (0: laminar).
    cs: f32,
    // Invocations per dispatch row: an index is gid.x + gid.y * row.
    row: u32,
    pad0: u32,
    pad1: u32,
}

// An opening cell: kind 0 imposes the velocity value.xyz, kind 1 the density offset value.x
// (rho - 1).
struct Opening {
    cell: u32,
    inside: u32,
    kind: u32,
    pad: u32,
    value: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> solid: array<u32>;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;
@group(0) @binding(4) var<storage, read> openings: array<Opening>;

const NONE: u32 = 0xffffffffu;

// Directions: rest, 6 faces, 12 edges, in solidcraft_sim::lbm::C's order.
var<private> CX: array<i32, 19> = array<i32, 19>(0, 1, -1, 0, 0, 0, 0, 1, -1, 1, -1, 1, -1, 1, -1, 0, 0, 0, 0);
var<private> CY: array<i32, 19> = array<i32, 19>(0, 0, 0, 1, -1, 0, 0, 1, -1, -1, 1, 0, 0, 0, 0, 1, -1, 1, -1);
var<private> CZ: array<i32, 19> = array<i32, 19>(0, 0, 0, 0, 0, 1, -1, 0, 0, 0, 0, 1, -1, -1, 1, 1, -1, -1, 1);
var<private> OPP: array<u32, 19> = array<u32, 19>(0u, 2u, 1u, 4u, 3u, 6u, 5u, 8u, 7u, 10u, 9u, 12u, 11u, 14u, 13u, 16u, 15u, 18u, 17u);
var<private> W: array<f32, 19> = array<f32, 19>(
    1.0 / 3.0,
    1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0,
    1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0,
    1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0,
);

fn dir(q: u32) -> vec3<f32> {
    return vec3<f32>(f32(CX[q]), f32(CY[q]), f32(CZ[q]));
}

// The equilibrium's offset from rest for density 1 + drho and velocity u.
fn feq(q: u32, drho: f32, u: vec3<f32>) -> f32 {
    let cu = 3.0 * dot(dir(q), u);
    let uu = 1.5 * dot(u, u);
    return W[q] * (drho + (1.0 + drho) * (cu + 0.5 * cu * cu - uu));
}

// One coordinate moved by c: wrapped on a periodic axis; -1 past a wall face.
fn along(v: i32, c: i32, n: u32, periodic: u32) -> i32 {
    let p = v + c;
    let m = i32(n);
    if (p >= 0 && p < m) {
        return p;
    }
    if (periodic == 0u) {
        return -1;
    }
    return (p + m) % m;
}

// The cell along direction q from (i, j, k), or NONE past a wall face.
fn neighbour(i: i32, j: i32, k: i32, q: u32) -> u32 {
    let x = along(i, CX[q], params.nx, params.px);
    let y = along(j, CY[q], params.ny, params.py);
    let z = along(k, CZ[q], params.nz, params.pz);
    if (x < 0 || y < 0 || z < 0) {
        return NONE;
    }
    return u32(x) + params.nx * (u32(y) + params.ny * u32(z));
}

// Collide every fluid cell, then stream to its neighbours; a population heading into a wall
// comes back the opposite way. Every slot of a fluid cell is written by exactly one invocation.
@compute @workgroup_size(64)
fn collide_stream(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cell = gid.x + gid.y * params.row;
    if (cell >= params.cells) {
        return;
    }
    if (solid[cell] != 0u) {
        return;
    }
    let n = params.cells;
    var f: array<f32, 19>;
    var drho = 0.0;
    var m = vec3<f32>(0.0);
    for (var q = 0u; q < 19u; q++) {
        let v = src[q * n + cell];
        f[q] = v;
        drho += v;
        m += v * dir(q);
    }
    let g = vec3<f32>(params.gx, params.gy, params.gz);
    let u = (m + 0.5 * g) / (1.0 + drho);
    var eq: array<f32, 19>;
    for (var q = 0u; q < 19u; q++) {
        eq[q] = feq(q, drho, u);
    }
    var tau = params.tau;
    if (params.cs > 0.0) {
        // The non-equilibrium momentum flux, a symmetric tensor.
        var d = vec3<f32>(0.0);
        var o = vec3<f32>(0.0);
        for (var q = 0u; q < 19u; q++) {
            let c = dir(q);
            let neq = f[q] - eq[q];
            d += c * c * neq;
            o += vec3<f32>(c.x * c.y, c.x * c.z, c.y * c.z) * neq;
        }
        let norm = sqrt(dot(d, d) + 2.0 * dot(o, o));
        tau = 0.5 * (tau + sqrt(tau * tau + 18.0 * sqrt(2.0) * params.cs * params.cs * norm / (1.0 + drho)));
    }
    let i = i32(cell % params.nx);
    let j = i32((cell / params.nx) % params.ny);
    let k = i32(cell / (params.nx * params.ny));
    let keep = 1.0 - 0.5 / tau;
    for (var q = 0u; q < 19u; q++) {
        let c = dir(q);
        let source = dot(3.0 * (c - u) + 9.0 * dot(c, u) * c, g);
        let post = f[q] - (f[q] - eq[q]) / tau + keep * W[q] * source;
        let to = neighbour(i, j, k, q);
        var slot = OPP[q] * n + cell;
        if (to != NONE) {
            if (solid[to] == 0u) {
                slot = q * n + to;
            }
        }
        dst[slot] = post;
    }
}

// Rewrite the opening cells after streaming: the imposed density or velocity, the other moment
// and the non-equilibrium part taken from the fluid cell just inside (never an opening itself).
@compute @workgroup_size(64)
fn apply_openings(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x + gid.y * params.row;
    if (index >= params.openings) {
        return;
    }
    let o = openings[index];
    let n = params.cells;
    var f: array<f32, 19>;
    var drho_in = 0.0;
    var m = vec3<f32>(0.0);
    for (var q = 0u; q < 19u; q++) {
        let v = dst[q * n + o.inside];
        f[q] = v;
        drho_in += v;
        m += v * dir(q);
    }
    let g = vec3<f32>(params.gx, params.gy, params.gz);
    let u_in = (m + 0.5 * g) / (1.0 + drho_in);
    var drho = drho_in;
    var u = o.value.xyz;
    if (o.kind == 1u) {
        drho = o.value.x;
        u = u_in;
    }
    for (var q = 0u; q < 19u; q++) {
        dst[q * n + o.cell] = feq(q, drho, u) + f[q] - feq(q, drho_in, u_in);
    }
}
