// Spur gear
// @param teeth Teeth | 12
// @param gear_module Module | 2
// @param bore Bore diameter | 8
// @param thickness Thickness | 8
fn as_n(v) {
    if type_of(v) == "string" { parse_float(v) } else { v }
}
let z = as_n(teeth);
let m = as_n(gear_module);
let bore_d = as_n(bore);
let t = as_n(thickness);
let outer_r = m * z / 2.0 + m;
let root_r = m * z / 2.0 - 1.25 * m;
let pi = 3.141592653589793;
let pts = [];
let i = 0;
while i < z {
    let a = 2.0 * pi * i / z;
    let half = pi / (2.0 * z);
    let a0 = a - half * 1.2;
    let a1 = a - half * 0.65;
    let a2 = a + half * 0.65;
    let a3 = a + half * 1.2;
    pts.push([root_r * cos(a0), root_r * sin(a0)]);
    pts.push([outer_r * cos(a1), outer_r * sin(a1)]);
    pts.push([outer_r * cos(a2), outer_r * sin(a2)]);
    pts.push([root_r * cos(a3), root_r * sin(a3)]);
    i = i + 1;
}
run("sketch.create", #{ plane: "XY", name: "Gear" });
run("sketch.line", #{ points: pts, closed: true });
run("sketch.finish");
run("solid.extrude", #{ sketch: "Gear", distance: t, name: "Gear" });
run("solid.hole", #{ position: [0, 0, t], diameter: bore_d });
