use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::model::{
    identities_match, Mode, OutputIdentity, OutputState, Position, Topology, VirtualPreset,
};
use crate::profile::Profile;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LayoutPlan {
    pub outputs: HashMap<String, OutputState>,
    pub preset_used: Option<VirtualPreset>,
}

#[derive(Debug)]
pub enum PlanError {
    MissingOutput(String),
    InvalidConfiguration(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::MissingOutput(o) => write!(f, "Missing output: {o}"),
            PlanError::InvalidConfiguration(s) => write!(f, "Invalid configuration: {s}"),
        }
    }
}

impl std::error::Error for PlanError {}

pub struct Planner;

impl LayoutPlan {
    #[must_use]
    pub fn new(outputs: HashMap<String, OutputState>) -> Self {
        Self {
            outputs,
            preset_used: None,
        }
    }

    #[must_use]
    pub fn with_preset_used(mut self, preset_used: VirtualPreset) -> Self {
        self.preset_used = Some(preset_used);
        self
    }
}

impl Planner {
    /// Build a layout plan from a matched profile.
    ///
    /// # Errors
    /// Returns `MissingOutput` when a matched topology name cannot be resolved.
    pub fn plan_from_profile(
        profile: &Profile,
        matched_outputs: &HashMap<String, String>,
        topology: &Topology,
    ) -> Result<LayoutPlan, PlanError> {
        let mut planned: HashMap<String, OutputState> = HashMap::new();

        for (topo_name, layout_name) in matched_outputs {
            let output_state = topology.outputs.get(topo_name);
            let config = profile.layout.get(layout_name).cloned();

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

    /// Build a layout plan from a named preset.
    ///
    /// # Errors
    /// Returns `InvalidConfiguration` when the topology cannot satisfy the preset.
    pub fn plan_from_preset(
        preset: VirtualPreset,
        topology: &Topology,
        builtin_output: Option<&OutputIdentity>,
        primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        match preset {
            VirtualPreset::Off => Self::plan_off(topology, builtin_output),
            VirtualPreset::Builtin => Self::plan_builtin(topology, builtin_output),
            VirtualPreset::Horizontal
            | VirtualPreset::HorizontalReverse
            | VirtualPreset::Vertical
            | VirtualPreset::VerticalReverse => Self::plan_linear(topology, preset, primary_hint),
            VirtualPreset::Common => Self::plan_common(topology),
            VirtualPreset::Largest => Self::plan_largest(topology, primary_hint),
            VirtualPreset::Mirror => Self::plan_mirror(topology, primary_hint),
        }
    }

    fn plan_off(
        topology: &Topology,
        builtin_output: Option<&OutputIdentity>,
    ) -> Result<LayoutPlan, PlanError> {
        if topology.outputs.is_empty() {
            return Err(PlanError::InvalidConfiguration(
                "No outputs to disable".to_string(),
            ));
        }

        let has_internal_output = topology_has_internal_output(topology, builtin_output);

        let outputs = topology
            .outputs
            .iter()
            .map(|(name, state)| {
                let mut state = state.clone();
                state.enabled = has_internal_output && is_internal_output(&state, builtin_output);
                state.position = Position { x: 0, y: 0 };
                state.mirror_target = None;
                (name.clone(), state)
            })
            .collect();

        Ok(LayoutPlan {
            outputs,
            preset_used: Some(VirtualPreset::Off),
        })
    }

    fn plan_builtin(
        topology: &Topology,
        builtin_output: Option<&OutputIdentity>,
    ) -> Result<LayoutPlan, PlanError> {
        if topology.outputs.is_empty() {
            return Err(PlanError::InvalidConfiguration(
                "No outputs to configure".to_string(),
            ));
        }

        if !topology_has_internal_output(topology, builtin_output) {
            return Err(PlanError::InvalidConfiguration(
                "No built-in display is available for the `builtin` preset".to_string(),
            ));
        }

        let outputs = topology
            .outputs
            .iter()
            .map(|(name, state)| {
                let mut state = state.clone();
                let enable_output = if state.identity.is_ignored || state.identity.is_virtual {
                    state.enabled
                } else {
                    is_internal_output(&state, builtin_output)
                };

                state.enabled = enable_output;
                state.position = Position { x: 0, y: 0 };
                state.mirror_target = None;
                (name.clone(), state)
            })
            .collect();

        Ok(LayoutPlan {
            outputs,
            preset_used: Some(VirtualPreset::Builtin),
        })
    }
    fn plan_linear(
        topology: &Topology,
        preset: VirtualPreset,
        primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        let reverse = preset.is_reverse();
        let is_horizontal = matches!(
            preset,
            VirtualPreset::Horizontal | VirtualPreset::HorizontalReverse
        );

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

        let max_width = outputs
            .iter()
            .filter_map(|(_, state)| state.mode.and_then(|mode| i32::try_from(mode.width).ok()))
            .max()
            .unwrap_or(0);
        let mut x = 0i32;
        let mut y = 0i32;

        for (_, state) in &mut outputs {
            state.enabled = true;
            let position_x = if is_horizontal {
                x
            } else {
                let width = state
                    .mode
                    .map_or(0, |mode| i32::try_from(mode.width).unwrap_or(i32::MAX));
                (max_width - width) / 2
            };
            state.position = Position { x: position_x, y };
            if let Some(mode) = &state.mode {
                if is_horizontal {
                    x += i32::try_from(mode.width).unwrap_or(i32::MAX);
                } else {
                    y += i32::try_from(mode.height).unwrap_or(i32::MAX);
                }
            }
            state.mirror_target = None;
        }

        Ok(LayoutPlan {
            outputs: outputs.into_iter().collect(),
            preset_used: Some(preset),
        })
    }

    fn plan_common(topology: &Topology) -> Result<LayoutPlan, PlanError> {
        let outputs = available_outputs(topology);
        outputs.first().ok_or_else(|| {
            PlanError::InvalidConfiguration("No outputs available for common layout".to_string())
        })?;

        let mode = common_mode(&outputs)?;

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
            preset_used: Some(VirtualPreset::Common),
        })
    }

