use super::*;
use anyhow::anyhow;
use std::collections::HashMap;
use waytorandr_core::{Mode, OutputState, Position};
use zbus::zvariant::OwnedValue;

type TestMonitorTuple = (
    (String, String, String, String),
    Vec<(String, i32, i32, f64, f64, Vec<f64>, PropertyMap)>,
    PropertyMap,
);
type TestLogicalMonitorTuple = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<(String, String, String, String)>,
    PropertyMap,
);

fn mode(
    id: &str,
    width: i32,
    height: i32,
    refresh: f64,
    current: bool,
    preferred: bool,
    supported_scales: Vec<f64>,
) -> (String, i32, i32, f64, f64, Vec<f64>, PropertyMap) {
    let mut properties = PropertyMap::new();
    if current {
        properties.insert("is-current".to_string(), OwnedValue::from(true));
    }
    if preferred {
        properties.insert("is-preferred".to_string(), OwnedValue::from(true));
    }
    (
        id.to_string(),
        width,
        height,
        refresh,
        1.0,
        supported_scales,
        properties,
    )
}

fn sample_state() -> CurrentState {
    CurrentState::from_reply((
        7,
        sample_monitors(),
        sample_logical_monitors(),
        sample_properties(),
    ))
}

fn sample_monitors() -> Vec<TestMonitorTuple> {
    let mut builtin_props = PropertyMap::new();
    builtin_props.insert(
        "display-name".to_string(),
        OwnedValue::from(zbus::zvariant::Str::from("Built-in display")),
    );
    builtin_props.insert("is-builtin".to_string(), OwnedValue::from(true));

    let mut external_props = PropertyMap::new();
    external_props.insert(
        "display-name".to_string(),
        OwnedValue::from(zbus::zvariant::Str::from("Acer 27")),
    );
    external_props.insert("is-underscanning".to_string(), OwnedValue::from(false));

    vec![
        (
            (
                "eDP-1".to_string(),
                "LEN".to_string(),
                "0x40ad".to_string(),
                "0x00000000".to_string(),
            ),
            vec![
                mode("1920x1080@60", 1920, 1080, 60.0, true, true, vec![1.0, 2.0]),
                mode("1280x720@60", 1280, 720, 60.0, false, false, vec![1.0]),
            ],
            builtin_props,
        ),
        (
            (
                "DP-1".to_string(),
                "ACR".to_string(),
                "VG270U P".to_string(),
                "serial".to_string(),
            ),
            vec![
                mode(
                    "2560x1440@144",
                    2560,
                    1440,
                    144.0,
                    true,
                    true,
                    vec![1.0, 2.0],
                ),
                mode(
                    "2560x1440@60",
                    2560,
                    1440,
                    60.0,
                    false,
                    false,
                    vec![1.0, 2.0],
                ),
                mode(
                    "1920x1080@60",
                    1920,
                    1080,
                    60.0,
                    false,
                    false,
                    vec![1.0, 2.0],
                ),
            ],
            external_props,
        ),
    ]
}

fn sample_logical_monitors() -> Vec<TestLogicalMonitorTuple> {
    vec![
        (
            0,
            0,
            1.0,
            0,
            true,
            vec![(
                "eDP-1".to_string(),
                "LEN".to_string(),
                "0x40ad".to_string(),
                "0x00000000".to_string(),
            )],
            PropertyMap::new(),
        ),
        (
            1920,
            0,
            1.0,
            0,
            false,
            vec![(
                "DP-1".to_string(),
                "ACR".to_string(),
                "VG270U P".to_string(),
                "serial".to_string(),
            )],
            PropertyMap::new(),
        ),
    ]
}

fn sample_properties() -> PropertyMap {
    let mut properties = PropertyMap::new();
    properties.insert("layout-mode".to_string(), OwnedValue::from(2u32));
    properties.insert(
        "supports-changing-layout-mode".to_string(),
        OwnedValue::from(true),
    );
    properties
}

#[test]
fn export_topology_marks_enabled_outputs_from_logical_monitors() {
    let topology = GnomeBackend::topology_from_current_state(&sample_state());

    assert_eq!(
        topology.outputs["eDP-1"].identity.make.as_deref(),
        Some("LEN")
    );
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(0, 0));
    assert!(topology.outputs["eDP-1"].enabled);
    assert_eq!(
        topology.outputs["DP-1"].mode,
        Some(Mode::new(2560, 1440, 144))
    );
    assert_eq!(topology.outputs["DP-1"].mirror_target, None);
}

#[test]
fn build_apply_config_preserves_current_primary_and_layout_mode() -> anyhow::Result<()> {
    let plan = LayoutPlan::new(HashMap::from([
        ("eDP-1".to_string(), {
            let mut output = OutputState::new("eDP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output
        }),
        ("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(2560, 1440, 60));
            output.position = Position::new(1920, 0);
            output
        }),
    ]));

    let (serial, logical_monitors, properties) = apply::build_apply_config(&sample_state(), &plan)?;

    assert_eq!(serial, 7);
    assert_eq!(logical_monitors.len(), 2);
    assert!(logical_monitors[0].4);
    assert_eq!(logical_monitors[1].5[0].1, "2560x1440@60");
    assert_eq!(u32::try_from(&properties["layout-mode"]).ok(), Some(2));
    Ok(())
}

