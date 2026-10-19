//! A drawing sheet laid out the way Vietnamese technical drawings are: an ISO A-size sheet with
//! its frame and centring marks (TCVN 7285:2003, the Vietnamese edition of ISO 5457), a title
//! block (TCVN 3821:2008 / ISO 7200 leave its cells to the organisation; the two layouts here are
//! SolidCraft's own, with the cells Vietnamese school and production drawings carry), and the
//! bodies' parallel views at a standard scale (TCVN 7286:2003 / ISO 5455: 1, 2 and 5 times a
//! power of ten), arranged by first-angle projection (ISO 5456-2 method E, the one Vietnamese
//! drawings use) or third-angle. Visible outlines are thick (0.5 mm), hidden edges, cells and
//! text thin (0.25 mm), after TCVN 8-20:2002 / ISO 128-20. One DXF in paper millimetres.

use serde_json::{Map, Value, json};
use solidcraft_geom::Vec2;
use solidcraft_io::{DxfEntity, DxfLayer, DxfLinetype, DxfShape};
use solidcraft_render::StandardView;

use super::CommandSpec;
use crate::params::{bad, str_};
use crate::{Result, Session};

const CMD: &str = "file.export_drawing_dxf";
pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(CMD, "Export Drawing Sheet as DXF", export).icon("export").noundo().params(
    "sheet?: A0|A1|A2|A3|A4 (default A3); orientation?: landscape|portrait (default landscape, A4 portrait); \
         margins?: tcvn (20 mm left for binding, 10 mm elsewhere; default)|compact (20 mm left, 5 mm elsewhere); \
         title_block?: production (180×55 mm)|school (140×32 mm)|none (default production); \
         fields?: {title, drawing_no, material, quantity, mass, sheet, sheets, organization, designed_by, designed_date, \
         drawn_by, drawn_date, checked_by, checked_date, standards_by, standards_date, approved_by, approved_date: text or number, \
         at most 80 letters; revisions: up to 3 {mark, document, signature, date}}; \
         views?: front|back|left|right|top|bottom|iso list (default front, top, left); projection?: first_angle (TCVN, default)|third_angle; \
         scale?: \"1:2\"-style ratio (default the largest of 1, 2 and 5 times a power of ten that fits); \
         hidden?: dashed|omit (default dashed); tolerance?: 0.0001..1 mm (default 0.01); path?: .dxf file (else returns dxf text). \
         Visible bodies only; everything is in paper millimetres with the sheet's lower left corner at 0,0.",
)];

/// The longest text a cell accepts (letters).
const MAX_TEXT: usize = 80;
/// The fewest millimetres between two views and between a view and the drawing area's edge.
const MIN_GAP: f64 = 10.0;
/// Text width per letter as a share of its height, enough for the usual drafting fonts.
const LETTER_WIDTH: f64 = 0.8;
/// The smallest text height (TCVN 7284 / ISO 3098 nominal size).
const MIN_TEXT: f64 = 1.8;

const FIELDS: &[&str] = &[
    "title",
    "drawing_no",
    "material",
    "quantity",
    "mass",
    "sheet",
    "sheets",
    "organization",
    "designed_by",
    "designed_date",
    "drawn_by",
    "drawn_date",
    "checked_by",
    "checked_date",
    "standards_by",
    "standards_date",
    "approved_by",
    "approved_date",
];

/// A sheet's size in millimetres, portrait (width, height).
pub(super) fn sheet_size(name: &str) -> Option<(f64, f64)> {
    Some(match name.to_ascii_uppercase().as_str() {
        "A0" => (841.0, 1189.0),
        "A1" => (594.0, 841.0),
        "A2" => (420.0, 594.0),
        "A3" => (297.0, 420.0),
        "A4" => (210.0, 297.0),
        _ => return None,
    })
}

/// A rectangle in paper millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
}

/// A drawing scale `num:den` (paper over model).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Scale {
    pub num: f64,
    pub den: f64,
}

