use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use waytorandr_core::{ApplyResult, ConfigFailureKind, LayoutPlan, Mode, OutputState, Transform};

use super::state::{
    current_mode_for_output, mirror_target_name, round_refresh, transform_from_kscreen,
    KScreenConfig, KScreenMode, KScreenOutput,
};

pub(super) fn build_apply_args_or_rejection(
    plan: &LayoutPlan,
    config: &KScreenConfig,
) -> Result<Vec<String>, ApplyResult> {
    build_apply_args(plan, config).map_err(|source| {
        ApplyResult::failed(
            Some(ConfigFailureKind::Rejected),
            Some(format!("KScreen rejected the configuration: {source:#}")),
        )
    })
}

pub(super) fn build_apply_args(plan: &LayoutPlan, config: &KScreenConfig) -> Result<Vec<String>> {
    let connected = connected_outputs(config);

    let mut args = disable_commands(plan, &connected);
    args.extend(update_commands(plan, &connected)?);
    args.extend(mirror_commands(plan, &connected)?);
    Ok(args)
}

pub(super) struct ConnectedOutputs<'a> {
    pub(super) by_name: HashMap<&'a str, &'a KScreenOutput>,
    pub(super) ids_by_name: HashMap<&'a str, u32>,
    pub(super) names_by_id: HashMap<u32, &'a str>,
}

pub(super) fn connected_outputs(config: &KScreenConfig) -> ConnectedOutputs<'_> {
    let mut by_name = HashMap::new();
    let mut ids_by_name = HashMap::new();
    let mut names_by_id = HashMap::new();

    for output in config.outputs.iter().filter(|output| output.connected) {
        let name = output.name.as_str();
        by_name.insert(name, output);
        ids_by_name.insert(name, output.id);
        names_by_id.insert(output.id, name);
    }

    ConnectedOutputs {
        by_name,
        ids_by_name,
        names_by_id,
    }
}

fn disable_commands(plan: &LayoutPlan, connected: &ConnectedOutputs<'_>) -> Vec<String> {
    let mut commands = Vec::new();
    let mut current_outputs: Vec<&KScreenOutput> = connected.by_name.values().copied().collect();
    current_outputs.sort_by_key(|output| output.name.clone());

    for output in current_outputs {
        let disable = !plan
            .outputs
            .get(&output.name)
            .is_some_and(|desired| desired.enabled);

        if disable && output.enabled {
            commands.push(format!("output.{}.disable", output.name));
        }
    }

    commands
}

fn update_commands(plan: &LayoutPlan, connected: &ConnectedOutputs<'_>) -> Result<Vec<String>> {
    let mut commands = Vec::new();

    for output in enabled_connected_outputs(plan, connected)? {
        let name = output.name;
        let desired = output.desired;
        let current = output.current;

        if !current.enabled {
            commands.push(format!("output.{name}.enable"));
        }

        if current_mode_for_output(current) != desired.mode {
            let mode_arg = mode_command_for_output(current, desired.mode)
                .with_context(|| format!("failed to resolve mode for output `{name}`"))?;
            if let Some(mode_arg) = mode_arg {
                commands.push(format!("output.{name}.mode.{mode_arg}"));
            }
        }

        if !float_eq(current.scale, desired.scale) {
            commands.push(format!(
                "output.{name}.scale.{}",
                format_scale(desired.scale)
            ));
        }

        let current_transform = transform_from_kscreen(current.rotation);
        if current_transform != desired.transform {
            commands.push(format!(
                "output.{name}.rotation.{}",
                rotation_query(desired.transform)
            ));
        }

        if output.current_mirror_target.is_some() && output.desired_mirror_target.is_none() {
            commands.push(format!("output.{name}.mirror.none"));
        }

        if output.desired_mirror_target.is_none() && current.pos != desired.position {
            commands.push(format!(
                "output.{name}.position.{},{}",
                desired.position.x, desired.position.y
            ));
        }
    }

    Ok(commands)
}

fn mirror_commands(plan: &LayoutPlan, connected: &ConnectedOutputs<'_>) -> Result<Vec<String>> {
    let mut commands = Vec::new();
    for output in enabled_connected_outputs(plan, connected)? {
        if output.current_mirror_target.as_deref() != output.desired_mirror_target {
            if let Some(target_name) = output.desired_mirror_target {
                let target_id =
                    connected
                        .ids_by_name
                        .get(target_name)
                        .copied()
                        .ok_or_else(|| {
                            anyhow!(
                                "mirrored output `{}` targets disconnected output `{target_name}`",
                                output.name
                            )
                        })?;
                commands.push(format!("output.{}.mirror.{target_id}", output.name));
            }
        }
    }

    Ok(commands)
}

struct EnabledConnectedOutput<'a> {
    name: &'a str,
    desired: &'a OutputState,
    current: &'a KScreenOutput,
    current_mirror_target: Option<String>,
    desired_mirror_target: Option<&'a str>,
}

fn enabled_connected_outputs<'a>(
    plan: &'a LayoutPlan,
    connected: &'a ConnectedOutputs<'_>,
) -> Result<Vec<EnabledConnectedOutput<'a>>> {
    let mut desired_outputs: Vec<(&String, &OutputState)> = plan
        .outputs
        .iter()
        .filter(|(_, desired)| desired.enabled)
        .collect();
    desired_outputs.sort_by(|(left, _), (right, _)| left.cmp(right));

    desired_outputs
        .into_iter()
        .map(|(name, desired)| {
            let current = connected
                .by_name
                .get(name.as_str())
                .copied()
                .ok_or_else(|| {
                    anyhow!("output `{name}` is not connected on this KScreen session")
                })?;
            Ok(EnabledConnectedOutput {
                name,
                desired,
                current,
                current_mirror_target: mirror_target_name(current, &connected.names_by_id),
                desired_mirror_target: desired.mirror_target.as_deref(),
            })
        })
        .collect()
}

