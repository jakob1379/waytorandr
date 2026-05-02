use super::*;
use crate::model::{Mode, OutputIdentity, OutputState, Position, Transform, VirtualPreset};
use crate::profile::{Hooks, OutputConfig, OutputMatcher, Profile};

fn planner_output_state(connector: &str, width: u32, height: u32) -> OutputState {
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
fn horizontal_reverse_uses_reverse_order() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("A".to_string(), planner_output_state("A", 100, 50)),
            ("B".to_string(), planner_output_state("B", 200, 50)),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::HorizontalReverse, &topology, None, None)?;
    assert_eq!(plan.outputs["B"].position.x, 0);
    assert_eq!(plan.outputs["A"].position.x, 200);
    Ok(())
}

#[test]
fn common_clones_outputs_to_origin() -> anyhow::Result<()> {
    let mut a = planner_output_state("A", 1920, 1080);
    a.available_modes = vec![Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)];
    let mut b = planner_output_state("B", 2560, 1440);
    b.available_modes = vec![Mode::new(2560, 1440, 60), Mode::new(1920, 1080, 60)];
    b.enabled = false;
    let topology = Topology {
        outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Common, &topology, None, None)?;
    assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
    assert!(plan.outputs["A"].enabled);
    assert!(plan.outputs["B"].enabled);
    assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1920, 1080, 60)));
    assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1920, 1080, 60)));
    Ok(())
}

#[test]
fn largest_overlaps_outputs_at_each_outputs_best_mode() -> anyhow::Result<()> {
    let mut a = planner_output_state("A", 1920, 1080);
    a.available_modes = vec![Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)];
    let mut b = planner_output_state("B", 2560, 1440);
    b.available_modes = vec![Mode::new(2560, 1440, 60), Mode::new(1920, 1080, 60)];
    let topology = Topology {
        outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Largest, &topology, None, None)?;
    assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1920, 1080, 60)));
    assert_eq!(plan.outputs["B"].mode, Some(Mode::new(2560, 1440, 60)));
    assert_eq!(plan.outputs["A"].mirror_target, None);
    assert_eq!(plan.outputs["B"].mirror_target, None);
    Ok(())
}

#[test]
fn mirror_targets_secondary_outputs_at_primary() -> anyhow::Result<()> {
    let mut a = planner_output_state("A", 1920, 1080);
    a.available_modes = vec![Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)];
    let mut b = planner_output_state("B", 1280, 720);
    b.available_modes = vec![Mode::new(1280, 720, 60), Mode::new(1024, 768, 60)];
    let topology = Topology {
        outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Mirror, &topology, None, None)?;
    assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["B"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["A"].mirror_target, None);
    assert_eq!(plan.outputs["B"].mirror_target.as_deref(), Some("A"));
    assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1280, 720, 60)));
    assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1280, 720, 60)));
    Ok(())
}

#[test]
fn mirror_prefers_lowest_active_mode_over_lowest_supported_mode() -> anyhow::Result<()> {
    let mut a = planner_output_state("A", 2560, 1440);
    a.available_modes = vec![
        Mode::new(2560, 1440, 60),
        Mode::new(1920, 1080, 60),
        Mode::new(800, 600, 60),
    ];
    let mut b = planner_output_state("B", 1920, 1080);
    b.available_modes = vec![
        Mode::new(1920, 1080, 60),
        Mode::new(1280, 720, 60),
        Mode::new(800, 600, 60),
    ];
    let topology = Topology {
        outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Mirror, &topology, None, None)?;
    assert_eq!(plan.outputs["A"].mode, Some(Mode::new(1920, 1080, 60)));
    assert_eq!(plan.outputs["B"].mode, Some(Mode::new(1920, 1080, 60)));
    Ok(())
}

#[test]
fn off_keeps_internal_outputs_enabled_and_disables_externals() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), {
                let mut state = planner_output_state("eDP-1", 1920, 1080);
                state.identity.description = Some("Built-in display".to_string());
                state
            }),
            ("DP-1".to_string(), planner_output_state("DP-1", 1280, 720)),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Off, &topology, None, None)?;
    assert!(plan.outputs["eDP-1"].enabled);
    assert!(!plan.outputs["DP-1"].enabled);
    Ok(())
}