impl Scale {
    pub fn factor(self) -> f64 {
        self.num / self.den
    }
    pub fn label(self) -> String {
        format!("{}:{}", self.num, self.den)
    }
    /// `"1:2"`, `"2:1"`, `"1:2.5"`: two positive numbers up to 10000.
    pub fn parse(s: &str) -> Option<Scale> {
        let (a, b) = s.trim().split_once(':')?;
        let n = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0 && *v <= 10000.0);
        Some(Scale { num: n(a)?, den: n(b)? })
    }
    /// The standard scales from 50:1 to 1:10000, largest first.
    fn standard() -> impl Iterator<Item = Scale> {
        let up = [50.0, 20.0, 10.0, 5.0, 2.0].map(|num| Scale { num, den: 1.0 });
        let down = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0].map(|den| Scale { num: 1.0, den });
        up.into_iter().chain(down)
    }
    /// The largest standard scale no larger than `factor`.
    pub fn fitting(factor: f64) -> Option<Scale> {
        Scale::standard().find(|s| s.factor() <= factor * (1.0 + 1e-9))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Block {
    Production,
    School,
    None,
}

impl Block {
    /// Width and height in millimetres.
    pub fn size(self) -> (f64, f64) {
        match self {
            Block::Production => (180.0, 55.0),
            Block::School => (140.0, 32.0),
            Block::None => (0.0, 0.0),
        }
    }
}

/// One title block cell: its rectangle relative to the block's lower left corner, and its text
/// (nominal height in millimetres; shrunk to fit).
struct Cell {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    text: String,
    size: f64,
}

fn cell(x: f64, y: f64, w: f64, h: f64, text: impl Into<String>, size: f64) -> Cell {
    Cell { x, y, w, h, text: text.into(), size }
}

/// The cells of a title block, filled from `f` (missing fields are empty) and the scale.
fn cells(block: Block, f: &Map<String, Value>, scale: &str) -> Vec<Cell> {
    let v = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut c = Vec::new();
    match block {
        Block::None => {}
        Block::School => {
            // Left: who drew and checked it, then the school, faculty or class.
            for (row, label, who, date) in [(24.0, "Người vẽ", "drawn_by", "drawn_date"), (16.0, "Kiểm tra", "checked_by", "checked_date")] {
                c.push(cell(0.0, row, 20.0, 8.0, label, 2.5));
                c.push(cell(20.0, row, 30.0, 8.0, v(who), 3.5));
                c.push(cell(50.0, row, 20.0, 8.0, v(date), 2.5));
            }
            c.push(cell(0.0, 0.0, 70.0, 16.0, v("organization"), 3.5));
            // Middle: the exercise or part, and the drawing's code.
            c.push(cell(70.0, 16.0, 45.0, 16.0, v("title"), 5.0));
            c.push(cell(70.0, 0.0, 45.0, 16.0, v("drawing_no"), 3.5));
            // Right: material and scale, each under its label.
            c.push(cell(115.0, 24.0, 25.0, 8.0, "Vật liệu", 2.5));
            c.push(cell(115.0, 16.0, 25.0, 8.0, v("material"), 3.5));
            c.push(cell(115.0, 8.0, 25.0, 8.0, "Tỉ lệ", 2.5));
            c.push(cell(115.0, 0.0, 25.0, 8.0, scale, 3.5));
        }
        Block::Production => {
            // Left, top: the revision table, filled upwards from the signatures, under its header.
            let header = [(0.0, 10.0, "Sửa"), (10.0, 25.0, "Số tài liệu"), (35.0, 15.0, "Chữ ký"), (50.0, 15.0, "Ngày")];
            for (x, w, label) in header {
                c.push(cell(x, 50.0, w, 5.0, label, 2.5));
            }
            let revisions = f.get("revisions").and_then(Value::as_array).cloned().unwrap_or_default();
            for (i, y) in [35.0, 40.0, 45.0].into_iter().enumerate() {
                let r = revisions.get(i).and_then(Value::as_object);
                let rv = |k: &str| r.and_then(|r| r.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
                for ((x, w, _), k) in header.iter().zip(["mark", "document", "signature", "date"]) {
                    c.push(cell(*x, y, *w, 5.0, rv(k), 2.5));
                }
            }
            // Left, bottom: who designed, drew, checked against the standards and approved it.
            let people = [
                ("Thiết kế", "designed_by", "designed_date"),
                ("Người vẽ", "drawn_by", "drawn_date"),
                ("Kiểm tra", "checked_by", "checked_date"),
                ("T.chuẩn", "standards_by", "standards_date"),
                ("Duyệt", "approved_by", "approved_date"),
            ];
            for (i, (label, who, date)) in people.into_iter().enumerate() {
                let y = 28.0 - 7.0 * i as f64;
                c.push(cell(0.0, y, 17.0, 7.0, label, 2.5));
                c.push(cell(17.0, y, 23.0, 7.0, v(who), 2.5));
                c.push(cell(40.0, y, 10.0, 7.0, "", 2.5));
                c.push(cell(50.0, y, 15.0, 7.0, v(date), 2.5));
            }
            // Middle: the product's name, the drawing's code, its material.
            c.push(cell(65.0, 27.0, 70.0, 28.0, v("title"), 7.0));
            c.push(cell(65.0, 14.0, 70.0, 13.0, v("drawing_no"), 5.0));
            c.push(cell(65.0, 0.0, 20.0, 14.0, "Vật liệu", 2.5));
            c.push(cell(85.0, 0.0, 50.0, 14.0, v("material"), 3.5));
            // Right: quantity, mass and scale under their labels, the sheet count, the organisation.
            for (x, w, label, value) in
                [(135.0, 10.0, "SL", v("quantity")), (145.0, 20.0, "Khối lượng", v("mass")), (165.0, 15.0, "Tỉ lệ", scale.to_string())]
            {
                c.push(cell(x, 48.0, w, 7.0, label, 2.5));
                c.push(cell(x, 38.0, w, 10.0, value, 3.5));
            }
            let count = |label: &str, k: &str| if v(k).is_empty() { label.to_string() } else { format!("{label} {}", v(k)) };
            c.push(cell(135.0, 30.0, 22.5, 8.0, count("Tờ:", "sheet"), 2.5));
            c.push(cell(157.5, 30.0, 22.5, 8.0, count("Số tờ:", "sheets"), 2.5));
            c.push(cell(135.0, 0.0, 45.0, 30.0, v("organization"), 3.5));
        }
    }
    c
}

/// The text height that fits `text` in a `w`×`h` cell: the nominal `size` or smaller, never
/// below the smallest standard height.
fn fit_height(text: &str, size: f64, w: f64, h: f64) -> f64 {
    let letters = text.chars().count().max(1) as f64;
    size.min((w - 1.5) / (letters * LETTER_WIDTH)).min(h - 2.0).max(MIN_TEXT)
}

fn line(layer: &str, a: Vec2, b: Vec2) -> DxfEntity {
    DxfEntity { layer: layer.into(), shape: DxfShape::Line(a, b) }
}

fn rect(layer: &str, r: Rect, out: &mut Vec<DxfEntity>) {
    let c = [Vec2::new(r.x0, r.y0), Vec2::new(r.x1, r.y0), Vec2::new(r.x1, r.y1), Vec2::new(r.x0, r.y1)];
    for i in 0..4 {
        out.push(line(layer, c[i], c[(i + 1) % 4]));
    }
}

/// One line of text centred in a cell; `turned` writes it upside down (180°).
fn centred_text(at: Vec2, height: f64, turned: bool, text: &str) -> DxfEntity {
    // `at` of a TEXT is the middle of its baseline: half the height below the centre, or above
    // it when the text is turned over.
    let at = Vec2::new(at.x, if turned { at.y + height / 2.0 } else { at.y - height / 2.0 });
    DxfEntity { layer: "TITLE_TEXT".into(), shape: DxfShape::Text { at, height, angle_deg: if turned { 180.0 } else { 0.0 }, text: text.into() } }
}

/// The title block drawn into the frame's lower right corner; the production block also gets
/// the drawing-code box, turned over, in the frame's upper left corner.
fn title_block(block: Block, frame: Rect, f: &Map<String, Value>, scale: &str, out: &mut Vec<DxfEntity>) {
    let (bw, bh) = block.size();
    if block == Block::None {
        return;
    }
    let o = Vec2::new(frame.x1 - bw, frame.y0);
    // The outline: its left and top edges (the others are the frame's).
    out.push(line("FRAME", o, Vec2::new(o.x, o.y + bh)));
    out.push(line("FRAME", Vec2::new(o.x, o.y + bh), Vec2::new(frame.x1, o.y + bh)));
    let mut seen: Vec<[i64; 4]> = Vec::new();
    let key = |a: Vec2, b: Vec2| {
        let k = |v: f64| (v * 1000.0).round() as i64;
        let (a, b) = if (a.x, a.y) <= (b.x, b.y) { (a, b) } else { (b, a) };
        [k(a.x), k(a.y), k(b.x), k(b.y)]
    };
    let on_outline = |a: Vec2, b: Vec2| {
        let same = |p: f64, q: f64, v: f64| (p - v).abs() < 1e-9 && (q - v).abs() < 1e-9;
        same(a.x, b.x, 0.0) || same(a.x, b.x, bw) || same(a.y, b.y, 0.0) || same(a.y, b.y, bh)
    };
    for c in cells(block, f, scale) {
        let corners = [Vec2::new(c.x, c.y), Vec2::new(c.x + c.w, c.y), Vec2::new(c.x + c.w, c.y + c.h), Vec2::new(c.x, c.y + c.h)];
        for i in 0..4 {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            let k = key(a, b);
            if on_outline(a, b) || seen.contains(&k) {
                continue;
            }
            seen.push(k);
            out.push(line("TITLE_LINES", o + a, o + b));
        }
        if !c.text.is_empty() {
            let h = fit_height(&c.text, c.size, c.w, c.h);
            out.push(centred_text(Vec2::new(o.x + c.x + c.w / 2.0, o.y + c.y + c.h / 2.0), h, false, &c.text));
        }
    }
    if block == Block::Production {
        let r = Rect { x0: frame.x0, y0: frame.y1 - 14.0, x1: frame.x0 + 70.0, y1: frame.y1 };
        out.push(line("FRAME", Vec2::new(r.x0, r.y0), Vec2::new(r.x1, r.y0)));
        out.push(line("FRAME", Vec2::new(r.x1, r.y0), Vec2::new(r.x1, r.y1)));
        let code = f.get("drawing_no").and_then(Value::as_str).unwrap_or("");
        if !code.is_empty() {
            let h = fit_height(code, 5.0, r.w(), r.h());
            out.push(centred_text(Vec2::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0), h, true, code));
        }
    }
}

/// The sheet's edge, the frame, the centring marks (from the edge to 5 mm inside the frame, at
/// the middle of each side) and the frame rectangle.
fn sheet_lines(paper: (f64, f64), frame: Rect, out: &mut Vec<DxfEntity>) {
    let (w, h) = paper;
    rect("SHEET", Rect { x0: 0.0, y0: 0.0, x1: w, y1: h }, out);
    rect("FRAME", frame, out);
    let (mx, my) = (w / 2.0, h / 2.0);
    out.push(line("FRAME", Vec2::new(0.0, my), Vec2::new(frame.x0 + 5.0, my)));
    out.push(line("FRAME", Vec2::new(w, my), Vec2::new(frame.x1 - 5.0, my)));
    out.push(line("FRAME", Vec2::new(mx, 0.0), Vec2::new(mx, frame.y0 + 5.0)));
    out.push(line("FRAME", Vec2::new(mx, h), Vec2::new(mx, frame.y1 - 5.0)));
}

/// Where a view goes in the arrangement around the front view: (column, row), rows counted
/// from the top.
pub(super) fn grid_cell(view: StandardView, first_angle: bool) -> (usize, usize) {
    use StandardView::*;
    match (view, first_angle) {
        (Front, _) => (1, 1),
        (Back, _) => (3, 1),
        (Top, true) | (Bottom, false) => (1, 2),
        (Bottom, true) | (Top, false) => (1, 0),
        (Left, true) | (Right, false) => (2, 1),
        (Right, true) | (Left, false) => (0, 1),
        // Out of the way of the projected views: beside the view under or over the front.
        (Iso, true) => (2, 2),
        (Iso, false) => (2, 0),
    }
}

/// The extent of entities (lower left, upper right); `None` when they have no points.
fn extent(entities: &[DxfEntity]) -> Option<(Vec2, Vec2)> {
    let mut pts = Vec::new();
    for e in entities {
        match &e.shape {
            DxfShape::Line(a, b) => pts.extend([*a, *b]),
            DxfShape::Polyline { points } => pts.extend(points.iter().copied()),
            _ => {}
        }
    }
    pts.into_iter().filter(|p| p.is_finite()).fold(None, |acc, p| {
        Some(acc.map_or((p, p), |(lo, hi): (Vec2, Vec2)| (Vec2::new(lo.x.min(p.x), lo.y.min(p.y)), Vec2::new(hi.x.max(p.x), hi.y.max(p.y)))))
    })
}

/// A view's entity moved onto the sheet: `p` becomes `at + k·p`.
fn place(e: DxfEntity, k: f64, at: Vec2) -> Option<DxfEntity> {
    let m = |p: Vec2| Vec2::new(at.x + k * p.x, at.y + k * p.y);
    let shape = match e.shape {
        DxfShape::Line(a, b) => DxfShape::Line(m(a), m(b)),
        DxfShape::Polyline { points } => DxfShape::Polyline { points: points.into_iter().map(m).collect() },
        _ => return None,
    };
    Some(DxfEntity { layer: e.layer, shape })
}

/// One view to lay out: its grid cell and its extent in model millimetres.
pub(super) struct Laid {
    pub cell: (usize, usize),
    pub iso: bool,
    pub lo: Vec2,
    pub hi: Vec2,
}

/// Columns' and rows' model sizes and the middles views in them line up on. Views in a column
/// share their horizontal axis and views in a row their vertical one (both about the bodies'
/// centre), so a column or row is the union of its views; the iso view is centred on itself.
struct Bands {
    size: [f64; 4],
    mid: [f64; 4],
    used: [bool; 4],
}

fn bands(views: &[Laid], column: bool) -> Bands {
    let mut b = Bands { size: [0.0; 4], mid: [0.0; 4], used: [false; 4] };
    for i in 0..4 {
        let here: Vec<&Laid> = views.iter().filter(|v| (if column { v.cell.0 } else { v.cell.1 }) == i).collect();
        let aligned: Vec<&&Laid> = here.iter().filter(|v| !v.iso).collect();
        let range = |l: &Laid| if column { (l.lo.x, l.hi.x) } else { (l.lo.y, l.hi.y) };
        let union = |it: &mut dyn Iterator<Item = (f64, f64)>| {
            it.fold(None, |acc: Option<(f64, f64)>, (a, z)| Some(acc.map_or((a, z), |(p, q)| (p.min(a), q.max(z)))))
        };
        let mid_range = union(&mut aligned.iter().map(|l| range(l)));
        let all = union(&mut here.iter().map(|l| range(l)));
        if let Some((a, z)) = all {
            b.used[i] = true;
            // The aligned views' own span, and the iso view's own, decide the band's width.
            b.size[i] = here.iter().map(|l| range(l).1 - range(l).0).fold(mid_range.map_or(0.0, |(p, q)| q - p), f64::max);
            b.mid[i] = mid_range.map_or((a + z) / 2.0, |(p, q)| (p + q) / 2.0);
        }
    }
    b
}

/// The largest factor (paper mm per model mm) at which the bands fit `length` with the minimum gaps.
fn fit_factor(b: &Bands, length: f64) -> f64 {
    let n = b.used.iter().filter(|u| **u).count() as f64;
    let total: f64 = b.size.iter().sum();
    let room = length - MIN_GAP * (n + 1.0);
    if room <= 0.0 {
        return 0.0;
    }
    if total <= 1e-9 { f64::INFINITY } else { room / total }
}

/// Paper centres of the bands along `length` from `start` at factor `k`, the free room shared
/// equally between the gaps.
fn centres(b: &Bands, start: f64, length: f64, k: f64, reverse: bool) -> [f64; 4] {
    let n = b.used.iter().filter(|u| **u).count() as f64;
    let gap = (length - k * b.size.iter().sum::<f64>()) / (n + 1.0);
    let mut c = [0.0; 4];
    let mut pos = gap;
    for ((centre, used), size) in c.iter_mut().zip(b.used).zip(b.size) {
        if used {
            let mid = pos + k * size / 2.0;
            *centre = if reverse { start + length - mid } else { start + mid };
            pos += k * size + gap;
        }
    }
    c
}

fn text_field(v: &Value, what: &str) -> Result<String> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        _ => return Err(bad(CMD, format!("{what} must be text or a number"))),
    };
    if s.chars().count() > MAX_TEXT {
        return Err(bad(CMD, format!("{what} is longer than {MAX_TEXT} letters")));
    }
    // One line of text: DXF values end at a line break.
    Ok(s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string())
}

