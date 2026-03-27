use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
            position: Position::default(),
            scale: 1.0,
            transform: Transform::default(),
            mirror_target: None,
            backend_data: None,
        }
    }
}

impl Topology {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fingerprint(&self) -> String {
        let mut parts: Vec<String> = self
            .outputs
            .keys()
            .map(|k| {
                let o = &self.outputs[k];
                format!("{}:{}", k, if o.enabled { "on" } else { "off" })
            })
            .collect();
        parts.sort();
        parts.join(";")
    }

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

    pub fn state_fingerprint(&self) -> String {
        let mut parts: Vec<String> = self
            .outputs
            .iter()
            .map(|(name, output)| {
                format!(
                    "{}:{}:{}:{}",
                    name,
                    output.fingerprint(),
                    output.position.x,
                    output.position.y
                )
            })
            .collect();
        parts.sort();
        parts.join(";")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct OutputState {
    pub identity: OutputIdentity,
    pub enabled: bool,
    pub mode: Option<Mode>,
    pub position: Position,
    pub scale: f64,
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_target: Option<String>,
    pub backend_data: Option<serde_json::Value>,
}

impl OutputState {
    pub fn new(name: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.identity.connector = Some(name.into());
        state
    }

    pub fn fingerprint(&self) -> String {
        format!(
            "{}:{}:{}x{}@{}:{}:{}",
            self.identity.primary_key(),
            if self.enabled { "on" } else { "off" },
            self.mode
                .as_ref()
                .map(|m| m.width.to_string())
                .unwrap_or_default(),
            self.mode
                .as_ref()
                .map(|m| m.height.to_string())
                .unwrap_or_default(),
            self.mode
                .as_ref()
                .map(|m| m.refresh.to_string())
                .unwrap_or_default(),
            self.scale,
            self.transform,
        )
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

fn is_false(v: &bool) -> bool {
    !*v
}

impl OutputIdentity {
    pub fn new(connector: impl Into<String>) -> Self {
        Self {
            connector: Some(connector.into()),
            ..Self::default()
        }
    }

    pub fn primary_key(&self) -> String {
        if let Some(hash) = &self.edid_hash {
            return format!("edid:{}", hash);
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
            return format!("id:{}", parts.join(":"));
        }
        if let Some(conn) = normalized_identity_value(self.connector.as_deref()) {
            return format!("conn:{}", conn);
        }
        normalized_identity_value(self.description.as_deref())
            .unwrap_or_else(|| "unknown".to_string())
    }

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

    pub fn with_fallback(&self, fallback: &OutputIdentity) -> OutputIdentity {
        let mut identity = Self::default();
        identity.edid_hash = self
            .edid_hash
            .clone()
            .or_else(|| fallback.edid_hash.clone());
        identity.make = choose_identity_value(self.make.as_deref(), fallback.make.as_deref());
        identity.model = choose_identity_value(self.model.as_deref(), fallback.model.as_deref());
        identity.serial = choose_identity_value(self.serial.as_deref(), fallback.serial.as_deref());
        identity.connector =
            choose_identity_value(self.connector.as_deref(), fallback.connector.as_deref());
        identity.description =
            choose_identity_value(self.description.as_deref(), fallback.description.as_deref());
        identity.is_virtual = self.is_virtual;
        identity.is_ignored = self.is_ignored;
        identity
    }
}

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
    pub can_enumerate: bool,
    pub can_watch: bool,
    pub can_test: bool,
    pub can_apply: bool,
    pub supports_transforms: bool,
    pub supports_scale: bool,
    pub supports_mirror: bool,
    pub supports_brightness: bool,
    pub supports_gamma: bool,
    pub backend_name: String,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            can_enumerate: false,
            can_watch: false,
            can_test: false,
            can_apply: false,
            supports_transforms: false,
            supports_scale: false,
            supports_mirror: false,
            supports_brightness: false,
            supports_gamma: false,
            backend_name: "unknown".to_string(),
        }
    }
}

impl Capabilities {
    pub fn named(backend_name: impl Into<String>) -> Self {
        Self {
            backend_name: backend_name.into(),
            ..Self::default()
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
}
