//! MCP tool definitions and dispatch. Every tool is a thin wrapper over engine commands
//! (`engine.execute`) and the inspection / rendering control methods.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};
use solidcraft_engine::io::vfs;
use solidcraft_engine::timing;

use crate::backend::Backend;

/// Most commands in one `batch` call.
const MAX_BATCH: usize = 10_000;
/// Largest image returned by `screenshot`.
const MAX_IMAGE_BYTES: usize = 32 << 20;

const VIEWS: &[&str] = &["iso", "home", "front", "back", "top", "bottom", "left", "right", "fit"];

fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

/// What a tool may do, for the client's approval prompts (MCP tool annotations). Clients that
/// auto-approve on these hints must not be told less than the truth (#205).
#[derive(Clone, Copy)]
enum Effect {
    /// Only looks.
    ReadOnly,
    /// Changes the design in memory, undoably.
    Edits,
    /// Replaces the design in memory (unsaved changes are lost).
    Replaces,
    /// May read, write, overwrite or delete files on disk (any command, a file path).
    Files,
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, effect: Effect) -> Value {
    let (read_only, destructive, open_world) = match effect {
        Effect::ReadOnly => (true, false, false),
        Effect::Edits => (false, false, false),
        Effect::Replaces => (false, true, false),
        Effect::Files => (false, true, true),
    };
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": read_only, "openWorldHint": open_world},
    })
}

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