/// The title block fields as text, checked; the revisions as a list of text objects.
fn fields(p: &Value) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    let Some(f) = p.get("fields") else { return Ok(out) };
    let f = f.as_object().ok_or_else(|| bad(CMD, "fields must be an object"))?;
    for (k, v) in f {
        if k == "revisions" {
            let list = v.as_array().ok_or_else(|| bad(CMD, "fields.revisions must be a list"))?;
            if list.len() > 3 {
                return Err(bad(CMD, "the title block holds at most 3 revisions"));
            }
            let mut rows = Vec::new();
            for r in list {
                let r = r.as_object().ok_or_else(|| bad(CMD, "a revision must be an object {mark, document, signature, date}"))?;
                let mut row = Map::new();
                for (rk, rv) in r {
                    if !["mark", "document", "signature", "date"].contains(&rk.as_str()) {
                        return Err(bad(CMD, format!("unknown revision field `{rk}` (mark, document, signature, date)")));
                    }
                    row.insert(rk.clone(), json!(text_field(rv, &format!("revision {rk}"))?));
                }
                rows.push(Value::Object(row));
            }
            out.insert(k.clone(), Value::Array(rows));
        } else if FIELDS.contains(&k.as_str()) {
            out.insert(k.clone(), json!(text_field(v, &format!("fields.{k}"))?));
        } else {
            return Err(bad(CMD, format!("unknown field `{k}` ({}, revisions)", FIELDS.join(", "))));
        }
    }
    Ok(out)
}

