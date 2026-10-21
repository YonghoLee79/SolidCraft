# Manufacturing (2.5D CAM core)

An opt-in core for planning 2.5D (three-axis) milling from a design's sketches and posting
G-code. There is no ribbon tab or panel yet: everything is a command, so scripts, the CLI and
the control channel can drive it, and a UI can follow.

The job lives in the design (`manufacture`), so it saves, reopens and undoes with it. A design
without CAM saves exactly as before. The planning is the `solidcraft-manufacture` crate (L1,
geometry only); the `manufacture.*` commands resolve sketches into it.

Units are millimetres, Z is the tool axis (up is positive), feeds are mm/min.

## Job

- **Tool** (`manufacture.tool.set`): number, kind (`flat_end_mill`, `ball_end_mill`,
  `bull_nose_end_mill`, `drill`), diameter, flutes, corner radius, tip angle.
- **Setup** (`manufacture.setup.set`): a work frame (`plane` XY/XZ/YZ and `origin`, or a full
  `frame`), work offset G54-G59, optional stock box in work coordinates, clearance height. Sketch
  planes must be parallel to the setup's XY; tilted ones are refused.
- **Operation** (`manufacture.operation.set`): a setup, a tool, a kind, `top`/`bottom` (work Z),
  retract, stepdown, spindle speed, feed, plunge feed, coolant.

| Kind | Cuts |
|---|---|
| `face` | zig-zag over the stock top, with margin and stepover |
| `contour` | around the chosen sketch profiles: `outside`, `inside` or `on`, climb or conventional, with allowance and ramping |
| `pocket` | concentric passes inside the profiles, islands respected, optional wall finish pass |
| `drill` | at circle centres of a sketch, or at given points; optional peck and dwell |

`sketch` can be an id or a name. Deleting a tool or setup that an operation uses is refused.

## Output

`manufacture.toolpath {operation}` returns move count, length, time estimate, bounds and
warnings (`moves: true` lists the moves). `manufacture.post {dialect}` returns the program, or
writes it to `path` (an existing file is kept unless `overwrite: true`).

Dialects: `grbl` (G21 G90, M0 pause at tool changes, drilling expanded to moves) and
`fanuc` / `haas` (O-number, `T M6`, `G43`, G81/G82/G83 canned cycles, controller-specific end
of program).

## Simulation and collision checking

`manufacture.simulate` cuts the setup's stock away along the operations' toolpaths and checks the
tool against the model. The stock must be set on the setup. It runs one setup at a time, from
the setup's original stock; operations in other setups do not affect it.

- The stock is a height map (a vertical tool only removes material from the top down) and the
  model is a column of solid intervals per cell, found from its triangles. `resolution` is the
  cell size (default 0.5 mm; coarser cells are used when the stock would need more than 4
  million).
- Collisions: **gouge** (the flutes cut into the model), **shank** (the shank above
  `flute_length` meets stock still standing, or the model), **rapid** (a rapid move runs into
  stock or the model) and **floor** (the tip goes below the stock). Each is listed once per
  move with the first position and the deepest overlap.
- Overlaps under `tolerance` (default one cell, at least 0.05 mm) are not reported, and the
  tool outline is cut to the grid, so a cut that rides a wall or a sloped face by less than
  about a cell is not a gouge.
- The reply has the volumes (stock, removed, remaining, part, left to cut, cut from the part),
  the counts per kind, the collisions, and with `surface: <cells>` the stock top for drawing.
  `until: <move>` stops after that move, for stepping through a program.
- `part: false` ignores the model (stock removal only); `bodies` limits which bodies count.

Tools take optional `flute_length` and `shank_diameter`; without a flute length, nothing above
the tip is checked as a shank.

In the app, `ui.start` of `manufacture.simulate` opens the Machining Simulation window: the
stock is drawn over the viewport with its camera, with the path so far, a slider and Play to
step through the program, and the collisions marked with red crosses.

## Limits

Arcs are emitted as short G1 segments (chord error 0.005 mm by default, `tolerance`). No 4/5-axis
or 3D surfacing. The simulation does not model undercuts, tool holders or fixtures, and checks
the tool only against the stock and the model. Sizes and work are capped, and bad numbers are
errors, never panics.
