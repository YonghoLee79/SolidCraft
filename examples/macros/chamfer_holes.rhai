// Chamfer holes by size
// @param min_dia Minimum diameter | 5 mm
// @param max_dia Maximum diameter | 20 mm
// @param chamfer Chamfer distance | 1 mm
let holes = run("inspect.find_holes", #{ min_diameter: min_dia, max_diameter: max_dia });
run("solid.chamfer", #{ edges: holes.edges, distance: chamfer });
