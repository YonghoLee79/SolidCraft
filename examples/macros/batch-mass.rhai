// Folder of STEP → mass CSV
// @param dir Folder | .
// @param out CSV path | mass.csv
let files = list_dir(dir);
let rows = "file,volume_mm3,mass_g,area_mm2\n";
for path in files {
    let p = path;
    if p.contains(".step") || p.contains(".stp") || p.contains(".STEP") || p.contains(".STP") {
        run("doc.open", #{ path: p });
        let m = run("inspect.measure", #{ });
        let t = m.total;
        rows += p + "," + t.volume_mm3 + "," + t.mass_g + "," + t.area_mm2 + "\n";
    }
}
write_file(out, rows);
