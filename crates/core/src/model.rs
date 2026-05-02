use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod backend;
mod identity;

pub use backend::{BackendKind, Capabilities};
pub use identity::{identities_match, normalized_identity_value, OutputIdentity};

pub const MAX_TOPOLOGY_OUTPUTS: usize = 64;
pub const MAX_OUTPUT_MODES: usize = 256;
pub const MAX_TOPOLOGY_STRING_BYTES: usize = 512;
pub const MAX_MODE_DIMENSION: u32 = 32768;
pub const MAX_REFRESH_MILLIHZ: u32 = 2_000_000;
pub const MAX_ABS_POSITION: i32 = 1_000_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum VirtualPreset {
    Off,
    External,
    Builtin,
    Common,
    Largest,
    Mirror,
    Horizontal,
    HorizontalReverse,
    Vertical,
    VerticalReverse,
}

impl VirtualPreset {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::External => "external",
            Self::Builtin => "builtin",
            Self::Common => "common",
            Self::Largest => "largest",
            Self::Mirror => "mirror",
            Self::Horizontal => "horizontal",
            Self::HorizontalReverse => "horizontal-reverse",
            Self::Vertical => "vertical",
            Self::VerticalReverse => "vertical-reverse",
        }
    }

    #[must_use]
    pub const fn is_reverse(self) -> bool {
        matches!(self, Self::HorizontalReverse | Self::VerticalReverse)
    }
}

impl std::fmt::Display for VirtualPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Topology {
    pub outputs: HashMap<String, OutputState>,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            identity: OutputIdentity::default(),
            enabled: false,
            mode: None,
            available_modes: Vec::new(),
            position: Position::default(),
            scale: 1.0,
            scaled_resolution: None,
            transform: Transform::default(),
            mirror_target: None,
            backend_data: None,
        }
    }
}

