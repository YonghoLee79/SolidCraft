# Macros

A macro is a command script the app records while you work, and plays back. The toolbar's
**MACRO** tab has Record, Pause, Resume and Stop. Pause keeps the recording and skips the
commands you run until Resume. A chip at the top of the window stays up for the whole recording
and says whether it is capturing or paused.

Save writes a `.macro.json` file. Open loads one. Play runs the loaded macro. A macro can ask
for values first (length, a file path, a feature type); Play shows one row each, filled with
the default, and runs with what you type. A step that fails rolls the whole playback back, and
a successful playback is one undo step.

The file is the [script format](scripts.md), plus a name and parameters. A step reads a
parameter as `{{name}}` (inside a string — `"{{length}} - 10 mm"` is an expression — or as the
whole value, which keeps a list or a number as it is). An earlier step's result is kept with
`"let"` and read back as `{{name.field}}`:

```json
{"command": "inspect.find_holes", "params": {"max_diameter": "{{max_dia}}", "type": "Hole"}, "let": "holes"}
{"command": "solid.chamfer", "params": {"edges": "{{holes.edges}}", "distance": "1 mm"}}
```

`inspect.find_holes` finds cylindrical holes. `max_diameter` keeps only holes no wider than
that; `min_diameter` keeps only holes wider than that. `type` keeps holes made by a feature of
that type (`Hole`, `Extrude`, the feature's base name, or `solid.hole`). `feature` keeps holes
made by that feature's name or id. Each hole's `edges` are the round edges that close it, both
ends, which is what Chamfer and Fillet take.

Examples: [examples/macros](../examples/macros) — a plate from five values, chamfer every hole
under 8 mm on both sides, chamfer every hole larger than 5 mm, and flatten a sheet-metal part
to DXF.
