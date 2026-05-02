use super::apply::{
    build_apply_args, build_apply_args_or_rejection, connected_outputs, format_scale,
    mode_command_for_output,
};
use super::*;
use std::collections::HashMap;
use waytorandr_core::{Mode, OutputIdentity, OutputState, Position, Transform};

const SAMPLE_CONFIG: &str = r#"{
    "outputs": [
        {
            "connected": true,
            "currentModeId": "1",
            "enabled": true,
            "id": 1,
            "modes": [
                {
                    "id": "1",
                    "refreshRate": 60.00199890136719,
                    "size": { "height": 1080, "width": 1920 }
                },
                {
                    "id": "2",
                    "refreshRate": 50,
                    "size": { "height": 1080, "width": 1920 }
                }
            ],
            "name": "eDP-1",
            "pos": { "x": 0, "y": 0 },
            "rotation": 1,
            "scale": 1,
            "size": { "height": 1080, "width": 1920 }
        },
        {
            "connected": true,
            "currentModeId": "3",
            "enabled": false,
            "id": 2,
            "modes": [
                {
                    "id": "3",
                    "refreshRate": 143.99899291992188,
                    "size": { "height": 1440, "width": 2560 }
                },
                {
                    "id": "4",
                    "refreshRate": 59.95100021362305,
                    "size": { "height": 1440, "width": 2560 }
                },
                {
                    "id": "5",
                    "refreshRate": 60,
                    "size": { "height": 1080, "width": 1920 }
                }
            ],
            "name": "DVI-I-1",
            "pos": { "x": 1920, "y": 0 },
            "replicationSource": 0,
            "rotation": 8,
            "scale": 1.25,
            "size": { "height": 1440, "width": 2560 }
        },
        {
            "connected": false,
            "enabled": false,
            "name": "HDMI-A-2",
            "pos": { "x": 0, "y": 0 },
            "rotation": 1,
            "scale": 1,
            "size": { "height": 0, "width": 0 }
        }
    ]
}"#;

fn sample_config() -> serde_json::Result<KScreenConfig> {
    serde_json::from_str(SAMPLE_CONFIG)
}

fn kscreen_output_state(connector: &str, enabled: bool) -> OutputState {
    let mut state = OutputState::new(connector);
    state.identity = OutputIdentity::new(connector);
    state.enabled = enabled;
    state
}

#[test]
fn command_label_uses_configured_command() {
    let backend = KScreenBackend {
        command: "/tmp/custom-kscreen-doctor".into(),
    };

    assert_eq!(backend.command_label(), "/tmp/custom-kscreen-doctor");
}

#[test]
fn export_topology_uses_connected_outputs_only() -> anyhow::Result<()> {
    let config = sample_config()?;
    let topology = export_kscreen_topology(&config);

    assert_eq!(topology.outputs.len(), 2);
    assert_eq!(
        topology.outputs["eDP-1"].mode,
        Some(Mode {
            width: 1920,
            height: 1080,
            refresh: 60
        })
    );
    assert_eq!(topology.outputs["DVI-I-1"].transform, Transform::Rot270);
    assert!((topology.outputs["DVI-I-1"].scale - 1.25).abs() < 0.000_1);
    assert!(!topology.outputs.contains_key("HDMI-A-2"));
    Ok(())
}

#[test]
fn export_topology_sets_mirror_targets_from_replication_source() -> anyhow::Result<()> {
    let mut config = sample_config()?;
    let Some(mirrored) = config
        .outputs
        .iter_mut()
        .find(|output| output.name == "DVI-I-1")
    else {
        anyhow::bail!("sample config should contain DVI-I-1");
    };
    mirrored.enabled = true;
    mirrored.replication_source = 1;

    let topology = export_kscreen_topology(&config);

    assert_eq!(topology.outputs["eDP-1"].mirror_target, None);
    assert_eq!(
        topology.outputs["DVI-I-1"].mirror_target.as_deref(),
        Some("eDP-1")
    );
    Ok(())
}

#[test]
fn connected_outputs_builds_all_indexes_in_one_connected_projection() -> anyhow::Result<()> {
    let config = sample_config()?;

    let connected = connected_outputs(&config);

    assert_eq!(connected.by_name.len(), 2);
    assert!(connected.by_name.contains_key("eDP-1"));
    assert!(!connected.by_name.contains_key("HDMI-A-2"));
    assert_eq!(connected.ids_by_name.get("DVI-I-1"), Some(&2));
    assert_eq!(connected.names_by_id.get(&1), Some(&"eDP-1"));
    assert!(!connected.names_by_id.contains_key(&0));
    Ok(())
}

