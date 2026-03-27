use std::collections::HashMap;
use std::fmt;

use crate::model::{identities_match, OutputState, Position, Topology};
use crate::profile::Profile;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LayoutPlan {
    pub outputs: HashMap<String, OutputState>,
    pub preset_used: Option<String>,
}

#[derive(Debug)]
pub enum PlanError {
    UnsupportedPreset(String),
    MissingOutput(String),
    InvalidConfiguration(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::UnsupportedPreset(p) => write!(f, "Unsupported preset: {}", p),
            PlanError::MissingOutput(o) => write!(f, "Missing output: {}", o),
            PlanError::InvalidConfiguration(s) => write!(f, "Invalid configuration: {}", s),
        }
    }
}

impl std::error::Error for PlanError {}

pub struct Planner;

impl LayoutPlan {
    pub fn new(outputs: HashMap<String, OutputState>) -> Self {
        Self {
            outputs,
            preset_used: None,
        }
    }

    pub fn with_preset_used(mut self, preset_used: impl Into<String>) -> Self {
        self.preset_used = Some(preset_used.into());
        self
    }
}

impl Planner {
    pub fn plan_from_profile(
        profile: &Profile,
        matched_outputs: &HashMap<String, String>,
        topology: &Topology,
    ) -> Result<LayoutPlan, PlanError> {
        let mut planned: HashMap<String, OutputState> = HashMap::new();

        for topo_name in matched_outputs.keys() {
            let output_state = topology.outputs.get(topo_name);
            let config = profile.layout.get(topo_name).cloned().or_else(|| {
                output_state.and_then(|state| {
                    profile
                        .layout
                        .values()
                        .find(|config| identities_match(&config.state.identity, &state.identity))
                        .cloned()
                })
            });

            let state = match (config, output_state) {
                (Some(mut cfg), Some(output)) => {
                    cfg.state.identity = output.identity.clone();
                    cfg.state
                }
                (Some(cfg), None) => cfg.state,
                (None, Some(state)) => state.clone(),
                (None, None) => return Err(PlanError::MissingOutput(topo_name.clone())),
            };

            planned.insert(topo_name.clone(), state);
        }

        Ok(LayoutPlan {
            outputs: planned,
            preset_used: None,
        })
    }

    pub fn plan_from_preset(
        preset: &str,
        topology: &Topology,
        primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        match preset {
            "off" => Self::plan_off(topology),
            "horizontal" | "vertical" | "horizontal-reverse" | "vertical-reverse" => {
                Self::plan_linear(topology, preset, primary_hint)
            }
            "common" => Self::plan_common(topology),
            "common-largest" => Self::plan_common_largest(topology),
            _ => Err(PlanError::UnsupportedPreset(preset.to_string())),
        }
    }

    fn plan_off(topology: &Topology) -> Result<LayoutPlan, PlanError> {
        if topology.outputs.is_empty() {
            return Err(PlanError::InvalidConfiguration(
                "No outputs to disable".to_string(),
            ));
        }

        let outputs = topology
            .outputs
            .iter()
            .map(|(name, state)| {
                let mut state = state.clone();
                state.enabled = false;
                state.position = Position { x: 0, y: 0 };
                state.mirror_target = None;
                (name.clone(), state)
            })
            .collect();

        Ok(LayoutPlan {
            outputs,
            preset_used: Some("off".to_string()),
        })
    }

    fn plan_linear(
        topology: &Topology,
        preset: &str,
        primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        let reverse = preset.ends_with("-reverse");
        let base_preset = preset.trim_end_matches("-reverse");

        let mut outputs = available_outputs(topology);
        if outputs.is_empty() {
            return Err(PlanError::InvalidConfiguration(
                "No outputs to arrange".to_string(),
            ));
        }

        if let Some(primary) = primary_hint {
            if let Some(pos) = outputs.iter().position(|(name, _)| name == primary) {
                outputs.rotate_left(pos);
            }
        }

        if reverse {
            outputs.reverse();
        }

        let is_horizontal = base_preset == "horizontal";
        let max_width = outputs
            .iter()
            .filter_map(|(_, state)| state.mode.map(|mode| mode.width as i32))
            .max()
            .unwrap_or(0);
        let mut x = 0i32;
        let mut y = 0i32;

        for (_, state) in &mut outputs {
            state.enabled = true;
            let position_x = if is_horizontal {
                x
            } else {
                let width = state.mode.map(|mode| mode.width as i32).unwrap_or(0);
                (max_width - width) / 2
            };
            state.position = Position { x: position_x, y };
            if let Some(mode) = &state.mode {
                if is_horizontal {
                    x += mode.width as i32;
                } else {
                    y += mode.height as i32;
                }
            }
            state.mirror_target = None;
        }

        Ok(LayoutPlan {
            outputs: outputs.into_iter().collect(),
            preset_used: Some(preset.to_string()),
        })
    }

