// One hole on the top face, then a circular pattern
// @param count Count | 6
// @param pcd Pitch circle mm | 70
// @param hole Hole diameter mm | 6.5
let info = run("inspect.measure", #{ });
if info.body_count < 1 {
    throw "open a solid first (examples/macros/flange-blank.start.json)";
}
let body = info.bodies[0].name;
let faces = run("model.faces", #{ body: body });
let best = 0.0;
let z = 0.0;
for f in faces.faces {
    let n = f.plane_normal;
    if n != () && n[2] > 0.9 && f.area > best {
        best = f.area;
        z = f.centroid[2];
    }
}
let r = pcd / 2.0;
run("solid.hole", #{ position: [r, 0.0, z], diameter: hole + " mm", name: "BoltHole" });
run("solid.pattern.circular", #{ features: ["BoltHole"], axis: "Z", count: count });