#[test]
fn build_apply_args_disables_missing_outputs_and_updates_enabled_ones() -> anyhow::Result<()> {
    let config = sample_config()?;
    let mut laptop = kscreen_output_state("eDP-1", true);
    laptop.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 50,
    });
    laptop.position = Position { x: 0, y: 0 };

    let mut external = kscreen_output_state("DVI-I-1", true);
    external.mode = Some(Mode {
        width: 2560,
        height: 1440,
        refresh: 144,
    });
    external.position = Position { x: 1920, y: 0 };
    external.scale = 1.0;
    external.transform = Transform::Normal;

    let plan = LayoutPlan::new(HashMap::from([
        ("eDP-1".to_string(), laptop),
        ("DVI-I-1".to_string(), external),
    ]));

    let args = build_apply_args(&plan, &config)?;

    assert_eq!(
        args,
        vec![
            "output.DVI-I-1.enable".to_string(),
            "output.DVI-I-1.scale.1".to_string(),
            "output.DVI-I-1.rotation.normal".to_string(),
            "output.eDP-1.mode.2".to_string(),
        ]
    );
    Ok(())
}

#[test]
fn build_apply_args_can_disable_outputs_not_in_plan() -> anyhow::Result<()> {
    let config = sample_config()?;
    let mut laptop = kscreen_output_state("eDP-1", true);
    laptop.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 60,
    });

    let plan = LayoutPlan::new(HashMap::from([("eDP-1".to_string(), laptop)]));

    let args = build_apply_args(&plan, &config)?;

    assert!(args.is_empty());
    Ok(())
}

#[test]
fn build_apply_args_or_rejection_returns_structured_rejection_for_disconnected_output(
) -> anyhow::Result<()> {
    let config = sample_config()?;
    let mut disconnected = kscreen_output_state("HDMI-A-2", true);
    disconnected.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 60,
    });
    let plan = LayoutPlan::new(HashMap::from([("HDMI-A-2".to_string(), disconnected)]));

    let Err(result) = build_apply_args_or_rejection(&plan, &config) else {
        anyhow::bail!("disconnected output should be a structured apply rejection");
    };

    assert_eq!(result.failure(), Some(ConfigFailureKind::Rejected));
    assert_eq!(
        result.message(),
        Some("KScreen rejected the configuration: output `HDMI-A-2` is not connected on this KScreen session")
    );
    Ok(())
}

#[test]
fn build_apply_args_uses_native_kscreen_mirror_commands() -> anyhow::Result<()> {
    let config = sample_config()?;

    let mut laptop = kscreen_output_state("eDP-1", true);
    laptop.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 60,
    });
    laptop.position = Position { x: 0, y: 0 };

    let mut external = kscreen_output_state("DVI-I-1", true);
    external.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 60,
    });
    external.scale = 1.0;
    external.transform = Transform::Normal;
    external.mirror_target = Some("eDP-1".to_string());

    let plan = LayoutPlan::new(HashMap::from([
        ("eDP-1".to_string(), laptop),
        ("DVI-I-1".to_string(), external),
    ]));

    let args = build_apply_args(&plan, &config)?;

    assert_eq!(
        args,
        vec![
            "output.DVI-I-1.enable".to_string(),
            "output.DVI-I-1.mode.5".to_string(),
            "output.DVI-I-1.scale.1".to_string(),
            "output.DVI-I-1.rotation.normal".to_string(),
            "output.DVI-I-1.mirror.1".to_string(),
        ]
    );
    Ok(())
}

#[test]
fn build_apply_args_clears_mirror_before_setting_position() -> anyhow::Result<()> {
    let mut config = sample_config()?;
    let Some(mirrored) = config
        .outputs
        .iter_mut()
        .find(|output| output.name == "DVI-I-1")
    else {
        anyhow::bail!("sample config should contain DVI-I-1");
    };
    mirrored.enabled = true;
    mirrored.replication_source = 1;

    let mut laptop = kscreen_output_state("eDP-1", true);
    laptop.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 60,
    });
    laptop.position = Position { x: 0, y: 0 };

    let mut external = kscreen_output_state("DVI-I-1", true);
    external.mode = Some(Mode {
        width: 1920,
        height: 1080,
        refresh: 60,
    });
    external.position = Position { x: 1920, y: 0 };
    external.scale = 1.0;
    external.transform = Transform::Normal;

    let plan = LayoutPlan::new(HashMap::from([
        ("eDP-1".to_string(), laptop),
        ("DVI-I-1".to_string(), external),
    ]));

    let args = build_apply_args(&plan, &config)?;

    assert_eq!(
        args,
        vec![
            "output.DVI-I-1.mode.5".to_string(),
            "output.DVI-I-1.scale.1".to_string(),
            "output.DVI-I-1.rotation.normal".to_string(),
            "output.DVI-I-1.mirror.none".to_string(),
        ]
    );
    Ok(())
}

#[test]
fn mode_command_falls_back_to_nearest_refresh_for_matching_resolution() -> anyhow::Result<()> {
    let config = sample_config()?;
    let Some(output) = config
        .outputs
        .iter()
        .find(|output| output.name == "DVI-I-1")
    else {
        anyhow::bail!("sample config should contain DVI-I-1");
    };

    let mode = mode_command_for_output(
        output,
        Some(Mode {
            width: 2560,
            height: 1440,
            refresh: 59,
        }),
    )?;

    assert_eq!(mode.as_deref(), Some("4"));
    Ok(())
}

#[test]
fn format_scale_trims_redundant_zeroes() {
    assert_eq!(format_scale(1.0), "1");
    assert_eq!(format_scale(1.25), "1.25");
    assert_eq!(format_scale(1.5), "1.5");
}
