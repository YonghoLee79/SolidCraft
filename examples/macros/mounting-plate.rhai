// Mounting plate
// @param length Length | 200
// @param width Width | 150
let offset = 12.0;
let pitch = 50.0;
let min_pitch = 40.0;
let thick = 8.0;

let span_x = length - 2.0 * offset;
if span_x < 0 {
    throw "plate too small for a 12 mm edge offset";
}
let nx = (span_x / pitch + 0.5).floor();
if nx < 1 { nx = 1; }
while nx > 1 && span_x / nx < min_pitch { nx -= 1; }
let step_x = span_x / nx;

let span_y = width - 2.0 * offset;
if span_y < 0 {
    throw "plate too small for a 12 mm edge offset";
}
let ny = (span_y / pitch + 0.5).floor();
if ny < 1 { ny = 1; }
while ny > 1 && span_y / ny < min_pitch { ny -= 1; }
let step_y = span_y / ny;

run("solid.box", #{ length: length, width: width, height: thick, name: "Plate" });
let i = 1;
let ix = 0;
while ix <= nx {
    let x = offset + ix * step_x;
    run("solid.hole", #{ position: [x, offset, thick], clearance: "M8", name: "M8_" + i });
    i += 1;
    run("solid.hole", #{ position: [x, width - offset, thick], clearance: "M8", name: "M8_" + i });
    i += 1;
    ix += 1;
}
let iy = 1;
while iy < ny {
    let y = offset + iy * step_y;
    run("solid.hole", #{ position: [offset, y, thick], clearance: "M8", name: "M8_" + i });
    i += 1;
    run("solid.hole", #{ position: [length - offset, y, thick], clearance: "M8", name: "M8_" + i });
    i += 1;
    iy += 1;
}