impl Topology {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut parts: Vec<String> = self
            .outputs
            .keys()
            .map(|k| {
                let o = &self.outputs[k];
                let enabled = if o.enabled { "on" } else { "off" };
                format!("{k}:{enabled}")
            })
            .collect();
        parts.sort();
        parts.join(";")
    }

    #[must_use]
    pub fn setup_fingerprint(&self) -> String {
        let mut parts: Vec<String> = self
            .outputs
            .values()
            .filter(|output| !output.identity.is_ignored && !output.identity.is_virtual)
            .map(|output| output.identity.primary_key())
            .collect();
        parts.sort();
        parts.join(";")
    }

    #[must_use]
    pub fn state_fingerprint(&self) -> String {
        let mut parts: Vec<String> = self
            .outputs
            .iter()
            .map(|(name, output)| {
                let fingerprint = output.fingerprint();
                let x = output.position.x;
                let y = output.position.y;
                format!("{name}:{fingerprint}:{x}:{y}")
            })
            .collect();
        parts.sort();
        parts.join(";")
    }

    #[must_use]
    pub fn has_enabled_real_outputs(&self) -> bool {
        self.outputs.values().any(|output| {
            output.enabled && !output.identity.is_ignored && !output.identity.is_virtual
        })
    }

    #[must_use]
    pub fn has_strong_setup_identity(&self) -> bool {
        self.outputs
            .values()
            .filter(|output| !output.identity.is_ignored && !output.identity.is_virtual)
            .all(|output| output.identity.has_non_connector_identity())
            && self
                .outputs
                .values()
                .any(|output| !output.identity.is_ignored && !output.identity.is_virtual)
    }

    pub fn validate_limits(&self) -> Result<(), String> {
        if self.outputs.len() > MAX_TOPOLOGY_OUTPUTS {
            return Err(format!(
                "{} outputs reported, limit is {}",
                self.outputs.len(),
                MAX_TOPOLOGY_OUTPUTS
            ));
        }

        for (name, output) in &self.outputs {
            validate_string("output name", name)?;
            output.validate_limits(name)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct OutputState {
    pub identity: OutputIdentity,
    pub enabled: bool,
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_modes: Vec<Mode>,
    pub position: Position,
    pub scale: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaled_resolution: Option<Resolution>,
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_target: Option<String>,
    pub backend_data: Option<serde_json::Value>,
}

impl OutputState {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.identity.connector = Some(name.into());
        state
    }

    #[must_use]
    pub fn same_layout_as(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.mode == other.mode
            && self.position == other.position
            && self.scale.partial_cmp(&other.scale) == Some(std::cmp::Ordering::Equal)
            && self.transform == other.transform
            && self.mirror_target == other.mirror_target
    }

    pub fn refresh_scaled_resolution(&mut self) {
        self.scaled_resolution = self.mode.and_then(|mode| {
            Resolution::from_mode_scale_transform(mode, self.scale, self.transform)
        });
    }

    #[must_use]
    pub fn with_refreshed_scaled_resolution(mut self) -> Self {
        self.refresh_scaled_resolution();
        self
    }

    #[must_use]
    pub fn layout_resolution(&self) -> Option<Resolution> {
        self.mode
            .and_then(|mode| {
                Resolution::from_mode_scale_transform(mode, self.scale, self.transform)
            })
            .or(self.scaled_resolution)
    }

    #[must_use]
    pub fn fingerprint(&self) -> String {
        let key = self.identity.primary_key();
        let enabled = if self.enabled { "on" } else { "off" };
        let width = self
            .mode
            .as_ref()
            .map_or_else(String::new, |m| m.width.to_string());
        let height = self
            .mode
            .as_ref()
            .map_or_else(String::new, |m| m.height.to_string());
        let refresh = self
            .mode
            .as_ref()
            .map_or_else(String::new, |m| m.refresh.to_string());
        let scale = self.scale;
        let scaled_resolution = self
            .layout_resolution()
            .map_or_else(String::new, |resolution| {
                format!("{}x{}", resolution.width, resolution.height)
            });
        let transform = self.transform;
        format!(
            "{key}:{enabled}:{width}x{height}@{refresh}:{scale}:{scaled_resolution}:{transform}"
        )
    }

    pub fn validate_limits(&self, name: &str) -> Result<(), String> {
        self.identity.validate_limits(name)?;
        if self.available_modes.len() > MAX_OUTPUT_MODES {
            return Err(format!(
                "output {name} reported {} modes, limit is {}",
                self.available_modes.len(),
                MAX_OUTPUT_MODES
            ));
        }
        for mode in self.available_modes.iter().chain(self.mode.iter()) {
            mode.validate_limits(name)?;
        }
        if !self.scale.is_finite() || !(0.05..=100.0).contains(&self.scale) {
            return Err(format!(
                "output {name} reported invalid scale {}",
                self.scale
            ));
        }
        if self.position.x.unsigned_abs() > MAX_ABS_POSITION as u32
            || self.position.y.unsigned_abs() > MAX_ABS_POSITION as u32
        {
            return Err(format!("output {name} reported out-of-range position"));
        }
        if let Some(target) = &self.mirror_target {
            validate_string("mirror target", target)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl Position {
    #[must_use]
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh: u32,
}

impl Mode {
    #[must_use]
    pub fn new(width: u32, height: u32, refresh: u32) -> Self {
        Self {
            width,
            height,
            refresh,
        }
    }

    pub fn validate_limits(&self, name: &str) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_MODE_DIMENSION
            || self.height > MAX_MODE_DIMENSION
            || self.refresh > MAX_REFRESH_MILLIHZ
        {
            return Err(format!(
                "output {name} reported invalid mode {}x{}@{}",
                self.width, self.height, self.refresh
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub fn from_mode_scale(mode: Mode, scale: f64) -> Option<Self> {
        Self::from_mode_scale_transform(mode, scale, Transform::Normal)
    }

    #[must_use]
    pub fn from_mode_scale_transform(mode: Mode, scale: f64, transform: Transform) -> Option<Self> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }

        let resolution = Self {
            width: scaled_dimension(mode.width, scale),
            height: scaled_dimension(mode.height, scale),
        };

        Some(if transform.swaps_axes() {
            Self {
                width: resolution.height,
                height: resolution.width,
            }
        } else {
            resolution
        })
    }
}

fn scaled_dimension(value: u32, scale: f64) -> u32 {
    if value == 0 {
        return 0;
    }

    let scaled = (f64::from(value) / scale).round();
    if scaled < 1.0 {
        1
    } else if scaled > f64::from(u32::MAX) {
        u32::MAX
    } else {
        scaled as u32
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transform {
    #[default]
    Normal,
    Rot90,
    Rot180,
    Rot270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl std::fmt::Display for Transform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Transform::Normal => write!(f, "normal"),
            Transform::Rot90 => write!(f, "90"),
            Transform::Rot180 => write!(f, "180"),
            Transform::Rot270 => write!(f, "270"),
            Transform::Flipped => write!(f, "flipped"),
            Transform::Flipped90 => write!(f, "flipped-90"),
            Transform::Flipped180 => write!(f, "flipped-180"),
            Transform::Flipped270 => write!(f, "flipped-270"),
        }
    }
}

impl Transform {
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rot90 | Self::Rot270 | Self::Flipped90 | Self::Flipped270
        )
    }
}

fn validate_string(label: &str, value: &str) -> Result<(), String> {
    if value.len() > MAX_TOPOLOGY_STRING_BYTES {
        return Err(format!(
            "{label} is {} bytes, limit is {}",
            value.len(),
            MAX_TOPOLOGY_STRING_BYTES
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} contains control characters"));
    }

    Ok(())
}

#[cfg(test)]
mod tests;
