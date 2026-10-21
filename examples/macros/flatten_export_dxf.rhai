// Flatten and export DXF
// @param path DXF file | flat.dxf
run("sheet.flat_pattern", #{ });
run("sheet.export_dxf", #{ path: path });
