//! Drawing sheets: paper sizes, orientation and the drawing border, as data in paper millimetres.
//!
//! A [`Sheet`] is a paper size and an orientation; [`Sheet::paper_mm`] gives the paper as it lies.
//! A [`BorderProfile`] is the rule for the border inside the paper: its margins, the frame's line
//! width and the grid of reference fields with their labels. [`BorderProfile::layout`] works the
//! border out for one sheet, so a new size regrows it. Nothing here is frozen geometry, saved in a
//! design or drawn: a writer draws the [`BorderLayout`] later.
//!
//! Paper coordinates are millimetres from the paper's lower-left corner, x to the right and y up.
//! The frame's line is centred on [`BorderLayout::frame`].
//!
//! [`BorderProfile::iso`] is ISO 5457's border: a 20 mm filing margin on the left and 10 mm on the
//! other sides, a 0.7 mm frame, and 50 mm fields counted out from the paper's centre lines, the
//! corner fields taking what is left. Rows are lettered from the top without I and O, columns
//! numbered from the left, and an A4 sheet is labelled on its top and right sides only.
//!
//! Sizes: ISO 216 A0 to A5, Letter and Legal, ANSI A to F (ASME Y14.1), the architectural A to E
//! and E1, and custom sizes, finite, above 0 and at most [`MAX_SIDE_MM`] a side. Centring and
//! trimming marks, title blocks and the elongated ISO sizes are not here yet.

use serde::{Deserialize, Serialize};

use crate::{Aabb2, Vec2};

/// The longest side a custom sheet may have, in millimetres.
pub const MAX_SIDE_MM: f64 = 5000.0;
/// The most fields an equal division may cut each way.
pub const MAX_EQUAL_FIELDS: u32 = 200;
const MIN_FIELD_MM: f64 = 1.0;
const MAX_FIELD_MM: f64 = 1000.0;
const MAX_FRAME_LINE_MM: f64 = 10.0;
/// The smallest drawing space the margins may leave, each way.
const MIN_DRAWING_SPACE_MM: f64 = 1.0;
/// A corner field shorter than this joins its neighbour, so rounding never leaves a sliver.
const SLIVER_MM: f64 = 1e-9;
const INCH: f64 = 25.4;

/// A standard paper size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperSize {
    A0,
    A1,
    A2,
    A3,
    A4,
    A5,
    Letter,
    Legal,
    AnsiA,
    AnsiB,
    AnsiC,
    AnsiD,
    AnsiE,
    AnsiF,
    ArchA,
    ArchB,
    ArchC,
    ArchD,
    ArchE,
    ArchE1,
}

impl PaperSize {
    pub const ALL: [PaperSize; 20] = {
        use PaperSize::*;
        [A0, A1, A2, A3, A4, A5, Letter, Legal, AnsiA, AnsiB, AnsiC, AnsiD, AnsiE, AnsiF, ArchA, ArchB, ArchC, ArchD, ArchE, ArchE1]
    };

    /// The short and the long side, in millimetres (the US sizes from their inches).
    pub fn sides_mm(self) -> (f64, f64) {
        use PaperSize::*;
        let inches = |short: f64, long: f64| (short * INCH, long * INCH);
        match self {
            A0 => (841.0, 1189.0),
            A1 => (594.0, 841.0),
            A2 => (420.0, 594.0),
            A3 => (297.0, 420.0),
            A4 => (210.0, 297.0),
            A5 => (148.0, 210.0),
            Letter | AnsiA => inches(8.5, 11.0),
            Legal => inches(8.5, 14.0),
            AnsiB => inches(11.0, 17.0),
            AnsiC => inches(17.0, 22.0),
            AnsiD => inches(22.0, 34.0),
            AnsiE => inches(34.0, 44.0),
            AnsiF => inches(28.0, 40.0),
            ArchA => inches(9.0, 12.0),
            ArchB => inches(12.0, 18.0),
            ArchC => inches(18.0, 24.0),
            ArchD => inches(24.0, 36.0),
            ArchE => inches(36.0, 48.0),
            ArchE1 => inches(30.0, 42.0),
        }
    }
}

/// A sheet's size: a standard one or any other.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SheetSize {
    Standard(PaperSize),
    /// The two sides in either order; the orientation decides which lies across.
    Custom {
        sides_mm: [f64; 2],
    },
}

