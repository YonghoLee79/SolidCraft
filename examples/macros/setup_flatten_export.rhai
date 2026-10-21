// Sheet metal box
run("sketch.create", #{ plane: "XY", name: "Base" });
run("sketch.rectangle.two_point", #{ p0: [0, 0], p1: [80, 60] });
run("sketch.finish");
run("sheet.flange", #{ sketch: "Base" });
run("sheet.flange", #{ edges: [[40, 0, 2.5], [80, 30, 2.5], [40, 60, 2.5], [0, 30, 2.5]], height: 20 });
