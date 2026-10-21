# Macros

A **macro** is a [Rhai](https://rhai.rs) script the engine runs. Record captures the commands you
use; Save writes `.rhai`. You can add `if` and loops. Play is one undo step; a failure rolls back.

Command **scripts** (`solidcraft-cli run file.json`) stay JSON. That is not the MACRO tab.

## Record / play

The toolbar's **MACRO** tab has Record, Pause, Resume and Stop. Pause keeps the recording and
skips the commands you run until Resume. A chip at the top of the window stays up for the whole
recording.

Save writes a `.rhai` file. Open loads one. Play runs it. A macro can ask for values first
(`// @param name Label | default`); Play shows one row each.

```rhai
// @param min_dia Minimum diameter | 5 mm
let holes = run("inspect.find_holes", #{ min_diameter: min_dia });
run("solid.chamfer", #{ edges: holes.edges, distance: "1 mm" });
```

`run(command, params)` is `engine.execute`. `list_dir(path)` and `write_file(path, text)` are for
batch files (not in the browser). `file_stem(path)` is the name without extension.

`inspect.find_holes` finds cylindrical holes. `max_diameter` / `min_diameter` filter by size.
Empty `edges` on Chamfer is a no-op.

## Examples

Examples live in [examples/macros](../examples/macros). A **start part** is a JSON command script
that builds the design the macro expects:

```sh
solidcraft-cli run examples/macros/<start> --save part.solidcraft
solidcraft part.solidcraft
```

Then Open the `.rhai` file, optionally edit, then Play.

| Example | Start | What it shows |
|---|---|---|
| `plate-generator.rhai` | empty | Parameterized create |
| `chamfer-holes.rhai` | `chamfer-holes.start.json` | Query holes, chamfer by size |
| `flatten-export-dxf.rhai` | `flatten-export-dxf.start.json` | Sheet flatten + DXF |
| `chamfer-selection.rhai` | `chamfer-holes.start.json` | Current selection |
| `bolt-circle.rhai` | `flange-blank.start.json` | One hole + circular pattern |
| `batch-mass.rhai` | `batch-mass/*.start.json` | Folder of STEP → mass CSV |
| `batch-export.rhai` | same | Folder of STEP → STEP/STL |
| `chamfer-large-holes.rhai` | `enclosure.start.json` | Holes ≥ 2 mm |
| `mounting-plate.rhai` | empty | 8 mm plate, M8 holes, 12 mm inset, even ~50 mm pitch |

Try the enclosure:

```sh
solidcraft-cli run examples/macros/enclosure.start.json --save enclosure.solidcraft
solidcraft enclosure.solidcraft
```

Then Play `chamfer-large-holes.rhai`. The two Ø2.5 mm screw bosses pick up a 1 mm chamfer.

Mounting plate: Play asks for length and width. Thickness 8 mm, edge offset 12 mm, M8 clearance,
target pitch 50 mm. Gaps shorter than 40 mm drop a hole so spacing stays even.

The browser plays Rhai in the same engine. `list_dir` / `write_file` are desktop only.
