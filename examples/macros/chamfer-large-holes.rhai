// Chamfer holes wider than 2 mm
let holes = run("inspect.find_holes", #{ min_diameter: "2 mm" });
if holes.holes != () {
    for h in holes.holes {
        run("solid.chamfer", #{ edges: h.edges, distance: "1 mm" });
    }
}