pub(super) fn mode_command_for_output(
    output: &KScreenOutput,
    desired: Option<Mode>,
) -> Result<Option<String>> {
    let Some(desired) = desired else {
        return Ok(None);
    };

    let candidate = output
        .modes
        .iter()
        .filter(|mode| mode.size.width == desired.width && mode.size.height == desired.height)
        .min_by(|left, right| compare_mode_preference(left, right, desired.refresh))
        .ok_or_else(|| {
            anyhow!(
                "no KScreen mode matches {}x{}@{} for output `{}`",
                desired.width,
                desired.height,
                desired.refresh,
                output.name
            )
        })?;

    Ok(Some(candidate.id.clone()))
}

fn compare_mode_preference(
    left: &KScreenMode,
    right: &KScreenMode,
    desired_refresh: u32,
) -> std::cmp::Ordering {
    let left_matches = round_refresh(left.refresh_rate) == desired_refresh;
    let right_matches = round_refresh(right.refresh_rate) == desired_refresh;

    right_matches
        .cmp(&left_matches)
        .then_with(|| {
            refresh_distance(left.refresh_rate, desired_refresh)
                .total_cmp(&refresh_distance(right.refresh_rate, desired_refresh))
        })
        .then_with(|| left.refresh_rate.total_cmp(&right.refresh_rate))
        .then_with(|| left.id.cmp(&right.id))
}

fn refresh_distance(refresh_rate: f64, desired_refresh: u32) -> f64 {
    (refresh_rate - f64::from(desired_refresh)).abs()
}

fn float_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.000_1
}

pub(super) fn format_scale(scale: f64) -> String {
    let mut formatted = scale.to_string();
    if formatted.contains('e') || formatted.contains('E') {
        formatted = format!("{scale:.4}");
    }
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn rotation_query(transform: Transform) -> &'static str {
    match transform {
        Transform::Normal => "normal",
        Transform::Rot90 => "left",
        Transform::Rot180 => "inverted",
        Transform::Rot270 => "right",
        Transform::Flipped => "flipped",
        Transform::Flipped90 => "flipped90",
        Transform::Flipped180 => "flipped180",
        Transform::Flipped270 => "flipped270",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use waytorandr_core::{LayoutPlan, Mode, OutputState, Position};

    use super::super::state::{KScreenConfig, KScreenMode, KScreenOutput, KScreenSize};
    use super::{
        build_apply_args_or_rejection, connected_outputs, format_scale, mode_command_for_output,
    };

    fn mode(id: &str, width: u32, height: u32, refresh_rate: f64) -> KScreenMode {
        KScreenMode {
            id: id.to_string(),
            refresh_rate,
            size: KScreenSize { width, height },
        }
    }

    fn kscreen_apply_output(name: &str, id: u32) -> KScreenOutput {
        KScreenOutput {
            name: name.to_string(),
            connected: true,
            enabled: true,
            id,
            current_mode_id: Some("current".to_string()),
            modes: vec![mode("current", 1920, 1080, 60.0)],
            pos: Position::default(),
            scale: 1.0,
            replication_source: 0,
            rotation: 1,
            size: KScreenSize {
                width: 1920,
                height: 1080,
            },
        }
    }

    #[test]
    fn connected_outputs_indexes_connected_outputs_only() {
        let mut disconnected = kscreen_apply_output("HDMI-A-2", 2);
        disconnected.connected = false;
        let config = KScreenConfig {
            outputs: vec![kscreen_apply_output("DP-1", 1), disconnected],
        };

        let connected = connected_outputs(&config);

        assert!(connected.by_name.contains_key("DP-1"));
        assert!(!connected.by_name.contains_key("HDMI-A-2"));
        assert_eq!(connected.ids_by_name.get("DP-1"), Some(&1));
        assert_eq!(connected.names_by_id.get(&1), Some(&"DP-1"));
    }

    #[test]
    fn mode_command_prefers_nearest_refresh_then_stable_id() -> anyhow::Result<()> {
        let output = KScreenOutput {
            modes: vec![
                mode("b", 1920, 1080, 59.95),
                mode("a", 1920, 1080, 59.96),
                mode("wrong-size", 1280, 720, 60.0),
            ],
            ..kscreen_apply_output("DP-1", 1)
        };

        assert_eq!(
            mode_command_for_output(
                &output,
                Some(Mode {
                    width: 1920,
                    height: 1080,
                    refresh: 60,
                }),
            )?,
            Some("a".to_string())
        );
        Ok(())
    }

    #[test]
    fn build_apply_args_or_rejection_reports_disconnected_output() {
        let config = KScreenConfig { outputs: vec![] };
        let mut desired = OutputState::new("DP-1");
        desired.enabled = true;
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), desired)]));

        let Err(result) = build_apply_args_or_rejection(&plan, &config) else {
            panic!("disconnected output should be rejected");
        };

        assert!(result
            .message()
            .is_some_and(|message| message.contains("not connected")));
    }

    #[test]
    fn format_scale_trims_human_redundancy() {
        assert_eq!(format_scale(1.0), "1");
        assert_eq!(format_scale(1.25), "1.25");
    }
}
