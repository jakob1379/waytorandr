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
        let mut parts: Vec<String> = self.outputs.keys().map(|k| {
            let o = &self.outputs[k];
            format!("{}:{}", k, if o.enabled { "on" } else { "off" })
        }).collect();
        parts.sort();
        parts.join(";")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    pub fn new(name: &str) -> Self {
        let mut state = Self::default();
        state.identity.connector = Some(name.to_string());
        state
    }

    pub fn fingerprint(&self) -> String {
        format!(
            "{}:{}:{}x{}@{}:{}:{}",
            self.identity.primary_key(),
            if self.enabled { "on" } else { "off" },
            self.mode.as_ref().map(|m| m.width.to_string()).unwrap_or_default(),
            self.mode.as_ref().map(|m| m.height.to_string()).unwrap_or_default(),
            self.mode.as_ref().map(|m| m.refresh.to_string()).unwrap_or_default(),
            self.scale,
            self.transform,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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
    pub fn primary_key(&self) -> String {
        if let Some(hash) = &self.edid_hash {
            return format!("edid:{}", hash);
        }
        let parts: Vec<String> = [
            self.make.clone(),
            self.model.clone(),
            self.serial.clone(),
        ].into_iter().flatten().collect();
        if !parts.is_empty() {
            return format!("id:{}", parts.join(":"));
        }
        if let Some(conn) = &self.connector {
            return format!("conn:{}", conn);
        }
        self.description.clone().unwrap_or_else(|| "unknown".to_string())
    }

    pub fn match_strength(&self) -> u8 {
        let mut strength = 0u8;
        if self.edid_hash.is_some() { strength += 5; }
        if self.make.is_some() { strength += 2; }
        if self.model.is_some() { strength += 2; }
        if self.serial.is_some() { strength += 3; }
        if self.connector.is_some() { strength += 1; }
        if self.description.is_some() { strength += 0; }
        strength
    }
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
        Self { width, height, refresh }
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