    fn plan_largest(
        topology: &Topology,
        _primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        let outputs = available_outputs(topology);
        outputs.first().ok_or_else(|| {
            PlanError::InvalidConfiguration("No outputs available for largest layout".to_string())
        })?;

        let mut planned = HashMap::new();
        for (name, mut state) in outputs {
            state.enabled = true;
            state.mode = Some(best_mode(&state)?);
            state.position = Position { x: 0, y: 0 };
            state.mirror_target = None;
            planned.insert(name, state);
        }

        Ok(LayoutPlan {
            outputs: planned,
            preset_used: Some(VirtualPreset::Largest),
        })
    }

    fn plan_mirror(
        topology: &Topology,
        primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        let mut outputs = available_outputs(topology);
        if outputs.is_empty() {
            return Err(PlanError::InvalidConfiguration(
                "No outputs available for mirrored layout".to_string(),
            ));
        }

        if let Some(primary) = primary_hint {
            if let Some(pos) = outputs.iter().position(|(name, _)| name == primary) {
                outputs.rotate_left(pos);
            }
        }

        let primary_name = outputs
            .first()
            .map(|(name, _)| name.clone())
            .ok_or_else(|| {
                PlanError::InvalidConfiguration(
                    "No outputs available for mirrored layout".to_string(),
                )
            })?;
        let target_mode = mirror_mode(topology, &outputs)?;

        let mut planned = HashMap::new();
        for (name, mut state) in outputs {
            state.enabled = true;
            state.mode = Some(target_mode);
            state.position = Position { x: 0, y: 0 };
            state.mirror_target = if name == primary_name {
                None
            } else {
                Some(primary_name.clone())
            };
            planned.insert(name, state);
        }

        Ok(LayoutPlan {
            outputs: planned,
            preset_used: Some(VirtualPreset::Mirror),
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

fn mode_area(mode: Mode) -> u64 {
    u64::from(mode.width) * u64::from(mode.height)
}

fn output_modes(state: &OutputState) -> Vec<Mode> {
    if state.available_modes.is_empty() {
        state.mode.into_iter().collect()
    } else {
        state.available_modes.clone()
    }
}

fn shared_modes(outputs: &[(String, OutputState)]) -> Vec<Mode> {
    let mut shared: Option<HashSet<Mode>> = None;
    for (_, state) in outputs {
        let modes: HashSet<Mode> = output_modes(state).into_iter().collect();
        shared = Some(match shared {
            Some(current) => current.intersection(&modes).copied().collect(),
            None => modes,
        });
    }

    let mut modes: Vec<Mode> = shared.unwrap_or_default().into_iter().collect();
    modes.sort_by_key(|mode| (mode_area(*mode), mode.refresh));
    modes
}

fn common_mode(outputs: &[(String, OutputState)]) -> Result<Mode, PlanError> {
    shared_modes(outputs)
        .into_iter()
        .max_by_key(|mode| (mode_area(*mode), mode.refresh))
        .ok_or_else(|| PlanError::InvalidConfiguration("No common mode found".to_string()))
}

fn best_mode(state: &OutputState) -> Result<Mode, PlanError> {
    output_modes(state)
        .into_iter()
        .max_by_key(|mode| (mode_area(*mode), mode.refresh))
        .ok_or_else(|| PlanError::InvalidConfiguration("No mode found for output".to_string()))
}

fn mirror_mode(topology: &Topology, outputs: &[(String, OutputState)]) -> Result<Mode, PlanError> {
    let shared = shared_modes(outputs);
    let active_floor = topology
        .outputs
        .values()
        .filter(|state| state.enabled && !state.identity.is_ignored && !state.identity.is_virtual)
        .filter_map(|state| state.mode)
        .min_by_key(|mode| (mode_area(*mode), mode.refresh));

    if shared.is_empty() {
        return Err(PlanError::InvalidConfiguration(
            "No common mode found".to_string(),
        ));
    }

    if let Some(active_floor) = active_floor {
        if let Some(exact) = shared.iter().copied().find(|mode| *mode == active_floor) {
            return Ok(exact);
        }

        if let Some(closest_not_larger) = shared
            .iter()
            .copied()
            .filter(|mode| mode.width <= active_floor.width && mode.height <= active_floor.height)
            .max_by_key(|mode| (mode_area(*mode), mode.refresh))
        {
            return Ok(closest_not_larger);
        }
    }

    shared
        .into_iter()
        .min_by_key(|mode| (mode_area(*mode), mode.refresh))
        .ok_or_else(|| PlanError::InvalidConfiguration("No common mode found".to_string()))
}

fn is_internal_output(state: &OutputState, builtin_output: Option<&OutputIdentity>) -> bool {
    if let Some(builtin_output) = builtin_output {
        return identities_match(builtin_output, &state.identity);
    }

    let connector = state
        .identity
        .connector
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let description = state
        .identity
        .description
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    connector.starts_with("edp")
        || connector.starts_with("lvds")
        || connector.starts_with("dsi")
        || description.contains("built-in")
        || description.contains("internal display")
}

#[must_use]
pub fn topology_has_internal_output(
    topology: &Topology,
    builtin_output: Option<&OutputIdentity>,
) -> bool {
    topology.outputs.values().any(|state| {
        !state.identity.is_ignored
            && !state.identity.is_virtual
            && is_internal_output(state, builtin_output)
    })
}

#[must_use]
pub fn detect_preset(topology: &Topology) -> Option<VirtualPreset> {
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
        return Some(VirtualPreset::Horizontal);
    }
    if same_x && !same_y {
        return Some(VirtualPreset::Vertical);
    }
    if enabled.iter().any(|state| state.mirror_target.is_some()) {
        return Some(VirtualPreset::Mirror);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, OutputIdentity, OutputState, Position, Transform, VirtualPreset};
    use crate::profile::{Hooks, OutputConfig, OutputMatcher, Profile};

    fn output(connector: &str, width: u32, height: u32) -> OutputState {
        let mut state = OutputState::new(connector);
        let mode = Mode::new(width, height, 60);
        state.enabled = true;
        state.mode = Some(mode);
        state.available_modes = vec![mode];
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

        let plan =
            Planner::plan_from_preset(VirtualPreset::HorizontalReverse, &topology, None, None)
                .unwrap();
        assert_eq!(plan.outputs["B"].position.x, 0);
        assert_eq!(plan.outputs["A"].position.x, 200);
    }

    #[test]
    fn common_clones_outputs_to_origin() {
        let mut a = output("A", 1920, 1080);
        a.available_modes = vec![Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)];
        let mut b = output("B", 2560, 1440);
        b.available_modes = vec![Mode::new(2560, 1440, 60), Mode::new(1920, 1080, 60)];
        b.enabled = false;
        let topology = Topology {
            outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
        };

        let plan = Planner::plan_from_preset(VirtualPreset::Common, &topology, None, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
        assert!(plan.outputs["A"].enabled);
        assert!(plan.outputs["B"].enabled);
        assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1920, 1080, 60)));
        assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1920, 1080, 60)));
    }

    #[test]
    fn largest_overlaps_outputs_at_each_outputs_best_mode() {
        let mut a = output("A", 1920, 1080);
        a.available_modes = vec![Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)];
        let mut b = output("B", 2560, 1440);
        b.available_modes = vec![Mode::new(2560, 1440, 60), Mode::new(1920, 1080, 60)];
        let topology = Topology {
            outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
        };

        let plan =
            Planner::plan_from_preset(VirtualPreset::Largest, &topology, None, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1920, 1080, 60)));
        assert_eq!(plan.outputs["B"].mode, Some(Mode::new(2560, 1440, 60)));
        assert_eq!(plan.outputs["A"].mirror_target, None);
        assert_eq!(plan.outputs["B"].mirror_target, None);
    }

    #[test]
    fn mirror_targets_secondary_outputs_at_primary() {
        let mut a = output("A", 1920, 1080);
        a.available_modes = vec![Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)];
        let mut b = output("B", 1280, 720);
        b.available_modes = vec![Mode::new(1280, 720, 60), Mode::new(1024, 768, 60)];
        let topology = Topology {
            outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
        };

        let plan = Planner::plan_from_preset(VirtualPreset::Mirror, &topology, None, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["A"].mirror_target, None);
        assert_eq!(plan.outputs["B"].mirror_target.as_deref(), Some("A"));
        assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1280, 720, 60)));
        assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1280, 720, 60)));
    }

    #[test]
    fn mirror_prefers_lowest_active_mode_over_lowest_supported_mode() {
        let mut a = output("A", 2560, 1440);
        a.available_modes = vec![
            Mode::new(2560, 1440, 60),
            Mode::new(1920, 1080, 60),
            Mode::new(800, 600, 60),
        ];
        let mut b = output("B", 1920, 1080);
        b.available_modes = vec![
            Mode::new(1920, 1080, 60),
            Mode::new(1280, 720, 60),
            Mode::new(800, 600, 60),
        ];
        let topology = Topology {
            outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
        };

        let plan = Planner::plan_from_preset(VirtualPreset::Mirror, &topology, None, None).unwrap();
        assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1920, 1080, 60)));
        assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1920, 1080, 60)));
    }

    #[test]
    fn off_keeps_internal_outputs_enabled_and_disables_externals() {
        let topology = Topology {
            outputs: HashMap::from([
                ("eDP-1".to_string(), {
                    let mut state = output("eDP-1", 1920, 1080);
                    state.identity.description = Some("Built-in display".to_string());
                    state
                }),
                ("DP-1".to_string(), output("DP-1", 1280, 720)),
            ]),
        };

        let plan = Planner::plan_from_preset(VirtualPreset::Off, &topology, None, None).unwrap();
        assert!(plan.outputs["eDP-1"].enabled);
        assert!(!plan.outputs["DP-1"].enabled);
    }

    #[test]
    fn off_disables_all_outputs_when_no_internal_output_exists() {
        let topology = Topology {
            outputs: HashMap::from([
                ("A".to_string(), output("A", 1920, 1080)),
                ("B".to_string(), output("B", 1280, 720)),
            ]),
        };

        let plan = Planner::plan_from_preset(VirtualPreset::Off, &topology, None, None).unwrap();
        assert!(!plan.outputs["A"].enabled);
        assert!(!plan.outputs["B"].enabled);
    }

    #[test]
    fn builtin_keeps_internal_outputs_enabled_and_disables_externals() {
        let topology = Topology {
            outputs: HashMap::from([
                ("eDP-1".to_string(), {
                    let mut state = output("eDP-1", 1920, 1080);
                    state.identity.description = Some("Built-in display".to_string());
                    state
                }),
                ("DP-1".to_string(), output("DP-1", 1280, 720)),
            ]),
        };

        let plan =
            Planner::plan_from_preset(VirtualPreset::Builtin, &topology, None, None).unwrap();
        assert!(plan.outputs["eDP-1"].enabled);
        assert!(!plan.outputs["DP-1"].enabled);
    }

    #[test]
    fn builtin_errors_when_no_internal_output_exists() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1", 1920, 1080))]),
        };

        let err = Planner::plan_from_preset(VirtualPreset::Builtin, &topology, None, None)
            .expect_err("builtin should require an internal output");
        assert_eq!(
            err.to_string(),
            "Invalid configuration: No built-in display is available for the `builtin` preset"
        );
    }

    #[test]
    fn builtin_uses_configured_output_override() {
        let topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), output("DP-1", 1920, 1080)),
                ("HDMI-A-1".to_string(), output("HDMI-A-1", 1280, 720)),
            ]),
        };
        let builtin_output = OutputIdentity::new("DP-1");

        let plan = Planner::plan_from_preset(
            VirtualPreset::Builtin,
            &topology,
            Some(&builtin_output),
            None,
        )
        .unwrap();
        assert!(plan.outputs["DP-1"].enabled);
        assert!(!plan.outputs["HDMI-A-1"].enabled);
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

        let plan =
            Planner::plan_from_preset(VirtualPreset::Horizontal, &topology, None, None).unwrap();
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

        let plan =
            Planner::plan_from_preset(VirtualPreset::Vertical, &topology, None, None).unwrap();
        assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
        assert_eq!(plan.outputs["B"].position, Position::new(440, 1440));
        assert_eq!(plan.outputs["C"].position, Position::new(760, 2880));
    }

    #[test]
    fn plan_from_profile_maps_layout_using_stable_identity() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), {
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
            })]),
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
                        state.identity.description =
                            Some("Microstep - MSI MP273A - DP-4".to_string());
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
            hooks: Hooks::default(),
        };

        let matched_outputs = HashMap::from([("DP-1".to_string(), "DP-4".to_string())]);
        let plan = Planner::plan_from_profile(&profile, &matched_outputs, &topology)
            .expect("plan should build");

        assert!(!plan.outputs["DP-1"].enabled);
        assert_eq!(plan.outputs["DP-1"].position, Position::new(0, 0));
        assert_eq!(
            plan.outputs["DP-1"].identity.connector.as_deref(),
            Some("DP-1")
        );
    }

    #[test]
    fn detect_preset_returns_virtual_preset() {
        let mut a = output("A", 100, 50);
        a.position = Position::new(0, 0);
        let mut b = output("B", 200, 50);
        b.position = Position::new(100, 0);
        let topology = Topology {
            outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
        };

        assert_eq!(detect_preset(&topology), Some(VirtualPreset::Horizontal));
    }
}