/// Every tool, with its JSON schema. The same list serves the headless and the bridged backend.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "list_commands",
            "List commands",
            "The command catalog: id, label, toolbar tab/panel, shortcut, one-line parameter docs and whether the command is \
             enabled right now. Ids are dotted and lower-case (solid.extrude, sketch.create, \
             solid.fillet, sketch.inspect, timeline.rollback). Optional case-insensitive \
             substring `filter` over id, label, tab and panel (not the parameter docs, so a command id \
             finds that command rather than every command whose docs mention it).",
            obj(json!({"filter": string("Substring to match, e.g. \"sketch\", \"fillet\", \"pattern\"")}), &[]),
            Effect::ReadOnly,
        ),
        tool(
            "describe_command",
            "Describe a command",
            "One command's parameter docs and, for commands with a typed contract (solid.box, solid.extrude and engine.describe so far), the \
             JSON Schemas (2020-12) of its parameters (`input`) and of a successful result (`output`); null for the others.",
            obj(json!({"id": string("Command id, e.g. \"solid.extrude\"")}), &["id"]),
            Effect::ReadOnly,
        ),
        tool(
            "execute",
            "Execute a command",
            "Run one command with JSON parameters, exactly as the toolbar, palette and scripts do. Never opens a dialog. \
             Examples: sketch.create {plane: \"XY\"}; sketch.rectangle.two_point {p0: [0,0], p1: [40,30]}; sketch.circle.center \
             {center: [20,15], radius: 5}; sketch.dimension {entities: [curve id], value: \"width\"}; sketch.finish {}; solid.extrude \
             {distance: 20, operation: \"cut\"}; solid.fillet {edges: [[0,0,10]], radius: 3}; solid.box {length, \
             width, height}. Lengths are mm; expressions may use parameters and units (\"width / 2\", \"30 deg\"). Returns \
             the command's result; failures leave the design unchanged. Sketch commands that add geometry return \
             created: {points, curves, constraints, dimensions}: the ids of what they added, stable and not reused \
             after a delete (undo rewinds the counters with the design), usable right away \
             in sketch.dimension, sketch.constraint.* and sketch.delete. Feature commands return the feature's id \
             and name, every body (bodies) and the bodies the feature created, changed and removed (created_bodies, changed_bodies, removed_bodies).",
            obj(
                json!({
                    "command": string("Command id (see list_commands)"),
                    "params": {"type": "object", "description": "The command's parameters"},
                    "verbose": {"type": "boolean", "description": "false: omit the full feature body list and sketch ids repeated outside `created`; keep created_bodies, changed_bodies and removed_bodies (default true)"},
                }),
                &["command"],
            ),
            Effect::Files,
        ),
        tool(
            "batch",
            "Run several commands",
            "Run commands in order, stopping at the first error. Each step is {command, params}. On an error the steps \
             already run are undone (unless rollback: false), so the batch is all-or-nothing; the reply names the failing \
             step (index from 0), its error, and every result before it. The usual way to build a part: sketch, draw, \
             dimension, finish, extrude, fillet… in one call.",
            obj(
                json!({
                    "commands": {
                        "type": "array",
                        "description": "Steps: [{\"command\": id, \"params\": {...}}, ...]",
                        "items": {"type": "object", "properties": {"command": {"type": "string"}, "params": {"type": "object"}}, "required": ["command"]},
                    },
                    "rollback": {"type": "boolean", "description": "Undo completed steps when one fails (default true)"},
                    "timings": {"type": "boolean", "description": "Add `timings`: [{index, command, ms}] for every step run (the failing one included) and `total_ms` (default false)"},
                    "profile": {"type": "boolean", "description": "Like timings, and each step's `profile`: {boolean_ms, tessellate_ms, other_ms}, its time in kernel booleans, in tessellation and the rest (default false)"},
                    "verbose": {"type": "boolean", "description": "false: a compact reply. Feature steps leave out the full body list (created_bodies, changed_bodies and removed_bodies stay), sketch steps the ids they repeat outside `created` (default true)"},
                }),
                &["commands"],
            ),
            Effect::Files,
        ),
        tool(
            "inspect_design",
            "Inspect the design",
            "The whole design: user parameters (expression, value, errors), the timeline (features with type, suppressed, \
             rolled back, error, warning, timing), bodies (volume, area, centre of mass, bounding box, face/edge/vertex \
             counts), sketches (curves with ids and coordinates, points, constraints, profiles, degrees of freedom, solve \
             status), assembly joints (what each origin is attached to: occurrence, the feature, face, edge or sketch \
             curve its face names refer to, whether it resolves and why not; `execute` `joint.inspect {joint}` gives one), \
             the active sketch, selection and undo depth.",
            obj(
                json!({
                    "measure": {"type": "boolean", "description": "Measure bodies (default true)"},
                    "sketches": {"type": "boolean", "description": "Full sketch contents (default true; false = summary only)"},
                    "joints": {"type": "boolean", "description": "Joint definitions as joint.inspect gives them, when the design has joints (default true)"},
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "measure",
            "Measure bodies",
            "Volume (mm³), surface area (mm²), centre of mass, bounding box and face/edge/vertex counts of one body or all \
             bodies, with totals. With `items` (one or two selections, as `execute` takes them: faces, edges, vertices, \
             sketch curves, profiles): one item's length, area or position, a circle's, arc's or cylinder's `diameter` and \
             `radius` (with `circumference`, or an arc's `arc_length` and `angle_deg`), a slot's `slot` sizes; two items' \
             `min` and `max` distance and, where one is round, `center_distance` (centre to centre, or from its axis).",
            obj(
                json!({
                    "body": string("Body name (default: all bodies)"),
                    "items": {"type": "array", "items": {"type": "object"}, "description": "One or two selections to measure instead of bodies, e.g. {\"type\": \"face\", \"body\": \"Body1\", \"index\": 3, \"point\": [x,y,z]}"},
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "body_topology",
            "List edges and faces",
            "The edges (index, midpoint, length, endpoints) and faces (index, name, area, centroid, plane normal, a `point` that lies \
             on the face, and for a cylindrical face its `cylinder`: origin, axis, radius) of a body. Use an edge midpoint as the \
             [x,y,z] point that picks the edge for fillet and chamfer; a face's point or its name picks the face for the face commands.",
            obj(json!({"body": string("Body name, e.g. \"Body1\"")}), &["body"]),
            Effect::ReadOnly,
        ),
        tool(
            "fastener_metric_hole",
            "Fastener Metric Hole (DIN 912)",
            "Cut a metric clearance hole, optionally with a counterbore for a DIN 912 socket head cap screw. The drill \
             diameter, counterbore diameter and counterbore depth come from the standard size. `position` is a point on \
             the face to drill; the hole goes into the body that face belongs to.",
            obj(
                json!({
                    "position": {"type": "array", "items": {"type": "number"}, "description": "[x, y, z] point on the face to drill"},
                    "direction": {"type": "array", "items": {"type": "number"}, "description": "Drill direction (default [0, 0, -1])"},
                    "standard": {"type": "string", "enum": ["M2", "M3", "M4", "M5", "M6"], "description": "Metric screw size"},
                    "depth": {"type": "number", "description": "Hole depth (mm) from the face, the counterbore included"},
                    "counterbore": {"type": "boolean", "description": "Counterbore for a DIN 912 screw head (default false)"},
                    "body_name": string("Body expected to be drilled; checked to exist"),
                }),
                &["position", "standard", "depth"],
            ),
            Effect::Edits,
        ),
        tool(
            "fastener_hole_pattern",
            "Fastener Hole Pattern (bolt circle or grid)",
            "Cut DIN 912 fastener holes (as fastener_metric_hole) on a bolt circle or a rectangular grid in one call: one \
             hole, then a circular or rectangular pattern of it, so the timeline stays editable. Circular: `center` on the \
             face, `radius`, `count`, `start_angle_deg?`, `axis?` (the face's outward normal, default [0, 0, 1]; holes drill \
             along −axis). Rectangular: `origin` (the first hole), `count_x`, `spacing_x`, `count_y?`, `spacing_y?`, \
             `dir_x?` [1,0,0], `dir_y?` [0,1,0], `direction?` (drill, default [0, 0, -1]). Nothing is left behind on failure.",
            obj(
                json!({
                    "pattern": {"type": "string", "enum": ["circular", "rectangular"], "description": "Bolt circle or grid"},
                    "standard": {"type": "string", "enum": ["M2", "M3", "M4", "M5", "M6"], "description": "Metric screw size"},
                    "depth": {"type": "number", "description": "Hole depth (mm) from the face, the counterbore included"},
                    "counterbore": {"type": "boolean", "description": "Counterbore for a DIN 912 screw head (default false)"},
                    "center": {"type": "array", "items": {"type": "number"}, "description": "Circular: bolt circle centre [x, y, z] on the face"},
                    "radius": {"type": "number", "description": "Circular: bolt circle radius (mm)"},
                    "count": {"type": "integer", "description": "Circular: number of holes (2 to 360)"},
                    "start_angle_deg": {"type": "number", "description": "Circular: angle of the first hole (default 0)"},
                    "axis": {"type": "array", "items": {"type": "number"}, "description": "Circular: the face's outward normal (default [0, 0, 1])"},
                    "origin": {"type": "array", "items": {"type": "number"}, "description": "Rectangular: first hole [x, y, z] on the face"},
                    "count_x": {"type": "integer", "description": "Rectangular: holes along dir_x (1 to 100)"},
                    "spacing_x": {"type": "number", "description": "Rectangular: spacing along dir_x (mm)"},
                    "count_y": {"type": "integer", "description": "Rectangular: holes along dir_y (default 1, at most 100)"},
                    "spacing_y": {"type": "number", "description": "Rectangular: spacing along dir_y (mm)"},
                    "dir_x": {"type": "array", "items": {"type": "number"}, "description": "Rectangular: first direction (default [1, 0, 0])"},
                    "dir_y": {"type": "array", "items": {"type": "number"}, "description": "Rectangular: second direction (default [0, 1, 0])"},
                    "direction": {"type": "array", "items": {"type": "number"}, "description": "Rectangular: drill direction (default [0, 0, -1])"},
                }),
                &["pattern", "standard", "depth"],
            ),
            Effect::Edits,
        ),
        tool(
            "fdm_print_estimate",
            "FDM Print Estimate (filament and build volume)",
            "Estimate the plastic an FDM print of bodies takes (a shell of `wall_mm` plus `infill_percent` of the inside; \
             supports, brims and top/bottom layers are not counted, so a slicer's figure differs) as volume, grams and \
             metres of filament, and check that each body fits the printer's build volume as placed or turned by 90°.",
            obj(
                json!({
                    "bodies": {"type": "array", "items": {"type": "string"}, "description": "Body names (default: all bodies)"},
                    "material": {"type": "string", "enum": ["PLA", "PETG", "ABS", "ASA", "TPU", "PA"], "description": "Filament (default PLA)"},
                    "density_g_cm3": {"type": "number", "description": "Filament density instead of `material`"},
                    "infill_percent": {"type": "number", "description": "Infill density, 0 to 100 (default 15)"},
                    "wall_mm": {"type": "number", "description": "Shell thickness (default 1.2: three 0.4 mm perimeters)"},
                    "filament_diameter_mm": {"type": "number", "description": "Filament diameter (default 1.75)"},
                    "build_volume_mm": {"type": "array", "items": {"type": "number"}, "description": "Printer build volume [x, y, z] (default [220, 220, 250])"},
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "dfm_clearance_check",
            "DFM Clearance Check for 3D Print / CNC",
            "Measure the smallest gap between two bodies, compare it with a safe clearance (default 0.25 mm for FDM), \
             report a collision and say how much to widen the gap.",
            obj(
                json!({
                    "body_a": string("First body name"),
                    "body_b": string("Second body name"),
                    "min_clearance_mm": {"type": "number", "description": "Smallest safe gap (mm, default 0.25 for FDM 3D printing)"},
                }),
                &["body_a", "body_b"],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "set_parameter",
            "Set a parameter",
            "Create or change a user parameter and re-evaluate the timeline. `value` is a number (mm) or an expression \
             (\"40 mm\", \"width * 2\", \"30 deg\"); `expression` is the same thing under the name the parameters.change command \
             uses. Give one of the two (both only when they agree). Returns the new value, the features recomputed (`recomputed`), the \
             features whose earlier result for the same inputs was reused instead (`restored`, e.g. after setting a value back) and any \
             features that now fail.",
            obj(
                json!({
                    "name": string("Parameter name"),
                    "value": {"type": ["number", "string"], "description": "Number or expression (or give it as `expression`)"},
                    "expression": {"type": ["number", "string"], "description": "Same as `value` (the parameters.change name); give one of the two"},
                    "unit": string("mm, deg or \"\" (unitless); default from the expression"),
                    "comment": string("Comment shown in the parameters table"),
                }),
                &["name"],
            ),
            Effect::Edits,
        ),
        tool(
            "screenshot",
            "Look at the design",
            "Render the design and return it as a PNG image. Headless: a CPU render of the model, fitted to the view. \
             Connected to the app: a screenshot of the window (source \"window\", the default) or a render of the model \
             with the app's camera (source \"model\"). Both show an active section (inspect.section) with its cut face \
             capped. `view` first turns the camera to a standard view or a saved named view (view.save), `camera` to              any camera (as view.apply). \
             `grid: false` and `axes: false` leave the ground grid (on Z 0) and the origin's axes out of a model render. \
             A body whose appearance is see-through (appearance.assign with an `opacity` below 1) is drawn see-through, \
             so what is inside it shows: that is how to look at the body a cut would remove inside the material it meets.",
            obj(
                json!({
                    "view": string(
                        "Standard view (iso, home, front, back, top, bottom, left, right, fit) or the name of a saved named view                          (default: keep the current one; headless starts at iso)"
                    ),
                    "camera": {"type": "object", "description": "Look from this camera instead: {target: [x,y,z], yaw, pitch, distance, fov?}                         (radians, mm; as view.save and view.apply)"},
                    "path": string("Also keep the PNG at this path"),
                    "width": {"type": "integer", "description": "Render width in pixels (model renders; default 960)"},
                    "height": {"type": "integer", "description": "Render height in pixels (model renders; default 600)"},
                    "source": {"type": "string", "enum": ["window", "model"], "description": "window = app screenshot, model = model render"},
                    "grid": {"type": "boolean", "description": "false: leave the ground grid out of the render. It lies on Z 0 and cuts through a part that straddles it. Model renders only: default true, or as the app shows it when connected; with `grid` or `axes` a connected screenshot is a model render, and source \"window\" is an error"},
                    "axes": {"type": "boolean", "description": "false: leave the origin's three axes out of the render (see `grid`). Axes hidden with browser.visibility stay hidden"},
                }),
                &[],
            ),
            Effect::Files,
        ),
        tool(
            "export",
            "Export",
            "Export bodies to STEP (.step/.stp), 3MF (.3mf), binary STL (.stl), ASCII STL (format stla) or OBJ. The format \
             comes from the extension unless given. URDF (.urdf) exports the whole assembly for robot simulators: a link per \
             component placement with mass and inertia, joints from the assembly's joints, and STL meshes in metres written \
             next to it (docs/urdf.md); RViz needs mesh_prefix such as package://robot/meshes/.",
            obj(
                json!({
                    "path": string("Output file"),
                    "format": {"type": "string", "enum": ["step", "3mf", "3mf-tagged", "stl", "stla", "obj", "urdf"]},
                    "bodies": {"type": "array", "items": {"type": "string"}, "description": "Body names (default: all; not with urdf)"},
                    "mesh_prefix": string("urdf: put before each mesh file name (default: none, so relative to the .urdf)"),
                    "effort": {"type": "number", "description": "urdf: joint effort limit, N or N m (default 1000)"},
                    "velocity": {"type": "number", "description": "urdf: joint velocity limit, m/s or rad/s (default 10)"},
                }),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "crash_reports",
            "Crash reports",
            "SolidCraft's crash reports on this computer (the app's when connected to it). Without `id`: the list, newest \
             first (id, kind, time, message, command, count, version, path, replayable). With `id`: the whole report: \
             message, location, backtrace, app version and commit, system, the command that crashed and the recent ones, \
             and design_path / replay_path when it can be replayed: `solidcraft-cli crashes replay <id>`, or \
             `solidcraft-cli run <replay_path> --in <design_path>`. Reports never leave this computer.",
            obj(
                json!({
                    "id": string("A report id from the list"),
                    "limit": {"type": "integer", "description": "Reports to list (default 20)"},
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool("undo", "Undo", "Undo the last change to the design.", obj(json!({}), &[]), Effect::Edits),
        tool("redo", "Redo", "Redo the last undone change.", obj(json!({}), &[]), Effect::Edits),
        tool(
            "new_design",
            "New design",
            "Start a new, empty design (unsaved changes to the current one are discarded).",
            obj(json!({"name": string("Design name (default Untitled)")}), &[]),
            Effect::Replaces,
        ),
        tool(
            "open",
            "Open a design",
            "Open a .solidcraft design file, a STEP file (.step/.stp: a new design whose bodies come from the file, as an \
             Import base feature in a component named after the file; an assembly's parts become nested components) or a \
             mesh (.3mf/.stl/.obj/.glb/.gltf: mesh bodies in such a component; a glTF file's nodes become nested components and its \
             materials appearances); the result reports the component (id, component_name). Replaces the current design; insert into it \
             instead with the file.insert_step (STEP) or file.insert_mesh (mesh) commands. A STEP file from a system \
             that has Y up opens lying on its back: `up: \"y\"` turns its component a quarter turn about X (Y becomes Z). glTF is Y up, \
             so it is turned by default (`up: \"z\"` keeps its axes). `names` picks what the bodies are named after when the solids' own names say nothing useful.",
            obj(
                json!({
                    "path": string("Path to a .solidcraft, .step/.stp, .3mf, .stl, .obj, .glb or .gltf file"),
                    "up": {"type": "string", "enum": ["z", "y"], "description": "STEP and glTF only. z keeps the file's axes; y: the file has Y up, so its bodies are turned to stand Z up. Default: z for STEP, y for glTF"},
                    "names": {"type": "string", "enum": ["solid", "product", "instance"], "description": "STEP only. What the bodies are named after: solid (default: the solid's own name, else the product's), product (the part's name) or instance (the assembly occurrence's name, else the product's)"},
                }),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "save",
            "Save the design",
            "Save the design to `path` or its current file. The default is a design container; a .json path selects plain JSON. Set format to container or json to override the path.",
            obj(
                json!({"path": string("Path to save to (default: the current file)"), "format": {"type": "string", "enum": ["container", "json"], "description": "Design encoding (default: JSON for .json, container otherwise)"}}),
                &[],
            ),
            Effect::Files,
        ),
    ]
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

/// Validates `args` against the tool's schema (top level): unknown tool, missing required
/// arguments, unknown arguments, type and enum mismatches.
pub(crate) fn check_arguments(name: &str, args: &Value) -> Result<(), String> {
    let defs = tool_definitions();
    let def = defs.iter().find(|d| d["name"] == name).ok_or_else(|| format!("unknown tool `{name}`"))?;
    let schema = &def["inputSchema"];
    let Some(a) = args.as_object() else { return Err(format!("{name}: arguments must be an object")) };
    for r in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if a.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name}: missing required argument `{r}`"));
        }
    }
    let props = schema["properties"].as_object();
    for (k, v) in a {
        let Some(p) = props.and_then(|p| p.get(k)) else {
            return Err(format!("{name}: unknown argument `{k}`"));
        };
        if v.is_null() {
            continue;
        }
        let ok = match &p["type"] {
            Value::String(t) => type_matches(t, v),
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).any(|t| type_matches(t, v)),
            _ => true,
        };
        if !ok {
            return Err(format!("{name}: argument `{k}` must be of type {}", p["type"]));
        }
        if let Some(allowed) = p["enum"].as_array()
            && !allowed.contains(v)
        {
            return Err(format!("{name}: `{k}` must be one of {}", Value::Array(allowed.clone())));
        }
    }
    Ok(())
}

/// A tool result: content blocks plus the error flag.
fn text_result(v: &Value, is_error: bool) -> Value {
    let text = match v {
        Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    let mut r = json!({"content": [{"type": "text", "text": text}]});
    if is_error {
        r["isError"] = json!(true);
    }
    r
}

/// Run a tool. Failures are tool results with `isError: true` (the model sees the message);
/// nothing here panics on hostile arguments.
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> Value {
    let empty = Map::new();
    let a = args.as_object().unwrap_or(&empty);
    let r = match name {
        "screenshot" => return screenshot(b, a),
        "batch" => return batch(b, a),
        _ => dispatch(b, name, a),
    };
    match r {
        Ok(v) => text_result(&v, false),
        Err(e) => text_result(&Value::String(e), true),
    }
}

fn exec(b: &mut dyn Backend, command: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": command, "params": params}))
}

fn pick(a: &Map<String, Value>, keys: &[&str]) -> Value {
    Value::Object(keys.iter().filter_map(|k| a.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect())
}

fn str_arg<'a>(a: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn dispatch(b: &mut dyn Backend, name: &str, a: &Map<String, Value>) -> Result<Value, String> {
    match name {
        "list_commands" => {
            let all = b.call("engine.commands", json!({}))?;
            let Some(f) = str_arg(a, "filter").map(str::to_lowercase).filter(|f| !f.is_empty()) else { return Ok(all) };
            let hit = |c: &Value| {
                // Not "params": many docs name other commands (e.g. "usable in sketch.delete"), #269.
                ["id", "label", "panel", "tab"].iter().any(|k| c.get(*k).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&f)))
            };
            Ok(Value::Array(all.as_array().map(|v| v.iter().filter(|c| hit(c)).cloned().collect()).unwrap_or_default()))
        }
        "describe_command" => {
            let id = str_arg(a, "id").ok_or("`id` is required")?;
            exec(b, "engine.describe", json!({"command": id}))
        }
        "execute" => {
            let id = str_arg(a, "command").ok_or("`command` is required")?;
            let v = exec(b, id, a.get("params").cloned().filter(Value::is_object).unwrap_or(json!({})))?;
            Ok(if a.get("verbose").and_then(Value::as_bool).unwrap_or(true) { v } else { compact(v) })
        }
        "inspect_design" => {
            let measure = a.get("measure").and_then(Value::as_bool).unwrap_or(true);
            let mut d = b.call("document.inspect", json!({"measure": measure}))?;
            if a.get("sketches").and_then(Value::as_bool).unwrap_or(true) {
                let ids: Vec<Value> = d["sketches"].as_array().map(|v| v.iter().map(|s| s["id"].clone()).collect()).unwrap_or_default();
                let full: Vec<Value> = ids
                    .into_iter()
                    .map(|id| exec(b, "sketch.inspect", json!({"sketch": id.clone()})).unwrap_or_else(|e| json!({"id": id, "error": e})))
                    .collect();
                d["sketches"] = Value::Array(full);
            }
            // Joints: what each is attached to (left out when there are none, or the app is older).
            if a.get("joints").and_then(Value::as_bool).unwrap_or(true)
                && let Ok(j) = exec(b, "joint.inspect", json!({}))
                && j["joints"].as_array().is_some_and(|v| !v.is_empty())
            {
                d["joints"] = j["joints"].clone();
            }
            Ok(d)
        }
        "measure" => {
            if let Some(items) = a.get("items") {
                return exec(b, "inspect.measure", json!({ "items": items }));
            }
            let p = match str_arg(a, "body") {
                Some(body) => json!({"bodies": [body]}),
                None => json!({}),
            };
            exec(b, "inspect.measure", p)
        }
        "body_topology" => {
            let p = pick(a, &["body"]);
            let edges = exec(b, "model.edges", p.clone())?;
            let faces = exec(b, "model.faces", p)?;
            Ok(json!({"body": a.get("body"), "edges": edges["edges"], "faces": faces["faces"]}))
        }
        "set_parameter" => {
            let mut p = pick(a, &["name", "unit", "comment"]);
            p["expression"] = parameter_value(a)?;
            exec(b, "parameters.change", p)
        }
        "export" => exec(b, "file.export", pick(a, &["path", "format", "bodies", "mesh_prefix", "effort", "velocity"])),
        "crash_reports" => match a.get("id") {
            Some(_) => exec(b, "diagnostics.crash_report", pick(a, &["id"])),
            None => exec(b, "diagnostics.crash_reports", pick(a, &["limit"])),
        },
        "undo" => exec(b, "edit.undo", json!({})),
        "redo" => exec(b, "edit.redo", json!({})),
        "new_design" => exec(b, "file.new", pick(a, &["name"])),
        "open" => exec(b, "doc.open", pick(a, &["path", "up", "names"])),
        "save" => exec(b, "file.save", pick(a, &["path", "format"])),
        "fastener_metric_hole" => fastener_metric_hole(b, a),
        "dfm_clearance_check" => dfm_clearance_check(b, a),
        "fastener_hole_pattern" => fastener_hole_pattern(b, a),
        "fdm_print_estimate" => fdm_print_estimate(b, a),
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// `set_parameter`'s new value: `value` or `expression` (the parameters.change name), one of
/// them, or both when they agree (`50` and `"50"` do).
fn parameter_value(a: &Map<String, Value>) -> Result<Value, String> {
    let text = |v: &Value| match v {
        Value::String(s) => s.trim().to_string(),
        v => v.to_string(),
    };
    match (a.get("value").filter(|v| !v.is_null()), a.get("expression").filter(|v| !v.is_null())) {
        (Some(v), None) | (None, Some(v)) => Ok(v.clone()),
        (Some(v), Some(e)) if text(v) == text(e) => Ok(v.clone()),
        (Some(_), Some(_)) => Err("set_parameter: `value` and `expression` disagree; give one of them".into()),
        (None, None) => Err("set_parameter: give the new value as `value` (or `expression`): a number (mm) or an expression".into()),
    }
}

/// Drill, counterbore diameter and counterbore depth (mm) for a DIN 912 screw size.
fn din912(standard: &str) -> Option<(f64, f64, f64)> {
    Some(match standard {
        "M2" => (2.4, 3.8, 2.0),
        "M3" => (3.4, 6.0, 3.5),
        "M4" => (4.5, 7.5, 4.5),
        "M5" => (5.5, 9.0, 5.5),
        "M6" => (6.6, 10.5, 6.5),
        _ => return None,
    })
}

fn fastener_metric_hole(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<Value, String> {
    let standard = str_arg(a, "standard").ok_or("`standard` is required")?;
    let (drill, cb_dia, cb_depth) = din912(standard).ok_or_else(|| format!("unsupported standard `{standard}` (M2, M3, M4, M5 or M6)"))?;
    let depth = a.get("depth").and_then(Value::as_f64).filter(|d| d.is_finite() && *d > 0.0).ok_or("`depth` must be a positive number")?;
    let position = a.get("position").cloned().ok_or("`position` is required")?;
    let direction = a.get("direction").cloned().unwrap_or_else(|| json!([0.0, 0.0, -1.0]));
    let counterbore = a.get("counterbore").and_then(Value::as_bool).unwrap_or(false);
    // `solid.hole` drills the body whose face is at `position`; a named body is checked to exist.
    if let Some(body) = str_arg(a, "body_name") {
        exec(b, "inspect.measure", json!({"bodies": [body]}))?;
    }
    let mut p = json!({"position": position.clone(), "direction": direction, "diameter": drill, "depth": depth});
    if counterbore {
        p["type"] = json!("counterbore");
        p["cb_diameter"] = json!(cb_dia);
        p["cb_depth"] = json!(cb_depth);
    }
    let applied = exec(b, "solid.hole", p)?;
    // `solid.hole` accepts a point off every face and cuts nothing: that is no fastener hole.
    if applied.get("changed_bodies").and_then(Value::as_array).is_some_and(Vec::is_empty) {
        let undone = exec(b, "edit.undo", json!({})).is_ok();
        return Err(format!(
            "the hole at {position} cuts no body: `position` must be a point on a face{}",
            if undone { "" } else { " (an empty Hole feature was left: undo it)" }
        ));
    }
    Ok(json!({
        "status": "applied",
        "standard": standard,
        "drill_diameter_mm": drill,
        "drill_depth_mm": depth,
        "counterbore": counterbore,
        "counterbore_diameter_mm": counterbore.then_some(cb_dia),
        "counterbore_depth_mm": counterbore.then_some(cb_depth),
        "result": applied,
    }))
}

/// Largest coordinate or length (mm) the pattern tools take.
const MAX_MM: f64 = 1e6;

/// A finite `[x, y, z]` argument within ±[`MAX_MM`].
fn vec3_arg(a: &Map<String, Value>, k: &str) -> Result<Option<[f64; 3]>, String> {
    let Some(v) = a.get(k) else { return Ok(None) };
    let xs: Vec<f64> = v.as_array().map(|l| l.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    match xs.as_slice() {
        [x, y, z] if xs.iter().all(|c| c.is_finite() && c.abs() <= MAX_MM) => Ok(Some([*x, *y, *z])),
        _ => Err(format!("`{k}` must be [x, y, z] (finite numbers)")),
    }
}

/// A finite number argument in `lo..=hi`.
fn num_arg(a: &Map<String, Value>, k: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    match a.get(k) {
        None => Ok(None),
        Some(v) => {
            v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)).map(Some).ok_or_else(|| format!("`{k}` must be a number from {lo} to {hi}"))
        }
    }
}

/// A whole-number argument in `lo..=hi`.
fn count_arg(a: &Map<String, Value>, k: &str, lo: u64, hi: u64) -> Result<Option<u64>, String> {
    match a.get(k) {
        None => Ok(None),
        Some(v) => v.as_u64().filter(|n| (lo..=hi).contains(n)).map(Some).ok_or_else(|| format!("`{k}` must be a whole number from {lo} to {hi}")),
    }
}

fn unit(v: [f64; 3], k: &str) -> Result<[f64; 3], String> {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n.is_nan() || n <= 1e-12 {
        return Err(format!("`{k}` must not be the zero vector"));
    }
    Ok([v[0] / n, v[1] / n, v[2] / n])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn fastener_hole_pattern(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<Value, String> {
    let kind = str_arg(a, "pattern").ok_or("`pattern` is required (circular or rectangular)")?;
    let standard = str_arg(a, "standard").ok_or("`standard` is required")?;
    din912(standard).ok_or_else(|| format!("unsupported standard `{standard}` (M2, M3, M4, M5 or M6)"))?;
    let mut hole = Map::new();
    for k in ["standard", "depth", "counterbore"] {
        if let Some(v) = a.get(k) {
            hole.insert(k.into(), v.clone());
        }
    }
    // The first hole, the pattern's parameters and every hole's centre (for the agent to check).
    let (first, pattern_cmd, mut params, centres) = match kind {
        "circular" => {
            let c = vec3_arg(a, "center")?.ok_or("`center` is required for a circular pattern")?;
            let r = num_arg(a, "radius", 1e-3, MAX_MM)?.ok_or("`radius` is required for a circular pattern")?;
            let n = count_arg(a, "count", 2, 360)?.ok_or("`count` is required for a circular pattern")?;
            let start = num_arg(a, "start_angle_deg", -360.0, 360.0)?.unwrap_or(0.0).to_radians();
            let axis = unit(vec3_arg(a, "axis")?.unwrap_or([0.0, 0.0, 1.0]), "axis")?;
            // Two directions across the axis: the first from X (Y when the axis is along X).
            let seed = if axis[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let u = unit(cross(cross(axis, seed), axis), "axis")?;
            let v = cross(axis, u);
            let at = |t: f64| [0, 1, 2].map(|i| c[i] + r * (u[i] * t.cos() + v[i] * t.sin()));
            let step = std::f64::consts::TAU / n as f64;
            let centres: Vec<[f64; 3]> = (0..n).map(|i| at(start + step * i as f64)).collect();
            hole.insert("direction".into(), json!([-axis[0], -axis[1], -axis[2]]));
            (at(start), "solid.pattern.circular", json!({"axis": {"origin": c, "dir": axis}, "count": n}), centres)
        }
        "rectangular" => {
            let o = vec3_arg(a, "origin")?.ok_or("`origin` is required for a rectangular pattern")?;
            let nx = count_arg(a, "count_x", 1, 100)?.ok_or("`count_x` is required for a rectangular pattern")?;
            let ny = count_arg(a, "count_y", 1, 100)?.unwrap_or(1);
            if nx * ny < 2 {
                return Err("a rectangular pattern needs at least two holes (`count_x` × `count_y`)".into());
            }
            let sx = num_arg(a, "spacing_x", 1e-3, MAX_MM)?;
            let sy = num_arg(a, "spacing_y", 1e-3, MAX_MM)?;
            let sx = if nx > 1 { sx.ok_or("`spacing_x` is required when `count_x` > 1")? } else { sx.unwrap_or(1.0) };
            let sy = if ny > 1 { sy.ok_or("`spacing_y` is required when `count_y` > 1")? } else { sy.unwrap_or(1.0) };
            let dx = unit(vec3_arg(a, "dir_x")?.unwrap_or([1.0, 0.0, 0.0]), "dir_x")?;
            let dy = unit(vec3_arg(a, "dir_y")?.unwrap_or([0.0, 1.0, 0.0]), "dir_y")?;
            let dir = unit(vec3_arg(a, "direction")?.unwrap_or([0.0, 0.0, -1.0]), "direction")?;
            let centres: Vec<[f64; 3]> =
                (0..ny).flat_map(|j| (0..nx).map(move |i| [0, 1, 2].map(|k| o[k] + dx[k] * sx * i as f64 + dy[k] * sy * j as f64))).collect();
            hole.insert("direction".into(), json!(dir));
            let mut p = json!({"dir1": dx, "count1": nx, "spacing1": sx});
            if ny > 1 {
                p["dir2"] = json!(dy);
                p["count2"] = json!(ny);
                p["spacing2"] = json!(sy);
            }
            (o, "solid.pattern.rectangular", p, centres)
        }
        other => return Err(format!("unknown pattern `{other}` (circular or rectangular)")),
    };
    hole.insert("position".into(), json!(first));
    let before = total_volume(b)?;
    let drilled = fastener_metric_hole(b, &hole)?;
    let feature = drilled["result"]["name"].as_str().map(str::to_owned).ok_or("the hole did not report its feature name")?;
    params["features"] = json!([feature]);
    // Leave the design as it was: take back what this call added.
    let take_back = |b: &mut dyn Backend, steps: usize, why: String| {
        let undone = (0..steps).all(|_| exec(b, "edit.undo", json!({})).is_ok());
        format!("{why} ({})", if undone { "nothing was changed" } else { "undo the hole and pattern left behind" })
    };
    let one = total_volume(b).map(|v| before - v);
    let patterned = match exec(b, pattern_cmd, params) {
        Ok(v) => v,
        Err(e) => return Err(take_back(b, 1, e)),
    };
    // A pattern reports no per-instance result: every hole must cut as much as the first one. Less
    // means holes off the part, breaking out of an edge or running into each other.
    let n = centres.len() as f64;
    let removed = total_volume(b).map(|v| before - v);
    if let (Ok(one), Ok(removed)) = (one, removed) {
        let short = n - removed / one.max(1e-12);
        if one > 0.0 && short > 1e-3 {
            return Err(take_back(
                b,
                2,
                format!(
                    "the pattern's holes cut {removed:.3} mm³, {:.1} holes' worth less than {n} full holes of {one:.3} mm³: some are off the part, break out of an edge or overlap",
                    short
                ),
            ));
        }
    }
    Ok(json!({
        "status": "applied",
        "pattern": kind,
        "holes": centres.len(),
        "centres": centres,
        "hole_feature": feature,
        "pattern_feature": patterned.get("name"),
        "standard": standard,
        "drill_diameter_mm": drilled["drill_diameter_mm"],
        "counterbore_diameter_mm": drilled["counterbore_diameter_mm"],
        "counterbore_depth_mm": drilled["counterbore_depth_mm"],
        "drill_depth_mm": drilled["drill_depth_mm"],
    }))
}

/// The volume of every body together (mm³).
fn total_volume(b: &mut dyn Backend) -> Result<f64, String> {
    exec(b, "inspect.measure", json!({}))?
        .get("total")
        .and_then(|t| t.get("volume_mm3"))
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| "the design's volume could not be measured".to_string())
}

/// Typical filament densities (g/cm³).
fn filament_density(material: &str) -> Option<f64> {
    Some(match material {
        "PLA" => 1.24,
        "PETG" => 1.27,
        "ABS" => 1.04,
        "ASA" => 1.07,
        "TPU" => 1.21,
        "PA" => 1.14,
        _ => return None,
    })
}

fn fdm_print_estimate(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<Value, String> {
    let material = str_arg(a, "material").unwrap_or("PLA");
    let density = match num_arg(a, "density_g_cm3", 0.1, 30.0)? {
        Some(d) => d,
        None => filament_density(material).ok_or_else(|| format!("unknown material `{material}` (PLA, PETG, ABS, ASA, TPU or PA)"))?,
    };
    let infill = num_arg(a, "infill_percent", 0.0, 100.0)?.unwrap_or(15.0) / 100.0;
    let wall = num_arg(a, "wall_mm", 0.0, 100.0)?.unwrap_or(1.2);
    let dia = num_arg(a, "filament_diameter_mm", 0.1, 10.0)?.unwrap_or(1.75);
    let build = vec3_arg(a, "build_volume_mm")?.unwrap_or([220.0, 220.0, 250.0]);
    if build.iter().any(|d| *d <= 0.0) {
        return Err("`build_volume_mm` must be three positive sizes".into());
    }
    let p = match a.get("bodies") {
        Some(v) => json!({"bodies": v}),
        None => json!({}),
    };
    let m = exec(b, "inspect.measure", p)?;
    let bodies = m.get("bodies").and_then(Value::as_array).cloned().unwrap_or_default();
    if bodies.is_empty() {
        return Err("no bodies to print".into());
    }
    let mut sorted_build = build;
    sorted_build.sort_by(f64::total_cmp);
    let filament_area = std::f64::consts::PI * (dia / 2.0).powi(2);
    let (mut total_mm3, mut all_fit) = (0.0, true);
    let mut rows = Vec::new();
    for body in &bodies {
        let name = body.get("name").cloned().unwrap_or(Value::Null);
        let volume = body.get("volume_mm3").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0);
        let area = body.get("area_mm2").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0);
        let corner = |k: &str| body.get("bbox").and_then(|bb| vec3_value(bb.get(k)?));
        let size = match (corner("min"), corner("max")) {
            (Some(lo), Some(hi)) => [0, 1, 2].map(|i| (hi[i] - lo[i]).max(0.0)),
            _ => return Err(format!("body {name} has no bounding box")),
        };
        // The shell can't hold more than the whole part (thin parts are all shell).
        let shell = (area * wall).min(volume);
        let printed = shell + (volume - shell) * infill;
        total_mm3 += printed;
        let as_placed = (0..3).all(|i| size[i] <= build[i]);
        let mut sorted = size;
        sorted.sort_by(f64::total_cmp);
        let turned = (0..3).all(|i| sorted[i] <= sorted_build[i]);
        all_fit &= turned;
        rows.push(json!({
            "body": name,
            "size_mm": size,
            "solid_volume_mm3": volume,
            "printed_volume_mm3": printed,
            "mass_g": printed / 1000.0 * density,
            "filament_m": printed / filament_area / 1000.0,
            "fits_as_placed": as_placed,
            "fits_turned": turned,
        }));
    }
    Ok(json!({
        "material": if a.contains_key("density_g_cm3") { "custom" } else { material },
        "density_g_cm3": density,
        "infill_percent": infill * 100.0,
        "wall_mm": wall,
        "build_volume_mm": build,
        "bodies": rows,
        "total": {
            "printed_volume_mm3": total_mm3,
            "mass_g": total_mm3 / 1000.0 * density,
            "filament_m": total_mm3 / filament_area / 1000.0,
            "all_fit": all_fit,
        },
        "note": "Estimate: shell + infill only; supports, brims, top/bottom layers and purge are not counted.",
    }))
}

/// A finite `[x, y, z]` value.
fn vec3_value(v: &Value) -> Option<[f64; 3]> {
    match v.as_array()?.as_slice() {
        [x, y, z] => {
            let p = [x.as_f64()?, y.as_f64()?, z.as_f64()?];
            p.iter().all(|c| c.is_finite()).then_some(p)
        }
        _ => None,
    }
}

fn dfm_clearance_check(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<Value, String> {
    let body_a = str_arg(a, "body_a").ok_or("`body_a` is required")?;
    let body_b = str_arg(a, "body_b").ok_or("`body_b` is required")?;
    let min = a.get("min_clearance_mm").and_then(Value::as_f64).unwrap_or(0.25);
    if !min.is_finite() || !(0.0..=1e6).contains(&min) {
        return Err("`min_clearance_mm` must be a number from 0 to 1e6".into());
    }
    // Search at least 10 mm out, so a passing gap is still measured.
    let res = exec(b, "inspect.clearance", json!({"bodies": [body_a], "against": [body_b], "clearance": min.max(10.0), "nearest": 1}))?;
    if let Some(u) = res.get("unchecked").and_then(Value::as_array).and_then(|l| l.first()) {
        return Err(format!("the clearance between `{body_a}` and `{body_b}` could not be worked out: {}", u["reason"]));
    }
    if let Some(o) = res.get("overlaps").and_then(Value::as_array).and_then(|l| l.first()) {
        let vol = o.get("volume_mm3").and_then(Value::as_f64).unwrap_or(0.0);
        return Ok(json!({
            "passed": false,
            "measured_gap_mm": 0.0,
            "required_gap_mm": min,
            "is_colliding": true,
            "advice": format!("Bodies '{body_a}' and '{body_b}' collide with overlap volume {vol:.4} mm³. Add clearance/offset of at least {min:.3} mm to separate them."),
        }));
    }
    let near = res.get("near").and_then(Value::as_array).and_then(|l| l.first()).and_then(|n| n.get("distance_mm")).and_then(Value::as_f64);
    let nearest = || {
        res.get("nearest")
            .and_then(|m| m.get(body_a))
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|n| n.get("body").and_then(Value::as_str) == Some(body_b)))
            .and_then(|n| n.get("distance_mm"))
            .and_then(Value::as_f64)
    };
    let gap = near.or_else(nearest).ok_or_else(|| format!("no distance between `{body_a}` and `{body_b}` was measured"))?;
    let passed = gap >= min;
    let advice = if passed {
        format!("Clearance test PASSED: measured gap {gap:.3} mm meets or exceeds the required {min:.3} mm.")
    } else {
        format!(
            "Clearance test FAILED: measured gap {gap:.3} mm is below required {min:.3} mm. Widen gap by at least {:.3} mm to prevent binding when 3D printing (FDM).",
            min - gap
        )
    };
    Ok(json!({"passed": passed, "measured_gap_mm": gap, "required_gap_mm": min, "is_colliding": false, "advice": advice}))
}

fn undo_depth(b: &mut dyn Backend) -> Option<u64> {
    b.call("document.inspect", json!({})).ok().and_then(|d| d["undo"].as_u64())
}

/// How a failed batch goes back to where it started.
enum Undo {
    /// An engine checkpoint: the design, active sketch and undo history exactly as they were.
    Checkpoint(u64),
    /// An app without checkpoints: undo as many steps as the batch added (wrong once the undo
    /// history is full, as its depth no longer grows).
    Depth(u64),
    None,
}

impl Undo {
    fn begin(b: &mut dyn Backend) -> Undo {
        match exec(b, "edit.checkpoint", json!({})).ok().and_then(|v| v["checkpoint"].as_u64()) {
            Some(id) => Undo::Checkpoint(id),
            None => undo_depth(b).map_or(Undo::None, Undo::Depth),
        }
    }

    /// Go back; the number of steps undone.
    fn rollback(self, b: &mut dyn Backend, steps: usize) -> u64 {
        match self {
            Undo::Checkpoint(id) => match exec(b, "edit.restore_checkpoint", json!({"checkpoint": id})) {
                Ok(_) => steps as u64,
                Err(_) => 0,
            },
            Undo::Depth(before) => {
                let after = undo_depth(b).unwrap_or(before);
                let mut undone = 0u64;
                for _ in before..after {
                    if exec(b, "edit.undo", json!({})).is_err() {
                        break;
                    }
                    undone += 1;
                }
                undone
            }
            Undo::None => 0,
        }
    }

    /// The batch went through: forget the checkpoint.
    fn finish(self, b: &mut dyn Backend) {
        if let Undo::Checkpoint(id) = self {
            let _ = exec(b, "edit.restore_checkpoint", json!({"checkpoint": id, "forget": true}));
        }
    }
}

/// `batch`: run steps in order, stop at the first failure and (by default) go back to the
/// design as it was before the batch.
fn batch(b: &mut dyn Backend, a: &Map<String, Value>) -> Value {
    let Some(steps) = a.get("commands").and_then(Value::as_array) else {
        return text_result(&json!("`commands` must be an array of {command, params}"), true);
    };
    if steps.len() > MAX_BATCH {
        return text_result(&json!(format!("batch too long ({} steps, at most {MAX_BATCH})", steps.len())), true);
    }
    let rollback = a.get("rollback").and_then(Value::as_bool).unwrap_or(true);
    let verbose = a.get("verbose").and_then(Value::as_bool).unwrap_or(true);
    let profile = a.get("profile").and_then(Value::as_bool).unwrap_or(false);
    let timed = profile || a.get("timings").and_then(Value::as_bool).unwrap_or(false);
    // An app that runs batches itself answers the whole batch in one request, so one frame
    // (#743). One too long for a control line goes step by step: the app would refuse it unrun.
    // The margin covers the envelope and the `allow_paths` the backend adds.
    let mut params = json!({"commands": steps, "rollback": rollback});
    if timed && let Some(o) = params.as_object_mut() {
        o.insert("timings".into(), Value::Bool(true));
        o.insert("profile".into(), Value::Bool(profile));
    }
    if params.to_string().len() + 64 * 1024 <= solidcraft_engine::MAX_LINE {
        match b.call("engine.batch", params) {
            Ok(r) => return batch_reply(&r, rollback, verbose),
            // Older apps and the headless session, or the app wants the steps one by one.
            Err(e) if e.starts_with("unknown method") || e.starts_with("engine.batch unavailable") => {}
            // Never sent: no app to send it to.
            Err(e) if e.starts_with(crate::backend::NOT_RUNNING) => return text_result(&Value::String(e), true),
            // The app may have run it: sending the steps again would apply them twice.
            Err(e) => {
                let msg = format!("the app did not answer engine.batch ({e}); it may have run: inspect the design before sending it again");
                return text_result(&Value::String(msg), true);
            }
        }
    }
    let start = if rollback { Undo::begin(b) } else { Undo::None };
    let mut results = Vec::with_capacity(steps.len());
    let mut rows = Vec::new();
    let clock = timing::Clock::start();
    // The kernel's share can only be measured where the commands run: in this process.
    let profile = profile && b.in_process();
    for (i, step) in steps.iter().enumerate() {
        let command = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str);
        let (r, took) = timing::time(profile, || match command {
            Some(id) => exec(b, id, step.get("params").cloned().filter(|p| !p.is_null()).unwrap_or(json!({}))),
            None => Err("missing `command`".to_string()),
        });
        if timed {
            rows.push(took.to_json(i, command.unwrap_or_default()));
        }
        match r {
            Ok(v) => results.push(if verbose { v } else { compact(v) }),
            Err(e) => {
                let undone = start.rollback(b, i);
                let reply = json!({
                    "ok": false,
                    "completed": i,
                    "failed": {"index": i, "command": command, "error": e},
                    "rolled_back": rollback,
                    "undone": undone,
                    "results": results,
                });
                return text_result(&with_timings(reply, timed, rows, clock.ms()), true);
            }
        }
    }
    start.finish(b);
    let reply = json!({"ok": true, "completed": results.len(), "results": results});
    text_result(&with_timings(reply, timed, rows, clock.ms()), false)
}

/// `reply` with `timings` and `total_ms` when they were asked for.
fn with_timings(mut reply: Value, timed: bool, rows: Vec<Value>, total_ms: f64) -> Value {
    if timed && let Some(o) = reply.as_object_mut() {
        o.insert("timings".into(), Value::Array(rows));
        o.insert("total_ms".into(), json!(timing::round_ms(total_ms)));
    }
    reply
}

/// The `batch` reply from the app's `engine.batch` answer, shaped as the per-step path shapes it.
fn batch_reply(r: &Value, rollback: bool, verbose: bool) -> Value {
    let results: Vec<Value> =
        r["results"].as_array().cloned().unwrap_or_default().into_iter().map(|v| if verbose { v } else { compact(v) }).collect();
    let (mut reply, error) = match r.get("failed").filter(|f| !f.is_null()) {
        None => (json!({"ok": true, "completed": results.len(), "results": results}), false),
        Some(failed) => (
            json!({
                "ok": false,
                "completed": r["completed"],
                "failed": failed,
                "rolled_back": rollback,
                "undone": r["undone"],
                "results": results,
            }),
            true,
        ),
    };
    // The app timed the steps where they ran.
    if let Some(o) = reply.as_object_mut() {
        for k in ["timings", "total_ms"] {
            if let Some(v) = r.get(k).filter(|v| !v.is_null()) {
                o.insert(k.into(), v.clone());
            }
        }
    }
    text_result(&reply, error)
}

/// A command's result without what the design already had (`execute` or `batch` with `verbose: false`): a
/// feature's full body list (its created, changed and removed bodies stay), and the sketch ids
/// a sketch command repeats outside `created`.
fn compact(mut v: Value) -> Value {
    let Some(o) = v.as_object_mut() else { return v };
    if o.contains_key("created_bodies") {
        o.remove("bodies");
    }
    if let Some(created) = o.get("created").cloned() {
        for k in ["points", "curves", "constraints", "dimensions"] {
            if o.get(k).is_some_and(|x| created.get(k) == Some(x)) {
                o.remove(k);
            }
        }
    }
    v
}

static SHOT: AtomicU64 = AtomicU64::new(0);

/// `screenshot`: render or capture to a file, return it as an MCP image content block.
fn screenshot(b: &mut dyn Backend, a: &Map<String, Value>) -> Value {
    match capture(b, a) {
        Ok((png, info)) => json!({"content": [
            {"type": "image", "data": base64(&png), "mimeType": "image/png"},
            {"type": "text", "text": info.to_string()},
        ]}),
        Err(e) => text_result(&Value::String(e), true),
    }
}

fn capture(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<(Vec<u8>, Value), String> {
    // `grid` and `axes` shape a model render; the window shows what it shows.
    let guides: Vec<(&str, bool)> = ["grid", "axes"].into_iter().filter_map(|k| a.get(k).and_then(Value::as_bool).map(|v| (k, v))).collect();
    let window = match str_arg(a, "source") {
        Some("model") => false,
        Some("window") if !guides.is_empty() => {
            return Err("screenshot: `grid` and `axes` apply to model renders; use source \"model\" (the window shows what it shows)".into());
        }
        Some("window") => true,
        _ => b.has_ui() && guides.is_empty(),
    };
    let apply = |b: &mut dyn Backend, params: Value| b.call("engine.execute", json!({"command": "view.apply", "params": params}));
    match (str_arg(a, "view"), a.get("camera").filter(|c| !c.is_null())) {
        (Some(_), Some(_)) => return Err("screenshot: give `view` or `camera`, not both".into()),
        (None, Some(c)) => {
            apply(b, json!({"camera": c}))?;
        }
        (Some(v), None) if VIEWS.contains(&v) => {
            b.call("ui.view", json!({"view": v}))?;
            // Turning to a standard view also frames the model (like a axis view click).
            if v != "fit" && v != "home" {
                b.call("ui.view", json!({"view": "fit"}))?;
            }
        }
        // A saved named view.
        (Some(v), None) => {
            apply(b, json!({"view": v}))?;
        }
        (None, None) => {}
    }
    let keep = str_arg(a, "path").filter(|p| !p.trim().is_empty()).map(str::to_string);
    let path = keep.clone().unwrap_or_else(|| {
        let n = SHOT.fetch_add(1, Ordering::Relaxed);
        let dir = crate::scratch_dir().unwrap_or_else(vfs::temp_dir);
        dir.join(format!("shot-{n}.png")).to_string_lossy().to_string()
    });
    let size = |k: &str, d: u64| a.get(k).and_then(Value::as_u64).unwrap_or(d).clamp(16, 4096);
    let render = |b: &mut dyn Backend| {
        let mut params = json!({"path": path, "width": size("width", 960), "height": size("height", 600)});
        for (k, v) in &guides {
            params[*k] = json!(v);
        }
        b.call("ui.render", params)
    };
    let asked_window = str_arg(a, "source") == Some("window");
    let mut fallback = None;
    let (info, source) = if window {
        match b.call("ui.screenshot", json!({"path": path})) {
            Ok(v) => (v, "window"),
            // Asked for the window: say why there is none, so the caller can show it or ask for
            // a model render (#1677).
            Err(e) if asked_window => {
                return Err(format!("screenshot: the window gave no picture ({e}); show the window, or use source \"model\""));
            }
            // No presented frame (minimised or hidden window): a model render, and the reply says so.
            Err(e) => {
                fallback = Some(e);
                (render(b)?, "model")
            }
        }
    } else {
        (render(b)?, "model")
    };
    let read = vfs::read(&path, MAX_IMAGE_BYTES).map_err(|e| format!("{path}: {e} (is the app on another machine?)"));
    if keep.is_none() {
        let _ = vfs::remove(&path);
    }
    let png = read?;
    let (w, h) = match source {
        "model" => (json!(size("width", 960)), json!(size("height", 600))),
        _ => (info.get("width").cloned().unwrap_or(Value::Null), info.get("height").cloned().unwrap_or(Value::Null)),
    };
    let bytes = png.len();
    let mut reply = json!({"source": source, "path": keep, "width": w, "height": h, "bytes": bytes});
    if let Some(why) = fallback {
        reply["fallback"] = json!(format!("the window gave no picture ({why}); this is a model render"));
    }
    // What drew a model render (gpu, gpu-software or cpu), and the warning a CPU render carries.
    for k in ["renderer", "warning"] {
        if let Some(v) = info.get(k).filter(|v| !v.is_null()) {
            reply[k] = v.clone();
        }
    }
    Ok((png, reply))
}

/// Standard base64 with padding (RFC 4648).
pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}
