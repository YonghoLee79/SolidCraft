// Plate with four corner holes
// @param length Length | 120
// @param width Width | 80
// @param height Thickness | 8
// @param hole_dia Hole diameter | 6.5 mm
// @param edge_offset Hole offset from the edges | 10
run("solid.box", #{ length: length, width: width, height: height, name: "Plate" });
run("solid.hole", #{ position: [edge_offset, edge_offset, height], diameter: hole_dia, name: "HoleA" });
run("solid.hole", #{ position: [length - edge_offset, edge_offset, height], diameter: hole_dia, name: "HoleB" });
run("solid.hole", #{ position: [edge_offset, width - edge_offset, height], diameter: hole_dia, name: "HoleC" });
run("solid.hole", #{ position: [length - edge_offset, width - edge_offset, height], diameter: hole_dia, name: "HoleD" });