#[test]
fn build_apply_config_groups_mirrored_outputs_into_one_logical_monitor() -> anyhow::Result<()> {
    let plan = LayoutPlan::new(HashMap::from([
        ("eDP-1".to_string(), {
            let mut output = OutputState::new("eDP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output
        }),
        ("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output.mirror_target = Some("eDP-1".to_string());
            output
        }),
    ]));

    let (_, logical_monitors, _) = apply::build_apply_config(&sample_state(), &plan)?;

    assert_eq!(logical_monitors.len(), 1);
    assert_eq!(logical_monitors[0].5.len(), 2);
    assert_eq!(logical_monitors[0].5[0].0, "eDP-1");
    assert_eq!(logical_monitors[0].5[1].0, "DP-1");
    Ok(())
}

#[test]
fn build_apply_config_rejects_mixed_mode_mirroring() -> anyhow::Result<()> {
    let plan = LayoutPlan::new(HashMap::from([
        ("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(2560, 1440, 60));
            output
        }),
        ("eDP-1".to_string(), {
            let mut output = OutputState::new("eDP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output.mirror_target = Some("DP-1".to_string());
            output
        }),
    ]));

    let Err(err) = apply::build_apply_config(&sample_state(), &plan) else {
        anyhow::bail!("mixed-mode mirroring should be rejected");
    };

    assert!(err
        .to_string()
        .contains("cannot mirror outputs with different modes"));
    Ok(())
}

#[test]
fn build_apply_config_groups_same_origin_outputs_into_one_logical_monitor() -> anyhow::Result<()> {
    let plan = LayoutPlan::new(HashMap::from([
        ("eDP-1".to_string(), {
            let mut output = OutputState::new("eDP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output.position = Position::new(0, 0);
            output
        }),
        ("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output.position = Position::new(0, 0);
            output
        }),
    ]));

    let (_, logical_monitors, _) = apply::build_apply_config(&sample_state(), &plan)?;

    assert_eq!(logical_monitors.len(), 1);
    assert_eq!(logical_monitors[0].5.len(), 2);
    assert_eq!(logical_monitors[0].5[0].0, "DP-1");
    assert_eq!(logical_monitors[0].5[1].0, "eDP-1");
    Ok(())
}

#[test]
fn build_apply_config_uses_target_mode_scales_for_logical_monitor() -> anyhow::Result<()> {
    let plan = LayoutPlan::new(HashMap::from([("eDP-1".to_string(), {
        let mut output = OutputState::new("eDP-1");
        output.enabled = true;
        output.mode = Some(Mode::new(1280, 720, 60));
        output.scale = 2.0;
        output
    })]));

    let (_, logical_monitors, _) = apply::build_apply_config(&sample_state(), &plan)?;

    assert_eq!(logical_monitors.len(), 1);
    assert!((logical_monitors[0].2 - 1.0).abs() < 0.000_1);
    assert_eq!(logical_monitors[0].5[0].1, "1280x720@60");
    Ok(())
}

#[test]
fn layout_properties_omits_layout_mode_when_mutter_does_not_allow_it() {
    let mut properties = PropertyMap::new();
    properties.insert("layout-mode".to_string(), OwnedValue::from(2u32));

    assert!(apply::layout_properties(&properties).is_empty());
}

#[test]
fn export_topology_marks_secondary_monitors_as_mirrored() {
    let mut mirrored_state = sample_state();
    mirrored_state.logical_monitors = vec![LogicalMonitorConfig {
        position: Position::new(0, 0),
        scale: 1.0,
        transform: 0,
        primary: true,
        connectors: vec!["eDP-1".to_string(), "DP-1".to_string()],
    }];

    let topology = GnomeBackend::topology_from_current_state(&mirrored_state);

    assert_eq!(topology.outputs["eDP-1"].mirror_target, None);
    assert_eq!(
        topology.outputs["DP-1"].mirror_target.as_deref(),
        Some("eDP-1")
    );
}

#[test]
fn apply_monitors_config_validation_error_is_structured_rejection() {
    let result = validation_rejection_from_apply_error(&anyhow!("serial is stale"));

    assert_eq!(result.failure(), Some(ConfigFailureKind::TopologyChanged));
    assert_eq!(
        result.message.as_deref(),
        Some("GNOME rejected the configuration: serial is stale")
    );
}

#[test]
fn apply_monitors_config_apply_error_is_structured_failure() {
    let result = apply_failure_from_apply_error(&anyhow!("layout rejected"));

    assert_eq!(result.failure(), Some(ConfigFailureKind::Rejected));
    assert_eq!(
        result.message(),
        Some("GNOME failed to apply the configuration: layout rejected")
    );
}
