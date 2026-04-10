use crate::model::{OutputIdentity, Topology};
use crate::profile::Profile;
use std::collections::HashMap;
use std::hash::BuildHasher;

#[must_use]
pub fn canonicalize_profile<S: BuildHasher>(
    profile: &Profile,
    known_outputs: &HashMap<String, OutputIdentity, S>,
) -> Profile {
    normalize_profile_with_known_outputs(&profile.with_inferred_match_rules(), known_outputs)
}

#[must_use]
pub fn normalize_profile_with_known_outputs<S: BuildHasher>(
    profile: &Profile,
    known_outputs: &HashMap<String, OutputIdentity, S>,
) -> Profile {
    let mut normalized = profile.clone();

    for matcher in &mut normalized.match_rules {
        if let Some(connector) = matcher.identity.connector.as_deref() {
            if let Some(cached) = known_outputs.get(connector) {
                matcher.identity = matcher.identity.with_fallback(cached);
            }
        }
    }

    for (connector, config) in &mut normalized.layout {
        if let Some(cached) = known_outputs.get(connector) {
            config.state.identity = config.state.identity.with_fallback(cached);
        } else if let Some(connector) = config.state.identity.connector.as_deref() {
            if let Some(cached) = known_outputs.get(connector) {
                config.state.identity = config.state.identity.with_fallback(cached);
            }
        }
    }

    normalized
}

#[must_use]
pub fn normalize_topology_with_known_outputs<S: BuildHasher>(
    topology: &Topology,
    known_outputs: &HashMap<String, OutputIdentity, S>,
) -> Topology {
    let mut normalized = topology.clone();

    for (name, output) in &mut normalized.outputs {
        if let Some(cached) = known_outputs.get(name) {
            output.identity = output.identity.with_fallback(cached);
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OutputState, Position};
    use crate::profile::{Hooks, OutputConfig};

    #[test]
    fn canonicalize_profile_infers_and_normalizes_known_outputs() {
        let mut layout = HashMap::new();
        let mut output = OutputState::new("DP-1");
        output.enabled = true;
        output.position = Position::new(10, 20);
        layout.insert(
            "DP-1".to_string(),
            OutputConfig {
                state: output,
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

        let mut known_outputs = HashMap::new();
        known_outputs.insert(
            "DP-1".to_string(),
            OutputIdentity {
                make: Some("Dell".to_string()),
                model: Some("U2720Q".to_string()),
                serial: Some("abc123".to_string()),
                connector: Some("DP-1".to_string()),
                description: Some("Display".to_string()),
                ..OutputIdentity::default()
            },
        );

        let normalized = canonicalize_profile(&profile, &known_outputs);

        assert_eq!(normalized.match_rules.len(), 1);
        assert_eq!(
            normalized.match_rules[0].identity.make.as_deref(),
            Some("Dell")
        );
        assert_eq!(
            normalized.match_rules[0].identity.model.as_deref(),
            Some("U2720Q")
        );
        assert_eq!(
            normalized.layout["DP-1"].state.identity.serial.as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn normalize_topology_uses_known_output_identity() {
        let mut topology = Topology::default();
        let mut output = OutputState::new("DP-1");
        output.enabled = true;
        topology.outputs.insert("DP-1".to_string(), output);

        let mut known_outputs = HashMap::new();
        known_outputs.insert(
            "DP-1".to_string(),
            OutputIdentity {
                make: Some("Dell".to_string()),
                model: Some("U2720Q".to_string()),
                serial: Some("abc123".to_string()),
                connector: Some("DP-1".to_string()),
                description: Some("Display".to_string()),
                ..OutputIdentity::default()
            },
        );

        let normalized = normalize_topology_with_known_outputs(&topology, &known_outputs);

        assert_eq!(
            normalized.outputs["DP-1"].identity.make.as_deref(),
            Some("Dell")
        );
        assert_eq!(
            normalized.outputs["DP-1"].identity.model.as_deref(),
            Some("U2720Q")
        );
        assert_eq!(
            normalized.outputs["DP-1"].identity.serial.as_deref(),
            Some("abc123")
        );
    }
}
