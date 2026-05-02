use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Context, Result};
use zbus::zvariant::OwnedValue;

use waytorandr_core::{LayoutPlan, Mode, OutputState, Position, Transform};

use super::state::{
    current_monitor_mode, float_eq, property_as_bool, property_as_u32, round_refresh, CurrentState,
    MonitorConfig, MonitorMode, PropertyMap,
};

type ApplyMonitorTuple = (String, String, PropertyMap);
pub(super) type ApplyLogicalMonitorTuple = (i32, i32, f64, u32, bool, Vec<ApplyMonitorTuple>);

pub(super) fn build_apply_config(
    state: &CurrentState,
    plan: &LayoutPlan,
) -> Result<(u32, Vec<ApplyLogicalMonitorTuple>, PropertyMap)> {
    let primary_connector = select_primary_connector(state, plan);
    let mut enabled_outputs: Vec<(&String, &OutputState)> = plan
        .outputs
        .iter()
        .filter(|(_, output)| output.enabled)
        .collect();
    enabled_outputs.sort_by(|(left_name, left), (right_name, right)| {
        left.position
            .y
            .cmp(&right.position.y)
            .then(left.position.x.cmp(&right.position.x))
            .then(left_name.cmp(right_name))
    });

    let mut logical_monitors = Vec::new();
    for group in build_logical_groups(&enabled_outputs)? {
        logical_monitors.push(build_logical_monitor_tuple(
            state,
            &group,
            primary_connector.as_deref(),
        )?);
    }

    Ok((
        state.serial,
        logical_monitors,
        layout_properties(&state.properties),
    ))
}

fn build_logical_monitor_tuple(
    state: &CurrentState,
    group: &[(&String, &OutputState)],
    primary_connector: Option<&str>,
) -> Result<ApplyLogicalMonitorTuple> {
    let (group_root_name, group_root_desired) = group
        .first()
        .copied()
        .ok_or_else(|| anyhow!("empty mirrored output group"))?;
    let group_root_monitor = state.monitor(group_root_name).ok_or_else(|| {
        anyhow!("output `{group_root_name}` is not connected on this GNOME session")
    })?;
    let group_root_mode = resolve_mode(group_root_monitor, group_root_desired.mode)
        .with_context(|| format!("failed to resolve mode for output `{group_root_name}`"))?;
    let logical_is_primary =
        primary_connector.is_some_and(|connector| group_contains_connector(group, connector));

    let mut apply_monitors = Vec::new();
    let mut group_modes = HashSet::new();
    for (name, desired) in group {
        let monitor = state
            .monitor(name)
            .ok_or_else(|| anyhow!("output `{name}` is not connected on this GNOME session"))?;
        let mode = if name.as_str() == group_root_name.as_str() {
            group_root_mode
        } else {
            resolve_mode(monitor, desired.mode)
                .with_context(|| format!("failed to resolve mode for output `{name}`"))?
        };
        group_modes.insert(mode_signature(mode));
        apply_monitors.push((
            (*name).clone(),
            mode.id.clone(),
            monitor_apply_properties(monitor),
        ));
    }
    if group_modes.len() > 1 {
        bail!(
            "the `gnome` backend cannot mirror outputs with different modes in one logical monitor; use `waytorandr set mirror` or `waytorandr set common` instead"
        );
    }

    Ok((
        group_root_desired.position.x,
        group_root_desired.position.y,
        resolve_scale(group_root_mode, group_root_desired.scale),
        transform_to_gnome(group_root_desired.transform),
        logical_is_primary,
        apply_monitors,
    ))
}

fn group_contains_connector(group: &[(&String, &OutputState)], connector: &str) -> bool {
    group.iter().any(|(name, _)| name.as_str() == connector)
}

