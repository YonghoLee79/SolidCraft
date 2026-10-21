# Macros

A **JSON macro** is a command script the app records while you work. A **Rust macro** is a
small program that calls the same commands, with loops and `if`.

## JSON (record / play)

The toolbar's **MACRO** tab has Record, Pause, Resume and Stop. Pause keeps the recording and
skips the commands you run until Resume. A chip at the top of the window stays up for the whole
recording.

Save writes a `.macro.json` file, or a Cargo crate when the path ends in `.rs` (`Cargo.toml` +
`src/main.rs`). Open loads JSON. Play runs the loaded macro, or a compiled Rust binary against
this window. A macro can ask for values first; Play shows one row each. A step that
fails rolls the whole playback back, and a successful playback is one undo step.

The file is the [script format](scripts.md), plus a name and parameters. A step reads a
parameter as `{{name}}`. An earlier step's result is kept with `"let"`:

```json
{"command": "inspect.find_holes", "params": {"min_diameter": "{{min_dia}}"}, "let": "holes"}
{"command": "solid.chamfer", "params": {"edges": "{{holes.edges}}", "distance": "1 mm"}}
```

`inspect.find_holes` finds cylindrical holes (extrude cuts included). `max_diameter` / `min_diameter`
filter by size. `type` and `feature` are optional. Empty `edges` on Chamfer is a no-op.

JSON examples: [examples/macros](../examples/macros). A **start part** is a command script that
builds the design the macro expects. Load it, optionally edit, then Play:

```sh
solidcraft-cli run examples/macros/chamfer-holes.start.json --save holes.solidcraft
solidcraft holes.solidcraft
```

| Macro | Start part | Notes |
|---|---|---|
| `chamfer-large-holes` | `chamfer-holes.start.json` | Plate with Ø4, Ø8, Ø12; chamfers holes ≥ 5 mm |
| `chamfer-small-holes` | `chamfer-holes.start.json` | Same plate; chamfers holes ≤ 8 mm |
| `flatten-export-dxf` | `flatten-export-dxf.start.json` | Sheet L-bracket |
| `chamfer-selection` | `chamfer-holes.start.json` | Select edges, then Play the binary |
| `batch-mass` | `batch-mass/*.start.json` | `run --out dir/part.step`, then the binary on `dir` |
| `plate-generator`, `bolt-circle` | (empty design) | They create the part |

## Rust (custom macros)

`crates/macro-client` talks JSON-lines to a live app, or MCP to `solidcraft-cli mcp`. It does
not link the kernel.

```rust
use serde_json::json;
use solidcraft_macro_client::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::connect()?; // or App::headless()
    let holes = app.run("inspect.find_holes", json!({"min_diameter": "5 mm"}))?;
    for h in holes["holes"].as_array().unwrap_or(&vec![]) {
        let _ = app.run("solid.chamfer", json!({"edges": h["edges"], "distance": "1 mm"}));
    }
    Ok(())
}
```

Example binaries (`cargo run -p solidcraft-macros --bin chamfer-large-holes`):

| Binary | What it does |
|---|---|
| `chamfer-large-holes` | Chamfer both ends of holes wider than 5 mm (`chamfer-holes.start.json`) |
| `plate-generator` | Plate with four corner holes from argv |
| `flatten-export-dxf` | Flat pattern + DXF |
| `bolt-circle` | N holes on a pitch circle |
| `chamfer-selection` | Chamfer edges selected in the live app |
| `batch-mass` | Folder of STEP → CSV of volume, mass, area |

The desktop app starts a loopback control port; Play a binary from MACRO, or set
`SOLIDCRAFT_CONTROL_PORT`. Headless needs `solidcraft-cli` on `PATH` (or `SOLIDCRAFT_CLI`).
The browser keeps JSON only.

Desktop may spawn a user binary for a Rust macro; the CAD binary does not load plugins.
