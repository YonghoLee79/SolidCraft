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
{"command": "inspect.find_holes", "params": {"min_diameter": "{{min_dia}}", "max_diameter": "{{max_dia}}"}, "let": "holes"}
{"command": "solid.chamfer", "params": {"edges": "{{holes.edges}}", "distance": "1 mm"}}
```

`inspect.find_holes` finds cylindrical holes (extrude cuts included). `max_diameter` / `min_diameter`
filter by size. `type` and `feature` are optional. Empty `edges` on Chamfer is a no-op.

## Examples

JSON and Rust examples live in [examples/macros](../examples/macros). A **start part** is a
command script that builds the design the macro expects. Load it, optionally edit, then Play:

```sh
solidcraft-cli run examples/macros/<start> --save part.solidcraft
solidcraft part.solidcraft
```

| Example | Kind | Start | What it shows |
|---|---|---|---|
| `plate-generator` | JSON | empty | Parameterized create |
| `chamfer-holes` | JSON | `chamfer-holes.start.json` | Query holes, chamfer by size (`min_dia` / `max_dia`) |
| `flatten-export-dxf` | JSON | `flatten-export-dxf.start.json` | Sheet flatten + DXF |
| `chamfer-selection` | Rust | `chamfer-holes.start.json` | Run on the current selection |
| `bolt-circle` | Rust | `flange-blank.start.json` | One hole + `solid.pattern.circular` on the open part |
| `batch-mass` | Rust | `batch-mass/*.start.json` | Folder of STEP → mass CSV |
| `batch-export` | Rust | same | Folder of STEP → STEP/STL (`file.export`) |
| `chamfer-large-holes` | Rust | `enclosure.start.json` | Complex part: find holes ≥ 2 mm and chamfer |

Try the enclosure: load the start, Open it, then Play the `chamfer-large-holes` binary. The two
Ø2.5 mm screw bosses pick up a 1 mm chamfer.

```sh
solidcraft-cli run examples/macros/enclosure.start.json --save enclosure.solidcraft
solidcraft enclosure.solidcraft
```

Rust binaries: `cargo run -p solidcraft-macros --bin chamfer-large-holes` (and the other names
in the table). `batch-mass` / `batch-export` first need STEP files:
`solidcraft-cli run examples/macros/batch-mass/cube.start.json --out dir/cube.step`.

## Rust (custom macros)

`crates/macro-client` talks JSON-lines to a live app, or MCP to `solidcraft-cli mcp`. It does
not link the kernel.

```rust
use serde_json::json;
use solidcraft_macro_client::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::connect()?; // or App::headless()
    let holes = app.run("inspect.find_holes", json!({"min_diameter": "2 mm"}))?;
    for h in holes["holes"].as_array().unwrap_or(&vec![]) {
        let _ = app.run("solid.chamfer", json!({"edges": h["edges"], "distance": "1 mm"}));
    }
    Ok(())
}
```

The desktop app starts a loopback control port; Play a binary from MACRO, or set
`SOLIDCRAFT_CONTROL_PORT`. Headless needs `solidcraft-cli` on `PATH` (or `SOLIDCRAFT_CLI`).
The browser keeps JSON only.

Desktop may spawn a user binary for a Rust macro; the CAD binary does not load plugins.
