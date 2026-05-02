use std::collections::{HashMap, HashSet};

use super::{LayoutPlan, PlanError};
use crate::model::{
    identities_match, Mode, OutputIdentity, OutputState, Position, Topology, VirtualPreset,
};

pub(super) fn plan_from_preset(
    preset: VirtualPreset,
    topology: &Topology,
    builtin_output: Option<&OutputIdentity>,
    primary_hint: Option<&str>,
) -> Result<LayoutPlan, PlanError> {
    match preset {
        VirtualPreset::Off => plan_off(topology, builtin_output),
        VirtualPreset::External => plan_external(topology, builtin_output),
        VirtualPreset::Builtin => plan_builtin(topology, builtin_output),
        VirtualPreset::Horizontal
        | VirtualPreset::HorizontalReverse
        | VirtualPreset::Vertical
        | VirtualPreset::VerticalReverse => plan_linear(topology, preset, primary_hint),
        VirtualPreset::Common => plan_common(topology),
        VirtualPreset::Largest => plan_largest(topology),
        VirtualPreset::Mirror => plan_mirror(topology, primary_hint),
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

    let outputs: HashMap<_, _> = topology
        .outputs
        .iter()
        .map(|(name, state)| {
            let mut state = state.clone();
            state.enabled = has_internal_output && is_internal_output(&state, builtin_output);
            reset_preset_placement(&mut state);
            (name.clone(), state)
        })
        .collect();

    Ok(preset_plan(VirtualPreset::Off, outputs))
}

fn plan_external(
    topology: &Topology,
    builtin_output: Option<&OutputIdentity>,
) -> Result<LayoutPlan, PlanError> {
    if topology.outputs.is_empty() {
        return Err(PlanError::InvalidConfiguration(
            "No outputs to configure".to_string(),
        ));
    }

    let has_external_output = topology
        .outputs
        .values()
        .any(|state| is_real_output(state) && !is_internal_output(state, builtin_output));

    let mut outputs: HashMap<_, _> = topology
        .outputs
        .iter()
        .map(|(name, state)| {
            let mut state = state.clone();
            let enable_output = if state.identity.is_ignored || state.identity.is_virtual {
                state.enabled
            } else if has_external_output {
                !is_internal_output(&state, builtin_output)
            } else {
                is_internal_output(&state, builtin_output)
            };

            state.enabled = enable_output;
            if !enable_output {
                state.position = origin_position();
            }
            clear_mirror_target(&mut state);
            (name.clone(), state)
        })
        .collect();

    if let Some((min_x, min_y)) = outputs
        .values()
        .filter(|state| state.enabled && is_real_output(state))
        .map(|state| (state.position.x, state.position.y))
        .reduce(|(min_x, min_y), (x, y)| (min_x.min(x), min_y.min(y)))
    {
        for state in outputs.values_mut() {
            if state.enabled && is_real_output(state) {
                state.position.x -= min_x;
                state.position.y -= min_y;
            } else if !state.enabled {
                state.position = origin_position();
            }
        }
    }

    Ok(preset_plan(VirtualPreset::External, outputs))
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

    let outputs: HashMap<_, _> = topology
        .outputs
        .iter()
        .map(|(name, state)| {
            let mut state = state.clone();
            if !is_real_output(&state) {
                return (name.clone(), state);
            }

            state.enabled = is_internal_output(&state, builtin_output);
            reset_preset_placement(&mut state);
            (name.clone(), state)
        })
        .collect();

    Ok(preset_plan(VirtualPreset::Builtin, outputs))
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

    let mut outputs =
        available_outputs_with_primary_hint(topology, primary_hint, "No outputs to arrange")?;

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
        clear_mirror_target(state);
    }

    Ok(preset_plan(preset, outputs))
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
        enable_at_origin(&mut state);
        state.mode = Some(mode);
        planned.insert(name, state);
    }

    Ok(preset_plan(VirtualPreset::Common, planned))
}

fn plan_largest(topology: &Topology) -> Result<LayoutPlan, PlanError> {
    let outputs = available_outputs(topology);
    outputs.first().ok_or_else(|| {
        PlanError::InvalidConfiguration("No outputs available for largest layout".to_string())
    })?;

    let mut planned = HashMap::new();
    for (name, mut state) in outputs {
        enable_at_origin(&mut state);
        state.mode = Some(best_mode(&state)?);
        planned.insert(name, state);
    }

    Ok(preset_plan(VirtualPreset::Largest, planned))
}

fn plan_mirror(topology: &Topology, primary_hint: Option<&str>) -> Result<LayoutPlan, PlanError> {
    let outputs = available_outputs_with_primary_hint(
        topology,
        primary_hint,
        "No outputs available for mirrored layout",
    )?;

    let Some((primary_name, _)) = outputs.first() else {
        return Err(PlanError::InvalidConfiguration(
            "No outputs available for mirrored layout".to_string(),
        ));
    };
    let primary_name = primary_name.clone();
    let target_mode = mirror_mode(topology, &outputs)?;

    let mut planned = HashMap::new();
    for (name, mut state) in outputs {
        enable_at_origin(&mut state);
        state.mode = Some(target_mode);
        state.mirror_target = if name == primary_name {
            None
        } else {
            Some(primary_name.clone())
        };
        planned.insert(name, state);
    }

    Ok(preset_plan(VirtualPreset::Mirror, planned))
}

fn preset_plan(
    preset: VirtualPreset,
    outputs: impl IntoIterator<Item = (String, OutputState)>,
) -> LayoutPlan {
    LayoutPlan {
        outputs: outputs.into_iter().collect(),
        preset_used: Some(preset),
    }
}

fn origin_position() -> Position {
    Position { x: 0, y: 0 }
}

fn reset_preset_placement(state: &mut OutputState) {
    state.position = origin_position();
    clear_mirror_target(state);
}

fn enable_at_origin(state: &mut OutputState) {
    state.enabled = true;
    reset_preset_placement(state);
}

fn clear_mirror_target(state: &mut OutputState) {
    state.mirror_target = None;
}

fn is_real_output(state: &OutputState) -> bool {
    !state.identity.is_ignored && !state.identity.is_virtual
}

fn available_outputs(topology: &Topology) -> Vec<(String, OutputState)> {
    let mut outputs: Vec<_> = topology
        .outputs
        .iter()
        .filter(|(_, state)| is_real_output(state))
        .map(|(name, state)| (name.clone(), state.clone()))
        .collect();
    outputs.sort_by(|a, b| a.0.cmp(&b.0));
    outputs
}

fn available_outputs_with_primary_hint(
    topology: &Topology,
    primary_hint: Option<&str>,
    empty_message: &str,
) -> Result<Vec<(String, OutputState)>, PlanError> {
    let mut outputs = available_outputs(topology);
    if outputs.is_empty() {
        return Err(PlanError::InvalidConfiguration(empty_message.to_string()));
    }

    if let Some(primary) = primary_hint {
        if let Some(pos) = outputs.iter().position(|(name, _)| name == primary) {
            outputs.rotate_left(pos);
        }
    }

    Ok(outputs)
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
        .filter(|state| state.enabled && is_real_output(state))
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
fn topology_has_internal_output(
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
mod tests;
