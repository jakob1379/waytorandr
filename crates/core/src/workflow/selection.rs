use crate::error::{CoreError, CoreResult};
use crate::model::Topology;
use crate::planning::{LayoutPlan, Matcher, Planner};
use crate::profile::{Hooks, OutputMatcher, Profile};
use crate::state::State;
use crate::store::ProfilesSettings;

pub enum ProfileSelectionDecision {
    SetupDefault(Profile),
    ExactMatch(Profile),
    RememberedLayout(Topology),
    NoMatch,
}

impl ProfileSelectionDecision {
    #[must_use]
    pub fn selected_profile(&self) -> Option<&Profile> {
        match self {
            Self::SetupDefault(profile) | Self::ExactMatch(profile) => Some(profile),
            Self::RememberedLayout(_) | Self::NoMatch => None,
        }
    }
}

#[must_use]
pub fn select_profile_for_topology(
    topology: &Topology,
    profiles: &[Profile],
    settings: &ProfilesSettings,
) -> Option<Profile> {
    select_profile_application_target(topology, profiles, settings, &State::default())
        .selected_profile()
        .cloned()
}

#[must_use]
pub fn select_profile_application_target(
    topology: &Topology,
    profiles: &[Profile],
    settings: &ProfilesSettings,
    state: &State,
) -> ProfileSelectionDecision {
    let setup_fingerprint = topology.setup_fingerprint();

    if let Some(default_name) = settings.setup_default_profile(&setup_fingerprint) {
        if let Some(profile) = profiles.iter().find(|profile| {
            profile.name == default_name && profile.setup_fingerprint() == setup_fingerprint
        }) {
            return ProfileSelectionDecision::SetupDefault(profile.clone());
        }
    }

    if let Some(matched) = Matcher::match_profile_exact(topology, profiles) {
        return ProfileSelectionDecision::ExactMatch(matched.profile);
    }

    if let Some(remembered) = state.remembered_topology_for_setup(&setup_fingerprint) {
        if remembered.has_enabled_real_outputs() {
            return ProfileSelectionDecision::RememberedLayout(remembered.clone());
        }
    }

    ProfileSelectionDecision::NoMatch
}

#[must_use]
pub fn current_profile_name(
    topology: &Topology,
    profiles: &[Profile],
    state: &State,
) -> Option<String> {
    state
        .last_profile
        .as_deref()
        .and_then(|last_profile| profiles.iter().find(|profile| profile.name == last_profile))
        .and_then(|profile| {
            if Matcher::match_profile_exact(topology, std::slice::from_ref(profile)).is_some() {
                Some(profile.name.clone())
            } else {
                None
            }
        })
}

#[must_use]
pub fn profile_from_topology(name: &str, topology: &Topology) -> Profile {
    Profile {
        name: name.to_string(),
        priority: 0,
        match_rules: topology
            .outputs
            .values()
            .filter(|output| !output.identity.is_ignored && !output.identity.is_virtual)
            .map(|output| OutputMatcher {
                identity: output.identity.clone(),
                required: output.enabled,
                position_hint: Some(output.position),
            })
            .collect(),
        layout: topology
            .outputs
            .iter()
            .map(|(output_name, output)| (output_name.clone(), output.clone().into()))
            .collect(),
        hooks: Hooks::default(),
    }
}

