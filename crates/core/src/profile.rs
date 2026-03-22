use crate::model::{OutputIdentity, OutputState, Position};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub priority: u32,
    #[serde(default)]
    pub match_rules: Vec<OutputMatcher>,
    pub layout: HashMap<String, OutputConfig>,
    #[serde(default)]
    pub hooks: Hooks,
    #[serde(default)]
    pub options: ProfileOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputMatcher {
    pub identity: OutputIdentity,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub position_hint: Option<Position>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    #[serde(flatten)]
    pub state: OutputState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

impl From<OutputState> for OutputConfig {
    fn from(state: OutputState) -> Self {
        Self {
            state,
            preset: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Hooks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_apply: Vec<Hook>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_apply: Vec<Hook>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_failure: Vec<Hook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileOptions {
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignore_scale: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignore_transform: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl Profile {
    pub fn setup_fingerprint(&self) -> String {
        let mut parts: Vec<String> = if !self.match_rules.is_empty() {
            self.match_rules
                .iter()
                .map(|matcher| matcher.identity.primary_key())
                .collect()
        } else {
            self.layout
                .values()
                .map(|config| config.state.identity.primary_key())
                .collect()
        };
        parts.sort();
        parts.join(";")
    }

    pub fn layout_fingerprint(&self) -> String {
        let mut parts: Vec<String> = if !self.layout.is_empty() {
            self.layout
                .values()
                .map(|config| config.state.fingerprint())
                .collect()
        } else {
            self.match_rules
                .iter()
                .map(|matcher| {
                    format!(
                        "{}:{}",
                        matcher.identity.primary_key(),
                        if matcher.required {
                            "required"
                        } else {
                            "optional"
                        }
                    )
                })
                .collect()
        };
        parts.sort();
        parts.join(";")
    }

    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn load_from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let profile: Profile = toml::from_str(&content)?;
        Ok(profile)
    }
}

pub fn default_profile_dir() -> anyhow::Result<std::path::PathBuf> {
    let config_home = directories::BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?
        .config_dir()
        .join("waytorandr")
        .join("profiles");
    Ok(config_home)
}