#[test]
fn off_disables_all_outputs_when_no_internal_output_exists() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("A".to_string(), planner_output_state("A", 1920, 1080)),
            ("B".to_string(), planner_output_state("B", 1280, 720)),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Off, &topology, None, None)?;
    assert!(!plan.outputs["A"].enabled);
    assert!(!plan.outputs["B"].enabled);
    Ok(())
}

#[test]
fn external_disables_internal_outputs_and_keeps_externals() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), {
                let mut state = planner_output_state("eDP-1", 1920, 1080);
                state.identity.description = Some("Built-in display".to_string());
                state
            }),
            ("DP-1".to_string(), planner_output_state("DP-1", 1280, 720)),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::External, &topology, None, None)?;
    assert!(!plan.outputs["eDP-1"].enabled);
    assert!(plan.outputs["DP-1"].enabled);
    Ok(())
}

#[test]
fn external_keeps_internal_outputs_when_no_external_exists() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([("eDP-1".to_string(), {
            let mut state = planner_output_state("eDP-1", 1920, 1080);
            state.identity.description = Some("Built-in display".to_string());
            state
        })]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::External, &topology, None, None)?;
    assert!(plan.outputs["eDP-1"].enabled);
    Ok(())
}

#[test]
fn external_rebases_enabled_outputs_to_origin() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), {
                let mut state = planner_output_state("eDP-1", 1920, 1080);
                state.identity.description = Some("Built-in display".to_string());
                state.position = Position::new(0, 0);
                state
            }),
            ("DP-1".to_string(), {
                let mut state = planner_output_state("DP-1", 2560, 1440);
                state.position = Position::new(1920, 40);
                state
            }),
            ("HDMI-A-1".to_string(), {
                let mut state = planner_output_state("HDMI-A-1", 1920, 1080);
                state.position = Position::new(4480, 40);
                state
            }),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::External, &topology, None, None)?;
    assert!(!plan.outputs["eDP-1"].enabled);
    assert_eq!(plan.outputs["eDP-1"].position, Position::new(0, 0));
    assert!(plan.outputs["DP-1"].enabled);
    assert!(plan.outputs["HDMI-A-1"].enabled);
    assert_eq!(plan.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["HDMI-A-1"].position, Position::new(2560, 0));
    Ok(())
}

#[test]
fn external_does_not_rebase_enabled_ignored_or_virtual_outputs() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), {
                let mut state = planner_output_state("eDP-1", 1920, 1080);
                state.identity.description = Some("Built-in display".to_string());
                state.position = Position::new(0, 0);
                state
            }),
            ("DP-1".to_string(), {
                let mut state = planner_output_state("DP-1", 2560, 1440);
                state.position = Position::new(1920, 40);
                state
            }),
            ("VIRT-1".to_string(), {
                let mut state = planner_output_state("VIRT-1", 1920, 1080);
                state.identity.is_virtual = true;
                state.position = Position::new(7000, 500);
                state
            }),
            ("IGNORED-1".to_string(), {
                let mut state = planner_output_state("IGNORED-1", 800, 600);
                state.identity.is_ignored = true;
                state.position = Position::new(-300, 250);
                state
            }),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::External, &topology, None, None)?;
    assert_eq!(plan.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["VIRT-1"].position, Position::new(7000, 500));
    assert_eq!(plan.outputs["IGNORED-1"].position, Position::new(-300, 250));
    Ok(())
}

#[test]
fn builtin_keeps_internal_outputs_enabled_and_disables_externals() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), {
                let mut state = planner_output_state("eDP-1", 1920, 1080);
                state.identity.description = Some("Built-in display".to_string());
                state
            }),
            ("DP-1".to_string(), planner_output_state("DP-1", 1280, 720)),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Builtin, &topology, None, None)?;
    assert!(plan.outputs["eDP-1"].enabled);
    assert!(!plan.outputs["DP-1"].enabled);
    Ok(())
}

#[test]
fn builtin_errors_when_no_internal_output_exists() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([("DP-1".to_string(), planner_output_state("DP-1", 1920, 1080))]),
    };

    let Err(err) = Planner::plan_from_preset(VirtualPreset::Builtin, &topology, None, None) else {
        anyhow::bail!("builtin should require an internal output");
    };
    assert_eq!(
        err.to_string(),
        "Invalid configuration: No built-in display is available for the `builtin` preset"
    );
    Ok(())
}

