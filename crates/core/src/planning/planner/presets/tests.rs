use super::*;

fn preset_output_state(connector: &str, enabled: bool, mode: Mode) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = enabled;
    state.mode = Some(mode);
    state.available_modes = vec![mode];
    state
}

fn preset_topology(outputs: impl IntoIterator<Item = (&'static str, OutputState)>) -> Topology {
    Topology {
        outputs: outputs
            .into_iter()
            .map(|(name, state)| (name.to_string(), state))
            .collect(),
    }
}

#[test]
fn external_preset_preserves_virtual_output_state() -> anyhow::Result<()> {
    let mut virtual_output = preset_output_state("HEADLESS-1", true, Mode::new(1280, 720, 60));
    virtual_output.identity.is_virtual = true;
    virtual_output.position = Position::new(40, 50);

    let plan = plan_from_preset(
        VirtualPreset::External,
        &preset_topology([
            (
                "eDP-1",
                preset_output_state("eDP-1", true, Mode::new(1920, 1080, 60)),
            ),
            (
                "DP-1",
                preset_output_state("DP-1", true, Mode::new(2560, 1440, 60)),
            ),
            ("HEADLESS-1", virtual_output.clone()),
        ]),
        Some(&OutputIdentity::new("eDP-1")),
        None,
    )?;

    assert!(!plan.outputs["eDP-1"].enabled);
    assert!(plan.outputs["DP-1"].enabled);
    assert_eq!(plan.outputs["HEADLESS-1"], virtual_output);
    Ok(())
}

#[test]
fn mirror_preset_chooses_primary_hint_as_root() -> anyhow::Result<()> {
    let plan = plan_from_preset(
        VirtualPreset::Mirror,
        &preset_topology([
            (
                "DP-1",
                preset_output_state("DP-1", true, Mode::new(1920, 1080, 60)),
            ),
            (
                "HDMI-A-1",
                preset_output_state("HDMI-A-1", true, Mode::new(1920, 1080, 60)),
            ),
        ]),
        None,
        Some("HDMI-A-1"),
    )?;

    assert_eq!(plan.outputs["HDMI-A-1"].mirror_target, None);
    assert_eq!(
        plan.outputs["DP-1"].mirror_target.as_deref(),
        Some("HDMI-A-1")
    );
    Ok(())
}

#[test]
fn common_preset_rejects_outputs_without_shared_mode() -> anyhow::Result<()> {
    let Err(err) = plan_from_preset(
        VirtualPreset::Common,
        &preset_topology([
            (
                "DP-1",
                preset_output_state("DP-1", true, Mode::new(2560, 1440, 60)),
            ),
            (
                "HDMI-A-1",
                preset_output_state("HDMI-A-1", true, Mode::new(1280, 720, 60)),
            ),
        ]),
        None,
        None,
    ) else {
        anyhow::bail!("common preset should require a shared mode");
    };

    assert_eq!(
        err.to_string(),
        "Invalid configuration: No common mode found"
    );
    Ok(())
}
