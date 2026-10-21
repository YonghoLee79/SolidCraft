# Drawing sheets (TCVN)

`file.export_drawing_dxf` (MCP: `export_drawing`) writes one 2D DXF drawing sheet laid out the
way Vietnamese technical drawings are: the sheet and its frame, a title block (*khung tên*) and
the shown bodies' views at a standard scale. It is a precursor of the Drawing workspace (M11):
the sheet is exported, not saved in the design, and it has no dimensions yet.

```json
{"command": "file.export_drawing_dxf", "params": {
  "sheet": "A3", "title_block": "production", "path": "out/bracket.dxf",
  "fields": {"title": "Giá đỡ", "drawing_no": "SC-01", "material": "CT3", "drawn_by": "Nguyễn Văn A",
             "drawn_date": "19/10/2026", "organization": "Xưởng cơ khí"}}}
```

## What the sheet follows

| Part | Rule | Here |
| --- | --- | --- |
| Sheet sizes | TCVN 7285:2003 (ISO 5457) | A0 841×1189, A1 594×841, A2 420×594, A3 297×420, A4 210×297 mm; landscape by default, A4 portrait |
| Frame (*khung bản vẽ*) | TCVN 7285:2003 | `tcvn`: 20 mm binding margin on the left, 10 mm elsewhere; `compact`: 20 mm left, 5 mm elsewhere. Centring marks at the middle of each side, 5 mm into the drawing area |
| Title block (*khung tên*) | TCVN 3821:2008 (ISO 7200) | In the frame's lower right corner. The standard leaves the cells to the organisation; the two layouts below are SolidCraft's own |
| Scale (*tỉ lệ*) | TCVN 7286:2003 (ISO 5455) | The largest of 50:1 … 2:1, 1:1, 1:2 … 1:10000 (1, 2 and 5 times a power of ten) at which the views fit with at least 10 mm between them; `scale` sets another ratio, refused when it doesn't fit |
| Lines (*nét vẽ*) | TCVN 8-20:2002 (ISO 128-20) | Frame, title block outline and visible outlines 0.5 mm (DXF lineweight 50); hidden edges dashed, cells, sheet edge and text 0.25 mm |
| Text (*chữ*) | TCVN 7284 (ISO 3098) | Nominal heights 7, 5, 3.5 and 2.5 mm, shrunk to fit a cell but never below 1.8 mm. Vietnamese letters are written as DXF `\U+XXXX` escapes: choose a Unicode font for the `Standard` style in the viewer |
| Views (*hình chiếu*) | First-angle projection (ISO 5456-2 method E) | Front view in the middle, the view from above under it, the view from the left on its right; `projection: third_angle` turns this over. The iso view goes beside the view under the front |

## Title blocks

**`production`, 180×55 mm** (*dùng trong sản xuất*): revision table (mark, document, signature,
date; up to three rows) over the signature rows (designed, drawn, checked, standards check,
approved: name, signature, date); the part's name, its drawing code and material in the middle;
quantity, mass and scale, sheet number and count, and the organisation on the right. The drawing
code is repeated upside down in a 70×14 mm box in the frame's upper left corner.

**`school`, 140×32 mm** (*dùng trong trường học*): drawn and checked (name, date), the school or
class; the exercise or part and the drawing code; material and scale.

`fields` holds the text (at most 80 letters each): `title`, `drawing_no`, `material`, `quantity`,
`mass`, `sheet`, `sheets`, `organization`, `designed_by`/`_date`, `drawn_by`/`_date`,
`checked_by`/`_date`, `standards_by`/`_date`, `approved_by`/`_date` and `revisions`. The scale
cell is always the scale used. The reply lists in `unused_fields` the fields the chosen block
has no cell for; an unknown field is an error.

## Layers and coordinates

Everything is in paper millimetres with the sheet's lower left corner at 0,0. Layers: `SHEET`
(sheet edge), `FRAME` (frame, title block outline, centring marks), `TITLE_LINES`, `TITLE_TEXT`,
`VISIBLE` and `HIDDEN`. Views are the hidden-line drawings of [Export Current View as
DXF](view-dxf.md) at the chosen scale; curves are approximated within `tolerance` (model mm).
The reply gives each view's centre and size on the sheet.

Not yet: dimensions, centre lines, section views, a projection symbol, saving the sheet in the
design and editing it in the app.
