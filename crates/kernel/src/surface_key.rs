//! Bounded full-bit spline support keys shared by the sampling caches.
use truck_modeling as mt;

// Keys retain every geometry bit, including signed zero, weights and NaN payloads.
// At most 1,024 words (8 KiB) per support key; larger supports keep the original search.
const MAX_KEY_WORDS: usize = 1024;
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct SurfaceKey(Vec<u64>);
impl SurfaceKey {
    pub(super) fn new(surface: &mt::Surface) -> Option<Self> {
        fn push(out: &mut Vec<u64>, word: u64) -> Option<()> {
            if out.len() >= MAX_KEY_WORDS {
                return None;
            }
            out.push(word);
            Some(())
        }
        fn knots(out: &mut Vec<u64>, values: &mt::KnotVec) -> Option<()> {
            push(out, values.len() as u64)?;
            for &v in values.iter() {
                push(out, v.to_bits())?;
            }
            Some(())
        }
        let mut out = Vec::new();
        match surface {
            mt::Surface::BSplineSurface(s) => {
                push(&mut out, 0)?;
                knots(&mut out, s.uknot_vec())?;
                knots(&mut out, s.vknot_vec())?;
                push(&mut out, s.control_points().len() as u64)?;
                for row in s.control_points() {
                    push(&mut out, row.len() as u64)?;
                    for p in row {
                        for value in [p.x, p.y, p.z] {
                            push(&mut out, value.to_bits())?;
                        }
                    }
                }
            }
            mt::Surface::NurbsSurface(s) => {
                push(&mut out, 1)?;
                knots(&mut out, s.uknot_vec())?;
                knots(&mut out, s.vknot_vec())?;
                push(&mut out, s.control_points().len() as u64)?;
                for row in s.control_points() {
                    push(&mut out, row.len() as u64)?;
                    for p in row {
                        for value in [p.x, p.y, p.z, p.w] {
                            push(&mut out, value.to_bits())?;
                        }
                    }
                }
            }
            _ => return None,
        }
        Some(Self(out))
    }
}