    fn plan_common(topology: &Topology) -> Result<LayoutPlan, PlanError> {
        let outputs = available_outputs(topology);
        outputs.first().ok_or_else(|| {
            PlanError::InvalidConfiguration("No outputs available for common layout".to_string())
        })?;

        let mode = outputs
            .iter()
            .filter_map(|(_, state)| state.mode)
            .min_by_key(|mode| (mode.width * mode.height, mode.refresh))
            .ok_or_else(|| PlanError::InvalidConfiguration("No common mode found".to_string()))?;

        let mut planned = HashMap::new();
        for (name, state) in outputs {
            let mut state = state;
            state.enabled = true;
            state.mode = Some(mode);
            state.position = Position { x: 0, y: 0 };
            state.mirror_target = None;
            planned.insert(name, state);
        }

        Ok(LayoutPlan {
            outputs: planned,
            preset_used: Some("common".to_string()),
        })
    }

    fn plan_common_largest(topology: &Topology) -> Result<LayoutPlan, PlanError> {
        let outputs = available_outputs(topology);
        let (_primary, target_mode) = outputs
            .iter()
            .filter_map(|(name, state)| state.mode.map(|mode| (name.clone(), mode)))
            .max_by_key(|(_, mode)| mode.width * mode.height)
            .ok_or_else(|| {
                PlanError::InvalidConfiguration(
                    "No connected outputs with a mode found".to_string(),
                )
            })?;

        let mut planned = HashMap::new();
        for (name, mut state) in outputs {
            state.enabled = true;
            state.mode = Some(target_mode);
            state.position = Position { x: 0, y: 0 };
            state.mirror_target = None;
            planned.insert(name, state);
        }

        Ok(LayoutPlan {
            outputs: planned,
            preset_used: Some("common-largest".to_string()),
        })
    }
}

fn available_outputs(topology: &Topology) -> Vec<(String, OutputState)> {
    let mut outputs: Vec<_> = topology
        .outputs
        .iter()
        .filter(|(_, state)| !state.identity.is_ignored && !state.identity.is_virtual)
        .map(|(name, state)| (name.clone(), state.clone()))
        .collect();
    outputs.sort_by(|a, b| a.0.cmp(&b.0));
    outputs
}

