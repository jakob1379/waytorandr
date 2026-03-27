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
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            timeout_secs: default_timeout(),
        }
    }
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

    pub fn with_inferred_match_rules(&self) -> Self {
        if !self.match_rules.is_empty() {
            return self.clone();
        }

        let mut inferred = self.clone();
        inferred.match_rules = self
            .layout
            .values()
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
            options: ProfileOptions::default(),
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
}
