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

## Limits

Arcs are emitted as short G1 segments (chord error 0.005 mm by default, `tolerance`). No 4/5-axis,
3D surfacing, tool collision checks against the part, or simulation. Sizes and work are capped,
and bad numbers are errors, never panics.