pub fn detect_preset(topology: &Topology) -> Option<String> {
    let enabled: Vec<_> = topology
        .outputs
        .values()
        .filter(|s| !s.identity.is_ignored && !s.identity.is_virtual && s.enabled)
        .collect();

    if enabled.len() < 2 {
        return None;
    }

    let positions: Vec<_> = enabled
        .iter()
        .map(|s| (s.position.x, s.position.y))
        .collect();

    let same_y = positions.iter().all(|(_, y)| *y == positions[0].1);
    let same_x = positions.iter().all(|(x, _)| *x == positions[0].0);

    if same_y && !same_x {
        return Some("horizontal".to_string());
    }
    if same_x && !same_y {
        return Some("vertical".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, OutputIdentity, OutputState, Position, Transform};
    use crate::profile::{OutputConfig, OutputMatcher, Profile};

    fn output(connector: &str, width: u32, height: u32) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state.mode = Some(Mode::new(width, height, 60));
        state.position = Position::default();
        state.scale = 1.0;
        state.transform = Transform::Normal;
        state.mirror_target = None;
        state.backend_data = None;
        state
    }

    #[test]
    fn horizontal_reverse_uses_reverse_order() {
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 100, 50)),
                ("B".to_string(), output("B", 200, 50)),
            ]),
        };

        let plan = Planner::plan_from_preset("horizontal-reverse", &topology, None).unwrap();
        assert_eq!(plan.outputs["B"].position.x, 0);
        assert_eq!(plan.outputs["A"].position.x, 200);
    }

    #[test]
    fn common_clones_outputs_to_origin() {
        let mut b = output("B", 1280, 720);
        b.enabled = false;
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 1920, 1080)),
                ("B".to_string(), b),
            ]),
        };

        let plan = Planner::plan_from_preset("common", &topology, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
        assert!(plan.outputs["A"].enabled);
        assert!(plan.outputs["B"].enabled);
        assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1280, 720, 60)));
        assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1280, 720, 60)));
    }

    #[test]
    fn common_largest_uses_largest_mode_at_origin() {
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 1920, 1080)),
                ("B".to_string(), output("B", 2560, 1440)),
            ]),
        };

        let plan = Planner::plan_from_preset("common-largest", &topology, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["A"].mode, Some(Mode::new(2560, 1440, 60)));
        assert_eq!(plan.outputs["B"].mode, Some(Mode::new(2560, 1440, 60)));
    }

    #[test]
    fn off_disables_all_outputs() {
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 1920, 1080)),
                ("B".to_string(), output("B", 1280, 720)),
            ]),
        };

        let plan = Planner::plan_from_preset("off", &topology, None).unwrap();
        assert!(!plan.outputs["A"].enabled);
        assert!(!plan.outputs["B"].enabled);
    }

    #[test]
    fn horizontal_includes_disabled_connected_outputs() {
        let mut b = output("B", 1280, 720);
        b.enabled = false;
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 1920, 1080)),
                ("B".to_string(), b),
            ]),
        };

        let plan = Planner::plan_from_preset("horizontal", &topology, None).unwrap();
        assert!(plan.outputs["A"].enabled);
        assert!(plan.outputs["B"].enabled);
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(1920, 0));
    }

    #[test]
    fn vertical_centers_outputs_horizontally() {
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 3440, 1440)),
                ("B".to_string(), output("B", 2560, 1440)),
                ("C".to_string(), output("C", 1920, 1080)),
            ]),
        };

        let plan = Planner::plan_from_preset("vertical", &topology, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(440, 1440));
        assert_eq!(plan.outputs["C"].position, Position::new(760, 2880));
    }

    #[test]
    fn plan_from_profile_maps_layout_using_stable_identity() {
        let topology = Topology {
            outputs: HashMap::from([(
                "DP-1".to_string(),
                {
                    let mut state = OutputState::new("DP-1");
                    state.identity.make = Some("Microstep".to_string());
                    state.identity.model = Some("MSI MP273A".to_string());
                    state.identity.serial = Some("PB4H603B02982".to_string());
                    state.identity.description = Some("Microstep - MSI MP273A - DP-1".to_string());
                    state.enabled = true;
                    state.mode = Some(Mode::new(1920, 1080, 60));
                    state.position = Position::new(400, 200);
                    state.scale = 1.0;
                    state.transform = Transform::Normal;
                    state.mirror_target = None;
                    state.backend_data = None;
                    state
                },
            )]),
        };
        let profile = Profile {
            name: "default".to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: {
                    let mut identity = OutputIdentity::new("DP-4");
                    identity.make = Some("Microstep".to_string());
                    identity.model = Some("MSI MP273A".to_string());
                    identity.serial = Some("PB4H603B02982".to_string());
                    identity.description = Some("Microstep - MSI MP273A - DP-4".to_string());
                    identity
                },
                required: true,
                position_hint: Some(Position::new(0, 0)),
            }],
            layout: HashMap::from([(
                "DP-4".to_string(),
                OutputConfig {
                    state: {
                        let mut state = OutputState::new("DP-4");
                        state.identity.make = Some("Microstep".to_string());
                        state.identity.model = Some("MSI MP273A".to_string());
                        state.identity.serial = Some("PB4H603B02982".to_string());
                        state.identity.description = Some("Microstep - MSI MP273A - DP-4".to_string());
                        state.enabled = false;
                        state.mode = Some(Mode::new(1920, 1080, 60));
                        state.position = Position::new(0, 0);
                        state.scale = 1.0;
                        state.transform = Transform::Normal;
                        state.mirror_target = None;
                        state.backend_data = None;
                        state
                    },
                    preset: None,
                },
            )]),
            hooks: Default::default(),
            options: Default::default(),
        };

        let matched_outputs = HashMap::from([("DP-1".to_string(), "DP-1".to_string())]);
        let plan = Planner::plan_from_profile(&profile, &matched_outputs, &topology)
            .expect("plan should build");

        assert!(!plan.outputs["DP-1"].enabled);
        assert_eq!(plan.outputs["DP-1"].position, Position::new(0, 0));
        assert_eq!(
            plan.outputs["DP-1"].identity.connector.as_deref(),
            Some("DP-1")
        );
    }
}
