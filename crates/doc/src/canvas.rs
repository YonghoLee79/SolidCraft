//! Canvases: reference images placed on a plane (Insert Canvas), kept in the design.
//! The image bytes travel inside the design file (base64), so it reopens without the original.

use serde::{Deserialize, Serialize};
use solidcraft_geom::{Plane, Vec2, Vec3};

use crate::PlaneRef;

/// Largest image a canvas may hold (bytes).
pub const MAX_CANVAS_BYTES: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub id: u64,
    pub name: String,
    /// `png` or `jpeg`.
    pub format: String,
    /// The image file, base64.
    pub data: String,
    /// Pixel size of the image.
    pub pixels: [u32; 2],
    pub plane: PlaneRef,
    /// Centre of the image on the plane (plane coordinates, mm).
    pub center: Vec2,
    /// Width on the plane before the sideways stretch (mm); the height follows the image's
    /// aspect. The width shown is `width * scale_x` (see `size`).
    pub width: f64,
    /// Sideways stretch of the shown width.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale_x: f64,
    /// Lengthways stretch of the shown height.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale_y: f64,
    /// Rotation on the plane (radians).
    #[serde(default)]
    pub angle: f64,
    #[serde(default = "half")]
    pub opacity: f64,
    /// Mirrored left to right.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    /// Mirrored top to bottom.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_v: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    /// The component it belongs to (its plane is in that component's frame, so it moves with
    /// the component's occurrence); 0 = the root.
    #[serde(default, skip_serializing_if = "is_root")]
    pub component: u64,
}

fn is_root(c: &u64) -> bool {
    *c == 0
}

fn is_one(v: &f64) -> bool {
    *v == 1.0
}

fn one() -> f64 {
    1.0
}

fn half() -> f64 {
    0.5
}
fn yes() -> bool {
    true
}

impl Canvas {
    /// Shown width and height on the plane (mm), with the stretches applied.
    pub fn size(&self) -> Vec2 {
        let [w, h] = self.pixels;
        let shown = self.width * self.scale_x;
        if w == 0 { Vec2::new(shown, shown) } else { Vec2::new(shown, self.width * f64::from(h) / f64::from(w) * self.scale_y) }
    }
    pub fn height(&self) -> f64 {
        self.size().y
    }
    /// The four corners on the plane (plane coordinates): bottom-left, bottom-right, top-right,
    /// top-left of the image; the flips swap them about the centre.
    pub fn corners(&self) -> [Vec2; 4] {
        let size = self.size();
        let (hw, hh) = (size.x / 2.0, size.y / 2.0);
        let (s, c) = self.angle.sin_cos();
        let rot = |x: f64, y: f64| self.center + Vec2::new(x * c - y * s, x * s + y * c);
        let f = if self.flip { -1.0 } else { 1.0 };
        let g = if self.flip_v { -1.0 } else { 1.0 };
        [rot(-hw * f, -hh * g), rot(hw * f, -hh * g), rot(hw * f, hh * g), rot(-hw * f, hh * g)]
    }
    /// Corners on `plane` (in whatever frame the plane is given).
    pub fn world_corners(&self, plane: &Plane) -> [Vec3; 4] {
        self.corners().map(|p| plane.to_world(p))
    }
    pub fn bytes(&self) -> Option<Vec<u8>> {
        base64_decode(&self.data)
    }
}

impl crate::Document {
    /// A canvas's plane in the world: resolved in its component's frame, then placed where the
    /// component's occurrence puts it.
    pub fn canvas_plane(&self, vals: &std::collections::BTreeMap<String, crate::expr::Value>, c: &Canvas) -> crate::Result<Plane> {
        let p = self.resolve_plane(vals, &c.plane, 0)?;
        let m = self.component_transform(c.component);
        if crate::is_identity(&m) {
            return Ok(p);
        }
        Ok(Plane::new(crate::apply_point(&m, p.origin), crate::apply_vector(&m, p.x), crate::apply_vector(&m, p.y)).unwrap_or(p))
    }
}

/// Image kind and pixel size from the file header (PNG or JPEG).
pub fn image_info(b: &[u8]) -> Option<(&'static str, [u32; 2])> {
    if b.len() > 24 && b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes([*b.get(16)?, *b.get(17)?, *b.get(18)?, *b.get(19)?]);
        let h = u32::from_be_bytes([*b.get(20)?, *b.get(21)?, *b.get(22)?, *b.get(23)?]);
        return (w > 0 && h > 0).then_some(("png", [w, h]));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        // Walk the markers to a start-of-frame.
        let mut i = 2;
        while i + 9 < b.len() {
            if *b.get(i)? != 0xFF {
                i += 1;
                continue;
            }
            let m = *b.get(i + 1)?;
            if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) || m == 0xFF {
                i += if m == 0xFF { 1 } else { 2 };
                continue;
            }
            let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
            if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let h = u32::from(u16::from_be_bytes([*b.get(i + 5)?, *b.get(i + 6)?]));
                let w = u32::from(u16::from_be_bytes([*b.get(i + 7)?, *b.get(i + 8)?]));
                return (w > 0 && h > 0).then_some(("jpeg", [w, h]));
            }
            i += 2 + len;
        }
    }
    None
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(char::from(B64[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canvas as saved before the stretches and the top-to-bottom flip.
    fn old_canvas_json(width: f64) -> serde_json::Value {
        serde_json::json!({
            "id": 1, "name": "Canvas1", "format": "png", "data": "", "pixels": [400, 200],
            "plane": {"type": "origin", "name": "XY"}, "center": [0.0, 0.0], "width": width,
            "angle": 0.0, "opacity": 0.5, "visible": true
        })
    }

    #[test]
    fn an_older_canvas_loads_unstretched_and_saves_as_before() {
        let c: Canvas = serde_json::from_value(old_canvas_json(80.0)).unwrap();
        assert_eq!((c.scale_x, c.scale_y, c.flip_v), (1.0, 1.0, false));
        assert_eq!(c.size(), Vec2::new(80.0, 40.0));
        // Defaults are not written back, so an untouched canvas saves exactly as before.
        assert_eq!(serde_json::to_value(&c).unwrap(), old_canvas_json(80.0));
    }

    #[test]
    fn size_follows_the_stretches_and_flips_mirror_the_corners() {
        let mut c: Canvas = serde_json::from_value(old_canvas_json(100.0)).unwrap();
        c.scale_x = 1.5;
        c.scale_y = 0.5;
        assert_eq!(c.size(), Vec2::new(150.0, 25.0));
        assert_eq!(c.height(), 25.0);
        c.center = Vec2::new(10.0, 0.0);
        let [bl, ..] = c.corners();
        assert_eq!(bl, Vec2::new(-65.0, -12.5));
        c.flip = true;
        assert_eq!(c.corners()[0], Vec2::new(85.0, -12.5));
        c.flip_v = true;
        assert_eq!(c.corners()[0], Vec2::new(85.0, 12.5));
    }

    #[test]
    fn base64_round_trip_and_headers() {
        for n in 0..10 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&b)).unwrap(), b);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        assert_eq!(image_info(&png), Some(("png", [640, 480])));
        let jpg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0, 100, 0, 200, 3, 0, 0, 0];
        assert_eq!(image_info(&jpg), Some(("jpeg", [200, 100])));
        assert_eq!(image_info(b"nope"), None);
    }
}
