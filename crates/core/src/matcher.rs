use crate::model::{identities_match, OutputIdentity, OutputState, Topology};
use crate::profile::Profile;
use std::collections::HashMap;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MatchResult {
    pub profile: Profile,
    pub score: u32,
    pub matched_outputs: HashMap<String, String>,
    pub unmatched_required: Vec<String>,
    pub extra_outputs: Vec<String>,
}

pub struct Matcher;

impl Matcher {
    #[must_use]
    pub fn match_profile(topology: &Topology, profiles: &[Profile]) -> Option<MatchResult> {
        Self::matching_profiles(topology, profiles)
            .into_iter()
            .next()
    }

    #[must_use]
    pub fn matching_profiles(topology: &Topology, profiles: &[Profile]) -> Vec<MatchResult> {
        Self::matching_profiles_internal(topology, profiles, false)
    }

    #[must_use]
    pub fn match_profile_exact(topology: &Topology, profiles: &[Profile]) -> Option<MatchResult> {
        Self::matching_profiles_exact(topology, profiles)
            .into_iter()
            .next()
    }

    #[must_use]
    pub fn matching_profiles_exact(topology: &Topology, profiles: &[Profile]) -> Vec<MatchResult> {
        Self::matching_profiles_internal(topology, profiles, true)
    }