fn resolve_mode(monitor: &MonitorConfig, desired: Option<Mode>) -> Result<&MonitorMode> {
    if let Some(desired) = desired {
        let mut candidates: Vec<&MonitorMode> = monitor
            .modes
            .iter()
            .filter(|mode| mode.width == desired.width && mode.height == desired.height)
            .collect();
        candidates.sort_by(|left, right| {
            refresh_distance(left.refresh, desired.refresh)
                .partial_cmp(&refresh_distance(right.refresh, desired.refresh))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        return candidates.into_iter().next().ok_or_else(|| {
            anyhow!(
                "no matching mode for {}x{}@{} on `{}`",
                desired.width,
                desired.height,
                desired.refresh,
                monitor.connector
            )
        });
    }

    current_monitor_mode(monitor)
        .or_else(|| monitor.modes.iter().find(|mode| mode.preferred_scale > 0.0))
        .or_else(|| monitor.modes.first())
        .ok_or_else(|| anyhow!("monitor `{}` does not expose any modes", monitor.connector))
}

fn select_primary_connector(state: &CurrentState, plan: &LayoutPlan) -> Option<String> {
    let enabled_names: Vec<&str> = plan
        .outputs
        .iter()
        .filter(|(_, output)| output.enabled)
        .map(|(name, _)| name.as_str())
        .collect();
    if enabled_names.is_empty() {
        return None;
    }

    let current_primary = state
        .logical_by_connector()
        .into_iter()
        .find_map(|(connector, logical)| logical.primary.then_some(connector.to_string()));
    if current_primary
        .as_deref()
        .is_some_and(|connector| enabled_names.iter().any(|name| name == &connector))
    {
        return current_primary;
    }

    let mut sorted_enabled = enabled_names;
    sorted_enabled.sort_unstable();
    sorted_enabled.first().map(|name| (*name).to_string())
}

pub(super) fn layout_properties(properties: &PropertyMap) -> PropertyMap {
    let mut apply_properties = PropertyMap::new();
    if property_as_bool(properties, "supports-changing-layout-mode").unwrap_or(false) {
        if let Some(layout_mode) = property_as_u32(properties, "layout-mode") {
            apply_properties.insert("layout-mode".to_string(), OwnedValue::from(layout_mode));
        }
    }
    apply_properties
}

fn monitor_apply_properties(monitor: &MonitorConfig) -> PropertyMap {
    let mut properties = PropertyMap::new();
    if let Some(underscanning) = property_as_bool(&monitor.properties, "is-underscanning") {
        properties.insert("underscanning".to_string(), OwnedValue::from(underscanning));
    }
    properties
}

fn resolve_scale(mode: &MonitorMode, desired: f64) -> f64 {
    if mode.supported_scales.is_empty() {
        return desired;
    }
    if let Some(exact) = mode
        .supported_scales
        .iter()
        .copied()
        .find(|scale| float_eq(*scale, desired))
    {
        return exact;
    }

    if mode.preferred_scale > 0.0
        && mode
            .supported_scales
            .iter()
            .any(|scale| float_eq(*scale, mode.preferred_scale))
    {
        return mode.preferred_scale;
    }

    mode.supported_scales
        .iter()
        .copied()
        .min_by(|left, right| {
            scale_distance(*left, desired)
                .total_cmp(&scale_distance(*right, desired))
                .then_with(|| left.total_cmp(right))
        })
        .unwrap_or(desired)
}

fn scale_distance(scale: f64, desired: f64) -> f64 {
    (scale - desired).abs()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LogicalGroupKey {
    position: Position,
    mode: Option<Mode>,
    scale_bits: u64,
    transform: Transform,
}

impl LogicalGroupKey {
    fn from_output(output: &OutputState) -> Self {
        Self {
            position: output.position,
            mode: output.mode,
            scale_bits: output.scale.to_bits(),
            transform: output.transform,
        }
    }
}

fn build_logical_groups<'a>(
    enabled_outputs: &[(&'a String, &'a OutputState)],
) -> Result<Vec<Vec<(&'a String, &'a OutputState)>>> {
    let by_name: HashMap<&str, &OutputState> = enabled_outputs
        .iter()
        .map(|(name, state)| (name.as_str(), *state))
        .collect();
    let mut grouped: HashMap<&str, Vec<(&'a String, &'a OutputState)>> = HashMap::new();
    let mut root_order = Vec::new();
    let mut implicit_roots: HashMap<LogicalGroupKey, &str> = HashMap::new();

    for (name, desired) in enabled_outputs {
        let root_name = if let Some(root_name) = desired.mirror_target.as_deref() {
            root_name
        } else {
            let group_key = LogicalGroupKey::from_output(desired);
            implicit_roots.get(&group_key).copied().unwrap_or_else(|| {
                implicit_roots.insert(group_key, name.as_str());
                name.as_str()
            })
        };
        let Some(root_state) = by_name.get(root_name) else {
            bail!("mirrored output `{name}` targets disconnected output `{root_name}`");
        };
        if desired.mirror_target.is_some() && root_state.mirror_target.is_some() {
            bail!("mirrored output `{name}` targets `{root_name}`, which is itself mirrored");
        }
        if !root_order.contains(&root_name) {
            root_order.push(root_name);
        }
        grouped
            .entry(root_name)
            .or_default()
            .push((*name, *desired));
    }

    let mut groups = Vec::new();
    for root_name in root_order {
        let mut group = grouped
            .remove(root_name)
            .ok_or_else(|| anyhow!("missing mirrored output group for `{root_name}`"))?;
        group.sort_by(|(left_name, left), (right_name, right)| {
            left.mirror_target
                .is_some()
                .cmp(&right.mirror_target.is_some())
                .then(left_name.cmp(right_name))
        });
        groups.push(group);
    }

    Ok(groups)
}

fn refresh_distance(refresh: f64, desired_refresh: u32) -> f64 {
    (refresh - f64::from(desired_refresh)).abs()
}

fn mode_signature(mode: &MonitorMode) -> (u32, u32, u32) {
    (mode.width, mode.height, round_refresh(mode.refresh))
}

fn transform_to_gnome(value: Transform) -> u32 {
    match value {
        Transform::Normal => 0,
        Transform::Rot90 => 1,
        Transform::Rot180 => 2,
        Transform::Rot270 => 3,
        Transform::Flipped => 4,
        Transform::Flipped90 => 5,
        Transform::Flipped180 => 6,
        Transform::Flipped270 => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_properties_preserves_supported_layout_mode() {
        let mut properties = PropertyMap::new();
        properties.insert(
            "supports-changing-layout-mode".to_string(),
            OwnedValue::from(true),
        );
        properties.insert("layout-mode".to_string(), OwnedValue::from(2_u32));

        let apply_properties = layout_properties(&properties);

        assert_eq!(property_as_u32(&apply_properties, "layout-mode"), Some(2));
    }

    #[test]
    fn transform_to_gnome_maps_rotation_values() {
        assert_eq!(transform_to_gnome(Transform::Normal), 0);
        assert_eq!(transform_to_gnome(Transform::Rot90), 1);
        assert_eq!(transform_to_gnome(Transform::Flipped270), 7);
    }
}