#[test]
fn builtin_uses_configured_output_override() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("DP-1".to_string(), planner_output_state("DP-1", 1920, 1080)),
            (
                "HDMI-A-1".to_string(),
                planner_output_state("HDMI-A-1", 1280, 720),
            ),
        ]),
    };
    let builtin_output = OutputIdentity::new("DP-1");

    let plan = Planner::plan_from_preset(
        VirtualPreset::Builtin,
        &topology,
        Some(&builtin_output),
        None,
    )?;
    assert!(plan.outputs["DP-1"].enabled);
    assert!(!plan.outputs["HDMI-A-1"].enabled);
    Ok(())
}

#[test]
fn builtin_preserves_virtual_and_ignored_output_layout_state() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), {
                let mut state = planner_output_state("eDP-1", 1920, 1080);
                state.identity.description = Some("Built-in display".to_string());
                state.position = Position::new(0, 0);
                state
            }),
            ("VIRT-1".to_string(), {
                let mut state = planner_output_state("VIRT-1", 1920, 1080);
                state.identity.is_virtual = true;
                state.position = Position::new(7000, 500);
                state.mirror_target = Some("eDP-1".to_string());
                state
            }),
            ("IGNORED-1".to_string(), {
                let mut state = planner_output_state("IGNORED-1", 800, 600);
                state.identity.is_ignored = true;
                state.position = Position::new(-300, 250);
                state.mirror_target = Some("eDP-1".to_string());
                state
            }),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Builtin, &topology, None, None)?;
    assert_eq!(plan.outputs["VIRT-1"].position, Position::new(7000, 500));
    assert_eq!(plan.outputs["IGNORED-1"].position, Position::new(-300, 250));
    assert_eq!(
        plan.outputs["VIRT-1"].mirror_target.as_deref(),
        Some("eDP-1")
    );
    assert_eq!(
        plan.outputs["IGNORED-1"].mirror_target.as_deref(),
        Some("eDP-1")
    );
    Ok(())
}

#[test]
fn horizontal_includes_disabled_connected_outputs() -> anyhow::Result<()> {
    let mut b = planner_output_state("B", 1280, 720);
    b.enabled = false;
    let topology = Topology {
        outputs: HashMap::from([
            ("A".to_string(), planner_output_state("A", 1920, 1080)),
            ("B".to_string(), b),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Horizontal, &topology, None, None)?;
    assert!(plan.outputs["A"].enabled);
    assert!(plan.outputs["B"].enabled);
    assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["B"].position, Position::new(1920, 0));
    Ok(())
}

#[test]
fn vertical_centers_outputs_horizontally() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("A".to_string(), planner_output_state("A", 3440, 1440)),
            ("B".to_string(), planner_output_state("B", 2560, 1440)),
            ("C".to_string(), planner_output_state("C", 1920, 1080)),
        ]),
    };

    let plan = Planner::plan_from_preset(VirtualPreset::Vertical, &topology, None, None)?;
    assert_eq!(plan.outputs["A"].position, Position::new(0, 0));
    assert_eq!(plan.outputs["B"].position, Position::new(440, 1440));
    assert_eq!(plan.outputs["C"].position, Position::new(760, 2880));
    Ok(())
}

#[test]
fn plan_from_profile_maps_layout_using_stable_identity() -> anyhow::Result<()> {
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
        hooks: Hooks::default(),
    };

    let matched_outputs = vec![MatchedOutputBinding {
        topology_name: "DP-1".to_string(),
        layout_name: "DP-4".to_string(),
    }];
    let plan = Planner::plan_from_profile(&profile, &matched_outputs, &topology)?;

    assert!(!plan.outputs["DP-1"].enabled);
    assert_eq!(plan.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(
        plan.outputs["DP-1"].identity.connector.as_deref(),
        Some("DP-1")
    );
    Ok(())
}

#[test]
fn detect_preset_returns_virtual_preset() {
    let mut a = planner_output_state("A", 100, 50);
    a.position = Position::new(0, 0);
    let mut b = planner_output_state("B", 200, 50);
    b.position = Position::new(100, 0);
    let topology = Topology {
        outputs: HashMap::from([("A".to_string(), a), ("B".to_string(), b)]),
    };

    assert_eq!(detect_preset(&topology), Some(VirtualPreset::Horizontal));
}