/// Which way the paper lies. Landscape puts the long side across.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Landscape,
    Portrait,
}

/// One sheet of paper: its size and the way it lies (landscape when not given).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub size: SheetSize,
    #[serde(default)]
    pub orientation: Orientation,
}

/// The distances from the paper's edges to the frame, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Margins {
    pub left_mm: f64,
    pub right_mm: f64,
    pub top_mm: f64,
    pub bottom_mm: f64,
}

/// How the frame is cut into reference fields.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case", deny_unknown_fields)]
pub enum GridRule {
    /// No reference fields.
    None,
    /// Fields of `field_mm` counted out from the paper's centre lines, the corner fields taking
    /// what is left (ISO 5457, with 50 mm). Each way the larger half sets the count `n` (its
    /// length over `field_mm`, rounded, at least 1), and each half has `n` fields, or fewer where
    /// it is shorter.
    FromCentre { field_mm: f64 },
    /// The frame cut into equal fields.
    Equal { columns: u32, rows: u32 },
}

/// Which sides of the frame carry the field labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sides {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Sides {
    pub const ALL: Sides = Sides { top: true, bottom: true, left: true, right: true };
}

/// The rule for a drawing border: worked out for each sheet by [`BorderProfile::layout`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorderProfile {
    pub margins: Margins,
    pub frame_line_mm: f64,
    pub grid: GridRule,
    /// Row letters skip I and O (they read as 1 and 0).
    pub skip_i_and_o: bool,
    pub label_sides: Sides,
}

/// One reference field along its axis: `from_mm < to_mm` in paper coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub from_mm: f64,
    pub to_mm: f64,
    pub label: String,
}

/// A sheet's border, worked out: what a writer draws.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderLayout {
    pub paper: Aabb2,
    pub frame: Aabb2,
    pub frame_line_mm: f64,
    /// Left to right, numbered from 1.
    pub columns: Vec<Field>,
    /// Top to bottom, lettered from A.
    pub rows: Vec<Field>,
    pub label_sides: Sides,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("drawing sheet: {0}")]
