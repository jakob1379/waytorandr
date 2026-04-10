use crate::model::{OutputIdentity, OutputState, Position, VirtualPreset};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Profile {
    pub name: String,
    pub priority: u32,
    #[serde(default)]
    pub match_rules: Vec<OutputMatcher>,
    pub layout: HashMap<String, OutputConfig>,
    #[serde(default)]
    pub hooks: Hooks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputMatcher {
    pub identity: OutputIdentity,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub position_hint: Option<Position>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputConfig {
    #[serde(flatten)]
    pub state: OutputState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<VirtualPreset>,
}

impl From<OutputState> for OutputConfig {
    fn from(state: OutputState) -> Self {
        Self {
            state,
            preset: None,
        }
    }
}

impl OutputMatcher {
    #[must_use]
    pub fn new(identity: OutputIdentity, required: bool, position_hint: Option<Position>) -> Self {
        Self {
            identity,
            required,
            position_hint,
        }
    }
}

impl Profile {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        priority: u32,
        match_rules: Vec<OutputMatcher>,
        layout: HashMap<String, OutputConfig>,
    ) -> Self {
        Self {
            name: name.into(),
            priority,
            match_rules,
            layout,
            hooks: Hooks::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct Hooks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_apply: Vec<Hook>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_apply: Vec<Hook>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_failure: Vec<Hook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
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

impl Hook {
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            timeout_secs: default_timeout(),
        }
    }
}

impl Profile {
    #[must_use]
    pub fn setup_fingerprint(&self) -> String {
        let mut parts: Vec<String> = if self.match_rules.is_empty() {
            self.layout
                .values()
                .filter(|config| {
                    !config.state.identity.is_ignored && !config.state.identity.is_virtual
                })
                .map(|config| config.state.identity.primary_key())
                .collect()
        } else {
            self.match_rules
                .iter()
                .filter(|matcher| !matcher.identity.is_ignored && !matcher.identity.is_virtual)
                .map(|matcher| matcher.identity.primary_key())
                .collect()
        };
        parts.sort();
        parts.join(";")
    }

    #[must_use]
    pub fn layout_fingerprint(&self) -> String {
        let mut parts: Vec<String> = if self.layout.is_empty() {
            self.match_rules
                .iter()
                .map(|matcher| {
                    let key = matcher.identity.primary_key();
                    let state = if matcher.required {
                        "required"
                    } else {
                        "optional"
                    };
                    format!("{key}:{state}")
                })
                .collect()
        } else {
            self.layout
                .values()
                .map(|config| config.state.fingerprint())
                .collect()
        };
        parts.sort();
        parts.join(";")
    }

    #[must_use]
    pub fn with_inferred_match_rules(&self) -> Self {
        if !self.match_rules.is_empty() {
            return self.clone();
        }

        let mut inferred = self.clone();
        inferred.match_rules = self
            .layout
            .values()
            .filter(|config| !config.state.identity.is_ignored && !config.state.identity.is_virtual)
            .map(|config| OutputMatcher {
                identity: config.state.identity.clone(),
                required: config.state.enabled,
                position_hint: Some(config.state.position),
            })
            .collect();
        inferred
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OutputState, Position};

    #[test]
    fn infers_match_rules_from_layout_when_missing() {
        let mut layout = HashMap::new();
        layout.insert(
            "DP-1".to_string(),
            OutputConfig {
                state: {
                    let mut state = OutputState::new("DP-1");
                    state.enabled = true;
                    state.position = Position::new(10, 20);
                    state
                },
                preset: None,
            },
        );

        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: Vec::new(),
            layout,
            hooks: Hooks::default(),
        };

        let inferred = profile.with_inferred_match_rules();

        assert_eq!(inferred.match_rules.len(), 1);
        assert_eq!(
            inferred.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
        assert!(inferred.match_rules[0].required);
        assert_eq!(
            inferred.match_rules[0].position_hint,
            Some(Position::new(10, 20))
        );
    }

    #[test]
    fn ignores_virtual_and_ignored_outputs_in_fingerprints_and_inference() {
        let mut layout = HashMap::new();

        let mut regular = OutputState::new("DP-1");
        regular.enabled = true;
        layout.insert(
            "DP-1".to_string(),
            OutputConfig {
                state: regular,
                preset: None,
            },
        );

        let mut virtual_output = OutputState::new("VIRT-1");
        virtual_output.identity.is_virtual = true;
        layout.insert(
            "VIRT-1".to_string(),
            OutputConfig {
                state: virtual_output,
                preset: None,
            },
        );

        let mut ignored_output = OutputState::new("IGN-1");
        ignored_output.identity.is_ignored = true;
        layout.insert(
            "IGN-1".to_string(),
            OutputConfig {
                state: ignored_output,
                preset: None,
            },
        );

        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: Vec::new(),
            layout,
            hooks: Hooks::default(),
        };

        assert_eq!(profile.setup_fingerprint(), "conn:DP-1");

        let inferred = profile.with_inferred_match_rules();
        assert_eq!(inferred.match_rules.len(), 1);
        assert_eq!(
            inferred.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
    }

    #[test]
    fn ignores_virtual_and_ignored_match_rules_in_setup_fingerprint() {
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: vec![
                OutputMatcher::new(OutputIdentity::new("DP-1"), true, None),
                OutputMatcher::new(
                    OutputIdentity {
                        connector: Some("VIRT-1".to_string()),
                        is_virtual: true,
                        ..OutputIdentity::default()
                    },
                    true,
                    None,
                ),
                OutputMatcher::new(
                    OutputIdentity {
                        connector: Some("IGN-1".to_string()),
                        is_ignored: true,
                        ..OutputIdentity::default()
                    },
                    true,
                    None,
                ),
            ],
            layout: HashMap::new(),
            hooks: Hooks::default(),
        };

        assert_eq!(profile.setup_fingerprint(), "conn:DP-1");
    }

    #[test]
    fn deserializes_legacy_preset_strings_and_ignores_options() {
        let mut layout = HashMap::new();
        layout.insert(
            "DP-1".to_string(),
            OutputConfig {
                state: OutputState::new("DP-1"),
                preset: Some(VirtualPreset::Common),
            },
        );

        let mut value = serde_json::to_value(Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: Vec::new(),
            layout,
            hooks: Hooks::default(),
        })
        .expect("profile should serialize");

        value["layout"]["DP-1"]["preset"] = serde_json::Value::String("horizontal".to_string());
        value["options"] = serde_json::json!({
            "ignore_scale": true,
            "ignore_transform": true,
            "fallback": "legacy"
        });

        let profile: Profile = serde_json::from_value(value).expect("legacy profile should load");

        assert_eq!(
            profile.layout["DP-1"].preset,
            Some(VirtualPreset::Horizontal)
        );
    }
}