    fn matching_profiles_internal(
        topology: &Topology,
        profiles: &[Profile],
        require_exact: bool,
    ) -> Vec<MatchResult> {
        let mut candidates: Vec<MatchResult> = profiles
            .iter()
            .filter_map(|p| Self::score_profile(topology, p, require_exact))
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.profile.priority.cmp(&a.profile.priority))
                .then(a.profile.name.cmp(&b.profile.name))
        });

        candidates
    }

    fn score_profile(
        topology: &Topology,
        profile: &Profile,
        require_exact: bool,
    ) -> Option<MatchResult> {
        let mut matched_topology_outputs: HashMap<String, OutputState> = HashMap::new();
        let mut matched_outputs: HashMap<String, String> = HashMap::new();
        let mut unmatched_required: Vec<String> = Vec::new();
        let mut total_score = 0u32;
        let mut used_layout_names: Vec<String> = Vec::new();

        for matcher in &profile.match_rules {
            let matched = Self::find_matching_output(matcher, topology, &matched_topology_outputs);
            match matched {
                Some((topo_name, output)) => {
                    let layout_name = Self::find_matching_layout_entry(
                        profile,
                        &output.identity,
                        matcher,
                        &used_layout_names,
                    )?;
                    matched_topology_outputs.insert(topo_name.clone(), output.clone());
                    used_layout_names.push(layout_name.clone());
                    matched_outputs.insert(topo_name.clone(), layout_name);
                    total_score += Self::identity_match_score(&matcher.identity, &output.identity);
                }
                None if matcher.required => {
                    unmatched_required.push(Self::identity_desc(&matcher.identity));
                }
                None => {}
            }
        }

        if !unmatched_required.is_empty() {
            return None;
        }

        let mut extra_outputs: Vec<String> = topology
            .outputs
            .iter()
            .filter(|(name, output)| {
                !matched_topology_outputs.contains_key(*name)
                    && !output.identity.is_ignored
                    && !output.identity.is_virtual
            })
            .map(|(name, _)| name.clone())
            .collect();
        extra_outputs.sort();

        if (profile.match_rules.is_empty() || require_exact) && !extra_outputs.is_empty() {
            return None;
        }

        Some(MatchResult {
            profile: profile.clone(),
            score: total_score,
            matched_outputs,
            unmatched_required,
            extra_outputs,
        })
    }

    fn find_matching_output(
        matcher: &crate::profile::OutputMatcher,
        topology: &Topology,
        already_matched: &HashMap<String, OutputState>,
    ) -> Option<(String, OutputState)> {
        let mut candidates: Vec<(String, OutputState, bool, u8)> = topology
            .outputs
            .iter()
            .filter(|(name, state)| {
                !already_matched.contains_key(*name)
                    && !state.identity.is_ignored
                    && !state.identity.is_virtual
                    && Self::identities_match(&matcher.identity, &state.identity)
            })
            .map(|(name, state)| {
                (
                    name.clone(),
                    state.clone(),
                    matcher
                        .position_hint
                        .is_some_and(|hint| hint == state.position),
                    state.identity.match_strength(),
                )
            })
            .collect();

        candidates.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)).then(a.0.cmp(&b.0)));

        candidates
            .into_iter()
            .next()
            .map(|(name, state, _, _)| (name, state))
    }

    fn find_matching_layout_entry(
        profile: &Profile,
        identity: &OutputIdentity,
        matcher: &crate::profile::OutputMatcher,
        used_layout_names: &[String],
    ) -> Option<String> {
        let mut candidates: Vec<(String, bool)> = profile
            .layout
            .iter()
            .filter(|(name, config)| {
                !used_layout_names.contains(name)
                    && identities_match(&config.state.identity, identity)
                    && identities_match(&matcher.identity, &config.state.identity)
            })
            .map(|(name, config)| {
                (
                    name.clone(),
                    matcher
                        .position_hint
                        .is_some_and(|hint| hint == config.state.position),
                )
            })
            .collect();

        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        candidates.into_iter().next().map(|(name, _)| name)
    }

    #[must_use]
    pub fn identities_match(query: &OutputIdentity, candidate: &OutputIdentity) -> bool {
        identities_match(query, candidate)
    }

    fn identity_match_score(query: &OutputIdentity, _candidate: &OutputIdentity) -> u32 {
        let mut score = 0u32;

        if query.edid_hash.is_some() {
            score += 100;
        }
        if query.make.is_some() {
            score += 10;
        }
        if query.model.is_some() {
            score += 10;
        }
        if query.serial.is_some() {
            score += 20;
        }
        if query.connector.is_some() {
            score += 5;
        }

        score
    }

    fn identity_desc(identity: &OutputIdentity) -> String {
        if let Some(m) = &identity.model {
            return m.clone();
        }
        if let Some(d) = &identity.description {
            return d.clone();
        }
        if let Some(c) = &identity.connector {
            return c.clone();
        }
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Position, Transform};
    use crate::profile::{Hooks, OutputConfig, OutputMatcher};

    fn make_topology() -> Topology {
        let mut outputs = HashMap::new();
        outputs.insert("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.identity.edid_hash = Some("abc123".to_string());
            state.identity.make = Some("Dell".to_string());
            state.identity.model = Some("U2720Q".to_string());
            state.identity.serial = Some("SN001".to_string());
            state.identity.description = Some("Dell U2720Q".to_string());
            state.identity.is_virtual = false;
            state.identity.is_ignored = false;
            state.enabled = true;
            state.mode = Some(Mode {
                width: 3840,
                height: 2160,
                refresh: 60,
            });
            state.position = Position { x: 0, y: 0 };
            state.scale = 1.0;
            state.transform = Transform::Normal;
            state.mirror_target = None;
            state.backend_data = None;
            state
        });
        Topology { outputs }
    }

    fn duplicate_topology() -> Topology {
        let mut topology = Topology::default();

        let mut left = make_topology().outputs.remove("DP-1").unwrap();
        left.identity.connector = None;
        left.position = Position::new(0, 0);
        topology.outputs.insert("DP-1".to_string(), left);

        let mut right = make_topology().outputs.remove("DP-1").unwrap();
        right.identity.connector = None;
        right.position = Position::new(3840, 0);
        topology.outputs.insert("DP-2".to_string(), right);

        topology
    }

    #[test]
    fn test_exact_edid_match() {
        let topo = make_topology();
        let profile = Profile {
            name: "test".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: OutputIdentity {
                    edid_hash: Some("abc123".to_string()),
                    ..OutputIdentity::default()
                },
                required: true,
                position_hint: None,
            }],
            layout: HashMap::from([(
                "layout-1".to_string(),
                crate::profile::OutputConfig {
                    state: topo.outputs["DP-1"].clone(),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        };

        let result = Matcher::match_profile(&topo, &[profile]);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.profile.name, "test");
        assert!(result.unmatched_required.is_empty());
        assert_eq!(
            result.matched_outputs.get("DP-1").map(String::as_str),
            Some("layout-1")
        );
    }

    #[test]
    fn test_missing_required() {
        let topo = make_topology();
        let profile = Profile {
            name: "test".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: OutputIdentity {
                    edid_hash: Some("missing".to_string()),
                    ..OutputIdentity::default()
                },
                required: true,
                position_hint: None,
            }],
            layout: HashMap::from([(
                "layout-1".to_string(),
                crate::profile::OutputConfig {
                    state: topo.outputs["DP-1"].clone(),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        };

        let result = Matcher::match_profile(&topo, &[profile]);
        assert!(result.is_none());
    }

    #[test]
    fn test_serial_match_does_not_require_same_connector() {
        let query = {
            let mut identity = OutputIdentity::new("DP-4");
            identity.make = Some("Microstep".to_string());
            identity.model = Some("MSI MP273A".to_string());
            identity.serial = Some("PB4H603B02982".to_string());
            identity.description = Some("Microstep - MSI MP273A - DP-4".to_string());
            identity
        };
        let candidate = {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Microstep".to_string());
            identity.model = Some("MSI MP273A".to_string());
            identity.serial = Some("PB4H603B02982".to_string());
            identity.description = Some("Microstep - MSI MP273A - DP-1".to_string());
            identity
        };

        assert!(Matcher::identities_match(&query, &candidate));
    }

    #[test]
    fn unknown_identity_fields_fall_back_to_connector_match() {
        let query = {
            let mut identity = OutputIdentity::new("DP-4");
            identity.make = Some("Unknown".to_string());
            identity.model = Some("Unknown".to_string());
            identity.description = Some("Unknown - Unknown - DP-4".to_string());
            identity
        };
        let candidate = {
            let mut identity = OutputIdentity::new("DP-4");
            identity.make = Some("Microstep".to_string());
            identity.model = Some("MSI MP273A".to_string());
            identity.description = Some("Microstep - MSI MP273A - DP-4".to_string());
            identity
        };

        assert!(Matcher::identities_match(&query, &candidate));
    }

    #[test]
    fn duplicate_monitors_bind_one_to_one_with_position_hints() {
        let topology = duplicate_topology();
        let identity = OutputIdentity {
            edid_hash: Some("abc123".to_string()),
            make: Some("Dell".to_string()),
            model: Some("U2720Q".to_string()),
            serial: Some("SN001".to_string()),
            description: Some("Dell U2720Q".to_string()),
            ..OutputIdentity::default()
        };
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: vec![
                OutputMatcher::new(identity.clone(), true, Some(Position::new(0, 0))),
                OutputMatcher::new(identity.clone(), true, Some(Position::new(3840, 0))),
            ],
            layout: HashMap::from([
                (
                    "left".to_string(),
                    OutputConfig {
                        state: {
                            let mut state = output_state(&identity, Position::new(0, 0));
                            state.identity.connector = None;
                            state
                        },
                        preset: None,
                    },
                ),
                (
                    "right".to_string(),
                    OutputConfig {
                        state: {
                            let mut state = output_state(&identity, Position::new(3840, 0));
                            state.identity.connector = None;
                            state
                        },
                        preset: None,
                    },
                ),
            ]),
            hooks: Hooks::default(),
        };

        let result = Matcher::match_profile(&topology, &[profile]).unwrap();

        assert_eq!(result.matched_outputs.len(), 2);
        assert_eq!(
            result.matched_outputs.get("DP-1").map(String::as_str),
            Some("left")
        );
        assert_eq!(
            result.matched_outputs.get("DP-2").map(String::as_str),
            Some("right")
        );
    }

    #[test]
    fn exact_match_rejects_extra_real_outputs() {
        let mut topology = make_topology();
        topology.outputs.insert("HDMI-A-1".to_string(), {
            let mut state = OutputState::new("HDMI-A-1");
            state.enabled = true;
            state
        });
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher::new(
                OutputIdentity::new("DP-1"),
                true,
                Some(Position::new(0, 0)),
            )],
            layout: HashMap::from([(
                "left".to_string(),
                OutputConfig {
                    state: topology.outputs["DP-1"].clone(),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        };

        assert!(Matcher::match_profile(&topology, std::slice::from_ref(&profile)).is_some());
        assert!(Matcher::match_profile_exact(&topology, &[profile]).is_none());
    }

    fn output_state(identity: &OutputIdentity, position: Position) -> OutputState {
        let mut state = OutputState::new(identity.connector.as_deref().unwrap_or("DP-1"));
        state.identity = identity.clone();
        state.enabled = true;
        state.position = position;
        state
    }
}