/// Build a layout plan for `profile` against `topology`.
///
/// # Errors
/// Returns `CoreError::ProfileMismatch` if the profile does not match the topology,
/// or any planning error reported by the planner.
pub fn plan_profile_for_topology(profile: &Profile, topology: &Topology) -> CoreResult<LayoutPlan> {
    let matched = Matcher::match_profile_exact(topology, std::slice::from_ref(profile))
        .ok_or(CoreError::ProfileMismatch)?;
    Planner::plan_from_profile(&matched.profile, &matched.matched_outputs, topology)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OutputIdentity, OutputState, Position};
    use crate::profile::OutputConfig;
    use std::collections::HashMap;

    fn selection_output_state(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state
    }

    fn selection_profile(name: &str, connector: &str) -> Profile {
        Profile {
            name: name.to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: OutputIdentity::new(connector),
                required: true,
                position_hint: Some(Position::default()),
            }],
            layout: HashMap::from([(
                connector.to_string(),
                OutputConfig {
                    state: selection_output_state(connector),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        }
    }

    #[test]
    fn select_profile_for_topology_prefers_setup_default_before_match() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let mut settings = ProfilesSettings::default();
        settings.set_setup_default_profile(&topology.setup_fingerprint(), "external-only");
        let profiles = vec![
            selection_profile("both", "DP-1"),
            selection_profile("external-only", "DP-1"),
        ];

        let selected = select_profile_for_topology(&topology, &profiles, &settings).unwrap();

        assert_eq!(selected.name, "external-only");
    }

    #[test]
    fn select_profile_for_topology_uses_matching_profile_without_fallback() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let settings = ProfilesSettings::default();
        let profiles = vec![
            selection_profile("desk", "DP-1"),
            selection_profile("fallback", "HDMI-A-1"),
        ];

        let selected = select_profile_for_topology(&topology, &profiles, &settings).unwrap();

        assert_eq!(selected.name, "desk");
    }

    #[test]
    fn select_profile_application_target_uses_remembered_layout_after_profile_miss() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let settings = ProfilesSettings::default();
        let profiles = vec![selection_profile("dock", "HDMI-A-1")];
        let mut state = State::default();
        state
            .remembered_setups
            .insert(topology.setup_fingerprint(), topology.clone());

        let selected = select_profile_application_target(&topology, &profiles, &settings, &state);

        assert!(matches!(
            selected,
            ProfileSelectionDecision::RememberedLayout(_)
        ));
    }

    #[test]
    fn select_profile_application_target_rejects_disabled_remembered_layout() {
        let mut remembered = selection_output_state("DP-1");
        remembered.enabled = false;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let settings = ProfilesSettings::default();
        let profiles = vec![selection_profile("dock", "HDMI-A-1")];
        let mut state = State::default();
        state.remembered_setups.insert(
            topology.setup_fingerprint(),
            Topology {
                outputs: HashMap::from([("DP-1".to_string(), remembered)]),
            },
        );

        assert!(matches!(
            select_profile_application_target(&topology, &profiles, &settings, &state),
            ProfileSelectionDecision::NoMatch
        ));
    }

    #[test]
    fn current_profile_name_uses_recorded_profile_when_it_still_matches() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let profiles = vec![selection_profile("desk", "DP-1")];
        let mut state = State::default();
        state.last_profile = Some("desk".to_string());

        let selected = current_profile_name(&topology, &profiles, &state);

        assert_eq!(selected.as_deref(), Some("desk"));
    }

    #[test]
    fn current_profile_name_returns_none_when_recorded_profile_is_stale() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let profiles = vec![
            selection_profile("desk", "DP-1"),
            selection_profile("manual", "HDMI-A-1"),
        ];
        let mut state = State::default();
        state.last_profile = Some("manual".to_string());

        let selected = current_profile_name(&topology, &profiles, &state);

        assert_eq!(selected, None);
    }

    #[test]
    fn current_profile_name_returns_none_without_recorded_profile() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let profiles = vec![selection_profile("desk", "DP-1")];

        let selected = current_profile_name(&topology, &profiles, &State::default());

        assert_eq!(selected, None);
    }

    #[test]
    fn current_profile_name_rejects_profiles_when_topology_has_extra_real_outputs() {
        let topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), selection_output_state("DP-1")),
                ("HDMI-A-1".to_string(), selection_output_state("HDMI-A-1")),
            ]),
        };
        let profiles = vec![selection_profile("desk", "DP-1")];
        let mut state = State::default();
        state.last_profile = Some("desk".to_string());

        assert_eq!(current_profile_name(&topology, &profiles, &state), None);
    }

    #[test]
    fn profile_from_topology_builds_matchers_from_real_outputs() {
        let mut virtual_output = selection_output_state("HEADLESS-1");
        virtual_output.identity.is_virtual = true;
        let mut topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), selection_output_state("DP-1")),
                ("HEADLESS-1".to_string(), virtual_output),
            ]),
        };
        topology.outputs.get_mut("DP-1").unwrap().enabled = false;

        let profile = profile_from_topology("desk", &topology);

        assert_eq!(profile.name, "desk");
        assert_eq!(profile.layout.len(), 2);
        assert_eq!(profile.match_rules.len(), 1);
        assert_eq!(
            profile.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
        assert!(!profile.match_rules[0].required);
    }

    #[test]
    fn plan_profile_for_topology_returns_mismatch_error_for_nonmatching_profile() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let profile = selection_profile("desk", "HDMI-A-1");

        let result = plan_profile_for_topology(&profile, &topology);

        assert!(matches!(result, Err(CoreError::ProfileMismatch)));
    }

    #[test]
    fn plan_profile_for_topology_uses_matched_layout_binding() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), selection_output_state("DP-1"))]),
        };
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: OutputIdentity::new("DP-1"),
                required: true,
                position_hint: Some(Position::default()),
            }],
            layout: HashMap::from([(
                "left-panel".to_string(),
                OutputConfig {
                    state: selection_output_state("DP-1"),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        };

        let plan = plan_profile_for_topology(&profile, &topology).unwrap();

        assert!(plan.outputs.contains_key("DP-1"));
        assert_eq!(
            plan.outputs["DP-1"].identity.connector.as_deref(),
            Some("DP-1")
        );
    }

    #[test]
    fn plan_profile_for_topology_rejects_extra_real_outputs() {
        let topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), selection_output_state("DP-1")),
                ("HDMI-A-1".to_string(), selection_output_state("HDMI-A-1")),
            ]),
        };

        assert!(matches!(
            plan_profile_for_topology(&selection_profile("desk", "DP-1"), &topology),
            Err(CoreError::ProfileMismatch)
        ));
    }
}
