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
    pub fn match_profile(topology: &Topology, profiles: &[Profile]) -> Option<MatchResult> {
        Self::matching_profiles(topology, profiles)
            .into_iter()
            .next()
    }

    pub fn matching_profiles(topology: &Topology, profiles: &[Profile]) -> Vec<MatchResult> {
        let mut candidates: Vec<MatchResult> = profiles
            .iter()
            .filter_map(|p| Self::score_profile(topology, p))
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.profile.priority.cmp(&a.profile.priority))
                .then(a.profile.name.cmp(&b.profile.name))
        });

        candidates
    }

    fn score_profile(topology: &Topology, profile: &Profile) -> Option<MatchResult> {
        let mut matched_outputs: HashMap<String, String> = HashMap::new();
        let mut unmatched_required: Vec<String> = Vec::new();
        let mut total_score = 0u32;

        for matcher in &profile.match_rules {
            let matched = Self::find_matching_output(&matcher.identity, topology, &matched_outputs);
            match matched {
                Some((topo_name, output)) => {
                    matched_outputs.insert(topo_name.clone(), topo_name.clone());
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

        let topology_names: std::collections::HashSet<String> =
            topology.outputs.keys().cloned().collect();
        let matched_names: std::collections::HashSet<String> =
            matched_outputs.values().cloned().collect();
        let extra_outputs: Vec<String> = topology_names
            .difference(&matched_names)
            .filter(|name| {
                let name: &String = name;
                topology
                    .outputs
                    .get(name)
                    .map(|o| !o.identity.is_ignored && !o.identity.is_virtual)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        if profile.match_rules.is_empty() && !extra_outputs.is_empty() {
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
        identity: &OutputIdentity,
        topology: &Topology,
        already_matched: &HashMap<String, String>,
    ) -> Option<(String, OutputState)> {
        let mut best_match: Option<(String, OutputState, u8)> = None;

        for (name, state) in &topology.outputs {
            if already_matched.contains_key(name) {
                continue;
            }
            if state.identity.is_ignored || state.identity.is_virtual {
                continue;
            }

            if Self::identities_match(identity, &state.identity) {
                let score = state.identity.match_strength();
                match &best_match {
                    None => best_match = Some((name.clone(), state.clone(), score)),
                    Some((_, _, best)) if score > *best => {
                        best_match = Some((name.clone(), state.clone(), score));
                    }
                    _ => {}
                }
            }
        }

        best_match.map(|(name, state, _)| (name, state))
    }

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
    use crate::profile::OutputMatcher;

    fn make_topology() -> Topology {
        let mut outputs = HashMap::new();
        outputs.insert(
            "DP-1".to_string(),
            {
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
            },
        );
        Topology { outputs }
    }

    #[test]
    fn test_exact_edid_match() {
        let topo = make_topology();
        let profile = Profile {
            name: "test".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: {
                    let mut identity = OutputIdentity::default();
                    identity.edid_hash = Some("abc123".to_string());
                    identity
                },
                required: true,
                position_hint: None,
            }],
            layout: Default::default(),
            hooks: Default::default(),
            options: Default::default(),
        };

        let result = Matcher::match_profile(&topo, &[profile]);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.profile.name, "test");
        assert!(result.unmatched_required.is_empty());
    }

    #[test]
    fn test_missing_required() {
        let topo = make_topology();
        let profile = Profile {
            name: "test".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: {
                    let mut identity = OutputIdentity::default();
                    identity.edid_hash = Some("missing".to_string());
                    identity
                },
                required: true,
                position_hint: None,
            }],
            layout: Default::default(),
            hooks: Default::default(),
            options: Default::default(),
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
}
