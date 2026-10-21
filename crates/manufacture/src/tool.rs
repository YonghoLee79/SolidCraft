//! Cutting tools.

use serde::{Deserialize, Serialize};

use crate::error::{Result, finite, invalid};

/// Largest tool diameter the library accepts (mm).
pub const MAX_TOOL_DIAMETER: f64 = 500.0;
/// Longest tool name (bytes).
pub const MAX_NAME_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    #[default]
    FlatEndMill,
    BallEndMill,
    BullNoseEndMill,
    Drill,
}

impl ToolKind {
    /// A tool that drills along its axis only.
    pub fn is_drill(self) -> bool {
        matches!(self, ToolKind::Drill)
    }
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::FlatEndMill => "flat end mill",
            ToolKind::BallEndMill => "ball end mill",
            ToolKind::BullNoseEndMill => "bull nose end mill",
            ToolKind::Drill => "drill",
        }
    }
}

/// A tool in the library. `id` is the tool number the controller calls (`T<id>`, length offset
/// `H<id>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub kind: ToolKind,
    /// Cutting diameter (mm).
    pub diameter: f64,
    #[serde(default = "default_flutes")]
    pub flutes: u32,
    /// Corner radius of a bull nose mill (mm).
    #[serde(default)]
    pub corner_radius: f64,
    /// Point angle of a drill (degrees).
    #[serde(default = "default_tip_angle")]
    pub tip_angle: f64,
}

fn default_flutes() -> u32 {
    2
}
fn default_tip_angle() -> f64 {
    118.0
}

impl Tool {
    pub fn radius(&self) -> f64 {
        self.diameter * 0.5
    }

    pub fn validate(&self) -> Result<()> {
        if self.id == 0 {
            return invalid("a tool number is 1 or more");
        }
        if self.name.len() > MAX_NAME_BYTES {
            return invalid(format!("a tool name is at most {MAX_NAME_BYTES} bytes"));
        }
        finite(self.diameter, "the tool diameter")?;
        if !(self.diameter > 0.0 && self.diameter <= MAX_TOOL_DIAMETER) {
            return invalid(format!("the tool diameter must be above 0 and at most {MAX_TOOL_DIAMETER} mm"));
        }
        if !(1..=16).contains(&self.flutes) {
            return invalid("a tool has 1 to 16 flutes");
        }
        finite(self.corner_radius, "the corner radius")?;
        if self.corner_radius < 0.0 || self.corner_radius > self.radius() {
            return invalid("the corner radius is between 0 and half the diameter");
        }
        finite(self.tip_angle, "the point angle")?;
        if !(self.tip_angle > 0.0 && self.tip_angle < 180.0) {
            return invalid("the point angle is between 0 and 180 degrees");
        }
        Ok(())
    }
}
