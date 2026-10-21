// Chamfer the edges currently selected
let info = run("document.inspect", #{ });
let edges = [];
if info.selection != () {
    for item in info.selection {
        if item["type"] == "edge" {
            edges.push(item.point);
        }
    }
}
run("solid.chamfer", #{ edges: edges, distance: "1 mm" });
