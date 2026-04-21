use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Gnome,
    KScreen,
    Wlroots,
    Test,
    #[default]
    Unknown,
}

impl BackendKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gnome => "gnome",
            Self::KScreen => "kscreen",
            Self::Wlroots => "wlroots",
            Self::Test => "test",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub const fn is_native_mirror_backend(self) -> bool {
        matches!(self, Self::Gnome | Self::KScreen)
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.trim().to_ascii_lowercase();
        match lowered.as_str() {
            "gnome" => Some(Self::Gnome),
            "kscreen" => Some(Self::KScreen),
            "wlroots" => Some(Self::Wlroots),
            "test" => Some(Self::Test),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum VirtualPreset {
    Off,
    External,
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
        let transform = self.transform;
        format!("{key}:{enabled}:{width}x{height}@{refresh}:{scale}:{transform}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[non_exhaustive]
pub struct OutputIdentity {
    pub edid_hash: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub connector: Option<String>,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_virtual: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_ignored: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(v: &bool) -> bool {
    !*v
}

impl OutputIdentity {
    #[must_use]
    pub fn new(connector: impl Into<String>) -> Self {
        Self {
            connector: Some(connector.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn primary_key(&self) -> String {
        if let Some(hash) = &self.edid_hash {
            return format!("edid:{hash}");
        }
        let parts: Vec<String> = [
            normalized_identity_value(self.make.as_deref()),
            normalized_identity_value(self.model.as_deref()),
            normalized_identity_value(self.serial.as_deref()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            let joined = parts.join(":");
            return format!("id:{joined}");
        }
        if let Some(conn) = normalized_identity_value(self.connector.as_deref()) {
            return format!("conn:{conn}");
        }
        let description = normalized_identity_value(self.description.as_deref());
        description.unwrap_or_else(|| "unknown".to_string())
    }

    #[must_use]
    pub fn match_strength(&self) -> u8 {
        let mut strength = 0u8;
        if self.edid_hash.is_some() {
            strength += 5;
        }
        if normalized_identity_value(self.make.as_deref()).is_some() {
            strength += 2;
        }
        if normalized_identity_value(self.model.as_deref()).is_some() {
            strength += 2;
        }
        if normalized_identity_value(self.serial.as_deref()).is_some() {
            strength += 3;
        }
        if normalized_identity_value(self.connector.as_deref()).is_some() {
            strength += 1;
        }
        if normalized_identity_value(self.description.as_deref()).is_some() {
            strength += 0;
        }
        strength
    }

    #[must_use]
    pub fn with_fallback(&self, fallback: &OutputIdentity) -> OutputIdentity {
        Self {
            edid_hash: self
                .edid_hash
                .clone()
                .or_else(|| fallback.edid_hash.clone()),
            make: choose_identity_value(self.make.as_deref(), fallback.make.as_deref()),
            model: choose_identity_value(self.model.as_deref(), fallback.model.as_deref()),
            serial: choose_identity_value(self.serial.as_deref(), fallback.serial.as_deref()),
            connector: choose_identity_value(
                self.connector.as_deref(),
                fallback.connector.as_deref(),
            ),
            description: choose_identity_value(
                self.description.as_deref(),
                fallback.description.as_deref(),
            ),
            is_virtual: self.is_virtual,
            is_ignored: self.is_ignored,
        }
    }
}

#[must_use]
pub fn identities_match(query: &OutputIdentity, candidate: &OutputIdentity) -> bool {
    if let Some(query_hash) = &query.edid_hash {
        if let Some(cand_hash) = &candidate.edid_hash {
            return query_hash == cand_hash;
        }
        return false;
    }

    if let (Some(query_make), Some(cand_make)) = (
        normalized_identity_value(query.make.as_deref()),
        normalized_identity_value(candidate.make.as_deref()),
    ) {
        if query_make != cand_make {
            return false;
        }
    }

    if let (Some(query_model), Some(cand_model)) = (
        normalized_identity_value(query.model.as_deref()),
        normalized_identity_value(candidate.model.as_deref()),
    ) {
        if query_model != cand_model {
            return false;
        }
    }

    if let (Some(query_serial), Some(cand_serial)) = (
        normalized_identity_value(query.serial.as_deref()),
        normalized_identity_value(candidate.serial.as_deref()),
    ) {
        if query_serial != cand_serial {
            return false;
        }
    }

    if normalized_identity_value(query.serial.as_deref()).is_some() {
        return normalized_identity_value(candidate.serial.as_deref()).is_some();
    }

    if let (Some(query_conn), Some(cand_conn)) = (
        normalized_identity_value(query.connector.as_deref()),
        normalized_identity_value(candidate.connector.as_deref()),
    ) {
        if query_conn == cand_conn {
            return true;
        }
    }

    if let (Some(query_desc), Some(cand_desc)) = (
        normalized_identity_value(query.description.as_deref()),
        normalized_identity_value(candidate.description.as_deref()),
    ) {
        if query_desc == cand_desc {
            return true;
        }
    }

    normalized_identity_value(query.make.as_deref()).is_none()
        && normalized_identity_value(query.model.as_deref()).is_none()
        && normalized_identity_value(query.serial.as_deref()).is_none()
        && normalized_identity_value(query.connector.as_deref()).is_none()
        && normalized_identity_value(query.description.as_deref()).is_none()
}

#[must_use]
pub fn normalized_identity_value(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }

    let lower = value.to_ascii_lowercase();
    if matches!(lower.as_str(), "unknown" | "n/a" | "none") {
        return None;
    }
    if lower.starts_with("unknown - unknown -") {
        return None;
    }

    Some(value.to_string())
}

fn choose_identity_value(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    normalized_identity_value(primary).or_else(|| normalized_identity_value(fallback))
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Scale(pub f64);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Capabilities {
    pub can_test: bool,
    pub supports_mirror: bool,
    pub supports_largest_mirror: bool,
    pub backend: BackendKind,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            can_test: false,
            supports_mirror: false,
            supports_largest_mirror: false,
            backend: BackendKind::Unknown,
        }
    }
}

impl Capabilities {
    #[must_use]
    pub fn new(backend: BackendKind) -> Self {
        Self {
            backend,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.backend.as_str()
    }

    #[must_use]
    pub fn virtual_preset_unavailable_message(&self, preset: VirtualPreset) -> Option<String> {
        match preset {
            VirtualPreset::Mirror if !self.supports_mirror => Some(
                format!(
                    "native display mirroring is not available through the `{}` backend; use 'wl-mirror' for now on this compositor. See https://github.com/swaywm/wlr-protocols/issues/101",
                    self.backend
                ),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_key_ignores_unknown_identity_fields() {
        let mut identity = OutputIdentity::new("DP-4");
        identity.make = Some("Unknown".to_string());
        identity.model = Some("Unknown".to_string());
        identity.description = Some("Unknown - Unknown - DP-4".to_string());

        assert_eq!(identity.primary_key(), "conn:DP-4");
    }

    #[test]
    fn same_layout_as_ignores_mode_inventory_and_backend_metadata() {
        let mut left = OutputState::new("DP-1");
        left.enabled = true;
        left.mode = Some(Mode::new(1920, 1080, 60));
        left.available_modes = vec![Mode::new(1920, 1080, 60)];
        left.backend_data = Some(serde_json::json!({"side": "left"}));

        let mut right = left.clone();
        right.available_modes = vec![Mode::new(1280, 720, 60), Mode::new(1920, 1080, 60)];
        right.backend_data = Some(serde_json::json!({"side": "right"}));

        assert!(left.same_layout_as(&right));
    }

    #[test]
    fn virtual_preset_policy_is_centralized() {
        let capabilities = Capabilities {
            can_test: true,
            supports_mirror: false,
            supports_largest_mirror: false,
            backend: BackendKind::Wlroots,
        };

        assert!(capabilities
            .virtual_preset_unavailable_message(VirtualPreset::Mirror)
            .is_some());
        assert!(capabilities
            .virtual_preset_unavailable_message(VirtualPreset::Largest)
            .is_none());
        assert!(capabilities
            .virtual_preset_unavailable_message(VirtualPreset::Common)
            .is_none());
        assert!(capabilities
            .virtual_preset_unavailable_message(VirtualPreset::Horizontal)
            .is_none());
    }
}