/// The fields a title block has no cell for.
fn unused(block: Block, f: &Map<String, Value>) -> Vec<String> {
    let shown: &[&str] = match block {
        Block::Production => &[
            "title",
            "drawing_no",
            "material",
            "quantity",
            "mass",
            "sheet",
            "sheets",
            "organization",
            "designed_by",
            "designed_date",
            "drawn_by",
            "drawn_date",
            "checked_by",
            "checked_date",
            "standards_by",
            "standards_date",
            "approved_by",
            "approved_date",
            "revisions",
        ],
        Block::School => &["title", "drawing_no", "material", "organization", "drawn_by", "drawn_date", "checked_by", "checked_date"],
        Block::None => &[],
    };
    f.keys().filter(|k| !shown.contains(&k.as_str())).cloned().collect()
}

fn choice<'a>(p: &'a Value, k: &str, default: &'a str, allowed: &[&str]) -> Result<&'a str> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(s.as_str()),
        _ => Err(bad(CMD, format!("{k} must be {}", allowed.join(", ")))),
    }
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let sheet_name = match p.get("sheet") {
        None | Some(Value::Null) => "A3".to_string(),
        Some(Value::String(v)) if sheet_size(v).is_some() => v.to_ascii_uppercase(),
        _ => return Err(bad(CMD, "sheet must be A0, A1, A2, A3 or A4")),
    };
    let (pw, ph) = sheet_size(&sheet_name).ok_or_else(|| bad(CMD, "sheet must be A0, A1, A2, A3 or A4"))?;
    let orientation = choice(p, "orientation", if sheet_name == "A4" { "portrait" } else { "landscape" }, &["landscape", "portrait"])?;
    let paper = if orientation == "landscape" { (ph, pw) } else { (pw, ph) };
    let margins = choice(p, "margins", "tcvn", &["tcvn", "compact"])?;
    let other = if margins == "tcvn" { 10.0 } else { 5.0 };
    let frame = Rect { x0: 20.0, y0: other, x1: paper.0 - other, y1: paper.1 - other };
    let block = match choice(p, "title_block", "production", &["production", "school", "none"])? {
        "school" => Block::School,
        "none" => Block::None,
        _ => Block::Production,
    };
    let (bw, bh) = block.size();
    if bw > frame.w() + 1e-9 {
        return Err(bad(CMD, format!("the {bw:.0} mm title block is wider than this sheet's frame; use the school block or a larger sheet")));
    }
    let first_angle = choice(p, "projection", "first_angle", &["first_angle", "third_angle"])? == "first_angle";
    let dashed = choice(p, "hidden", "dashed", &["dashed", "omit"])? == "dashed";
    let tolerance = match p.get("tolerance") {
        None | Some(Value::Null) => 0.01,
        Some(v) => v.as_f64().filter(|t| (0.0001..=1.0).contains(t)).ok_or_else(|| bad(CMD, "tolerance must be 0.0001..1 mm"))?,
    };
    let views: Vec<StandardView> = match p.get("views") {
        None | Some(Value::Null) => vec![StandardView::Front, StandardView::Top, StandardView::Left],
        Some(Value::Array(list)) if !list.is_empty() && list.len() <= 7 => {
            let mut out = Vec::new();
            for v in list {
                let view =
                    v.as_str().and_then(StandardView::parse).ok_or_else(|| bad(CMD, "views are front, back, left, right, top, bottom or iso"))?;
                if out.contains(&view) {
                    return Err(bad(CMD, "a view is named twice"));
                }
                out.push(view);
            }
            out
        }
        _ => return Err(bad(CMD, "views must list 1 to 7 views")),
    };
    let wanted_scale = match p.get("scale") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().and_then(Scale::parse).ok_or_else(|| bad(CMD, "scale must be a ratio like \"1:2\" or \"2:1\""))?),
    };
    let f = fields(p)?;
    let path = match p.get("path") {
        None | Some(Value::Null) => None,
        Some(_) => Some(str_(p, "path").filter(|v| !v.trim().is_empty() && v.len() < 4096).ok_or_else(|| bad(CMD, "invalid path"))?),
    };
    if let Some(path) = path {
        solidcraft_io::vfs::check(path).map_err(|e| crate::EngineError::Other(e.to_string()))?;
    }

    // The drawing area: inside the frame, above the title block and below the code box.
    let top = if block == Block::Production { frame.y1 - 14.0 } else { frame.y1 };
    let area = Rect { x0: frame.x0, y0: frame.y0 + bh, x1: frame.x1, y1: top };

    let drawn = super::view_dxf::standard_views(s, CMD, &views, tolerance, dashed)?;
    let mut laid = Vec::new();
    for (view, entities) in views.iter().zip(&drawn) {
        let (lo, hi) = extent(entities).unwrap_or((Vec2::ZERO, Vec2::ZERO));
        laid.push(Laid { cell: grid_cell(*view, first_angle), iso: *view == StandardView::Iso, lo, hi });
    }
    let (cols, rows) = (bands(&laid, true), bands(&laid, false));
    let most = fit_factor(&cols, area.w()).min(fit_factor(&rows, area.h()));
    let scale = match wanted_scale {
        Some(sc) if sc.factor() <= most * (1.0 + 1e-9) => sc,
        Some(sc) => {
            let hint = Scale::fitting(most).map_or_else(String::new, |f| format!(" ({} fits)", f.label()));
            return Err(bad(CMD, format!("at {} the views don't fit the {sheet_name} sheet{hint}", sc.label())));
        }
        None => Scale::fitting(most).ok_or_else(|| bad(CMD, format!("the views don't fit the {sheet_name} sheet at any standard scale")))?,
    };
    let k = scale.factor();
    let cx = centres(&cols, area.x0, area.w(), k, false);
    let cy = centres(&rows, area.y0, area.h(), k, true);

    let mut entities = Vec::new();
    sheet_lines(paper, frame, &mut entities);
    title_block(block, frame, &f, &scale.label(), &mut entities);
    let mut placed = Vec::new();
    for ((view, list), l) in views.iter().zip(drawn).zip(&laid) {
        let (c, r) = l.cell;
        let mid = if l.iso {
            Vec2::new((l.lo.x + l.hi.x) / 2.0, (l.lo.y + l.hi.y) / 2.0)
        } else {
            Vec2::new(cols.mid.get(c).copied().unwrap_or(0.0), rows.mid.get(r).copied().unwrap_or(0.0))
        };
        let centre = Vec2::new(cx.get(c).copied().unwrap_or(0.0), cy.get(r).copied().unwrap_or(0.0));
        let at = Vec2::new(centre.x - k * mid.x, centre.y - k * mid.y);
        let count = list.len();
        entities.extend(list.into_iter().filter_map(|e| place(e, k, at)));
        placed.push(json!({
            "view": format!("{view:?}").to_lowercase(),
            "centre_mm": [at.x + k * (l.lo.x + l.hi.x) / 2.0, at.y + k * (l.lo.y + l.hi.y) / 2.0],
            "size_mm": [k * (l.hi.x - l.lo.x), k * (l.hi.y - l.lo.y)],
            "entities": count,
        }));
    }

    let layers = [
        DxfLayer::new("SHEET", 8).weight(25),
        DxfLayer::new("FRAME", 7).weight(50),
        DxfLayer::new("TITLE_LINES", 7).weight(25),
        DxfLayer::new("TITLE_TEXT", 7).weight(25),
        DxfLayer::new("VISIBLE", 7).weight(50),
        DxfLayer { linetype: DxfLinetype::Hidden, ..DxfLayer::new("HIDDEN", 8).weight(25) },
    ];
    let text = solidcraft_io::write_dxf_drawing(&layers, &entities);
    let mut result = json!({
        "standard": "TCVN",
        "sheet": sheet_name,
        "orientation": orientation,
        "size_mm": [paper.0, paper.1],
        "frame_mm": {"min": [frame.x0, frame.y0], "max": [frame.x1, frame.y1]},
        "title_block": match block { Block::Production => "production", Block::School => "school", Block::None => "none" },
        "projection": if first_angle { "first_angle" } else { "third_angle" },
        "scale": scale.label(),
        "views": placed,
        "units": "mm",
        "unused_fields": unused(block, &f),
    });
    if let Some(path) = path {
        solidcraft_io::vfs::check(path).map_err(|e| crate::EngineError::Other(e.to_string()))?;
        solidcraft_io::write_atomic(std::path::Path::new(path), text.as_bytes(), false).map_err(|e| crate::EngineError::Other(e.to_string()))?;
        result["path"] = json!(path);
    } else {
        result["dxf"] = json!(text);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "drawing_sheet_tests.rs"]
mod tests;