pub struct Invalid(pub &'static str);

type Result<T> = std::result::Result<T, Invalid>;

fn require(condition: bool, message: &'static str) -> Result<()> {
    if condition { Ok(()) } else { Err(Invalid(message)) }
}

impl Sheet {
    pub fn standard(size: PaperSize, orientation: Orientation) -> Sheet {
        Sheet { size: SheetSize::Standard(size), orientation }
    }

    /// The paper's width and height as it lies.
    pub fn paper_mm(&self) -> Result<Vec2> {
        let (short, long) = match self.size {
            SheetSize::Standard(size) => size.sides_mm(),
            SheetSize::Custom { sides_mm: [a, b] } => {
                for side in [a, b] {
                    require(side.is_finite() && side > 0.0 && side <= MAX_SIDE_MM, "a custom side is finite, above 0 and at most 5000 mm")?;
                }
                (a.min(b), a.max(b))
            }
        };
        Ok(match self.orientation {
            Orientation::Landscape => Vec2::new(long, short),
            Orientation::Portrait => Vec2::new(short, long),
        })
    }
}

impl BorderProfile {
    /// ISO 5457's border for `sheet`: only an A4 sheet's labels differ.
    pub fn iso(sheet: &Sheet) -> BorderProfile {
        let a4 = sheet.size == SheetSize::Standard(PaperSize::A4);
        BorderProfile {
            margins: Margins { left_mm: 20.0, right_mm: 10.0, top_mm: 10.0, bottom_mm: 10.0 },
            frame_line_mm: 0.7,
            grid: GridRule::FromCentre { field_mm: 50.0 },
            skip_i_and_o: true,
            label_sides: if a4 { Sides { top: true, bottom: false, left: false, right: true } } else { Sides::ALL },
        }
    }

    /// The border on `sheet`.
    pub fn layout(&self, sheet: &Sheet) -> Result<BorderLayout> {
        let paper = sheet.paper_mm()?;
        let m = self.margins;
        for margin in [m.left_mm, m.right_mm, m.top_mm, m.bottom_mm] {
            require(margin.is_finite() && (0.0..=MAX_SIDE_MM).contains(&margin), "a margin is finite and 0 to 5000 mm")?;
        }
        require(
            self.frame_line_mm.is_finite() && self.frame_line_mm > 0.0 && self.frame_line_mm <= MAX_FRAME_LINE_MM,
            "the frame's line is above 0 and at most 10 mm wide",
        )?;
        let frame = Aabb2 { min: Vec2::new(m.left_mm, m.bottom_mm), max: Vec2::new(paper.x - m.right_mm, paper.y - m.top_mm) };
        require(
            frame.max.x - frame.min.x >= MIN_DRAWING_SPACE_MM && frame.max.y - frame.min.y >= MIN_DRAWING_SPACE_MM,
            "the margins leave no drawing space",
        )?;
        let (across, down) = match self.grid {
            GridRule::None => (vec![], vec![]),
            GridRule::FromCentre { field_mm } => {
                require(field_mm.is_finite() && (MIN_FIELD_MM..=MAX_FIELD_MM).contains(&field_mm), "a field is 1 to 1000 mm long")?;
                (from_centre(frame.min.x, frame.max.x, paper.x / 2.0, field_mm)?, from_centre(frame.min.y, frame.max.y, paper.y / 2.0, field_mm)?)
            }
            GridRule::Equal { columns, rows } => {
                let counts = 1..=MAX_EQUAL_FIELDS;
                require(counts.contains(&columns) && counts.contains(&rows), "an equal division has 1 to 200 fields each way")?;
                (equal(frame.min.x, frame.max.x, columns), equal(frame.min.y, frame.max.y, rows))
            }
        };
        let columns = fields(&across, |i| (i + 1).to_string());
        // Rows run top to bottom: the edges are in rising y, so the top row comes last.
        let mut rows = fields(&down, |_| String::new());
        rows.reverse();
        for (i, row) in rows.iter_mut().enumerate() {
            row.label = letters(i, self.skip_i_and_o);
        }
        Ok(BorderLayout {
            paper: Aabb2 { min: Vec2::ZERO, max: paper },
            frame,
            frame_line_mm: self.frame_line_mm,
            columns,
            rows,
            label_sides: self.label_sides,
        })
    }
}

/// The field edges from `lo` to `hi`, in rising order, for fields of `field` out from `centre`.
fn from_centre(lo: f64, hi: f64, centre: f64, field: f64) -> Result<Vec<f64>> {
    let (below, above) = (centre - lo, hi - centre);
    require(below > SLIVER_MM && above > SLIVER_MM, "the paper's centre lines cross the frame")?;
    // At most MAX_SIDE_MM / MIN_FIELD_MM per half: a bounded count.
    let n = (below.max(above) / field).round().max(1.0);
    let count = |half: f64| n.min(((half - SLIVER_MM) / field).ceil()) as usize;
    let (k_below, k_above) = (count(below), count(above));
    let mut edges = Vec::with_capacity(k_below + k_above + 1);
    edges.push(lo);
    edges.extend((1..k_below).rev().map(|j| centre - j as f64 * field));
    edges.push(centre);
    edges.extend((1..k_above).map(|j| centre + j as f64 * field));
    edges.push(hi);
    Ok(edges)
}

/// The edges of `n` equal fields from `lo` to `hi`, ending exactly at `hi`.
fn equal(lo: f64, hi: f64, n: u32) -> Vec<f64> {
    let step = (hi - lo) / f64::from(n);
    (0..n).map(|i| lo + f64::from(i) * step).chain(std::iter::once(hi)).collect()
}

fn fields(edges: &[f64], label: impl Fn(usize) -> String) -> Vec<Field> {
    edges
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| match *w {
            [from_mm, to_mm] => Some(Field { from_mm, to_mm, label: label(i) }),
            _ => None,
        })
        .collect()
}

/// The row label for row `i` (0 is A): A to Z, then AA, AB and on (without I and O if asked).
pub fn letters(i: usize, skip_i_and_o: bool) -> String {
    const ALL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const NO_I_O: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    let alphabet = if skip_i_and_o { NO_I_O } else { ALL };
    let base = alphabet.len();
    let mut out = Vec::new();
    // Bijective numbering: after the last letter comes AA, not BA.
    let mut n = i.saturating_add(1);
    while n > 0 {
        n -= 1;
        out.extend(alphabet.get(n % base).map(|&b| char::from(b)));
        n /= base;
    }
    out.iter().rev().collect()
}

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;
