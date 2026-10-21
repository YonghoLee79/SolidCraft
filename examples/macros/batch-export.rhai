// Folder of STEP → STEP/STL
// @param dir Folder | .
// @param out_dir Output folder | export
// @param ext Extension | step
let files = list_dir(dir);
for path in files {
    let p = path;
    if p.contains(".step") || p.contains(".stp") || p.contains(".STEP") || p.contains(".STP") {
        run("doc.open", #{ path: p });
        let stem = file_stem(p);
        run("file.export", #{ path: out_dir + "/" + stem + "." + ext });
    }
}
