use std::collections::HashMap;

use serde::Deserialize;
use waytorandr_core::{Mode, OutputState, Position, Topology, Transform};

const ROTATION_NONE: i32 = 1;
const ROTATION_LEFT: i32 = 2;
const ROTATION_INVERTED: i32 = 4;
const ROTATION_RIGHT: i32 = 8;
const ROTATION_FLIPPED: i32 = 16;
const ROTATION_FLIPPED_90: i32 = 32;
const ROTATION_FLIPPED_180: i32 = 64;
const ROTATION_FLIPPED_270: i32 = 128;

#[derive(Debug, Deserialize)]
pub(super) struct KScreenConfig {
    #[serde(default)]
    pub(super) outputs: Vec<KScreenOutput>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KScreenOutput {
    pub(super) name: String,
    #[serde(default)]
    pub(super) connected: bool,
    #[serde(default)]
    pub(super) enabled: bool,
    #[serde(default)]
    pub(super) id: u32,
    #[serde(rename = "currentModeId")]
    pub(super) current_mode_id: Option<String>,
    #[serde(default)]
    pub(super) modes: Vec<KScreenMode>,
    #[serde(default)]
    pub(super) pos: Position,
    #[serde(default = "default_scale")]
    pub(super) scale: f64,
    #[serde(rename = "replicationSource", default)]
    pub(super) replication_source: u32,
    #[serde(default = "default_rotation")]
    pub(super) rotation: i32,
    #[serde(default)]
    pub(super) size: KScreenSize,
}

#[derive(Debug, Deserialize)]
pub(super) struct KScreenMode {
    pub(super) id: String,
    #[serde(rename = "refreshRate")]
    pub(super) refresh_rate: f64,
    pub(super) size: KScreenSize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub(super) struct KScreenSize {
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn export_kscreen_topology(config: &KScreenConfig) -> Topology {
    let mut outputs = HashMap::new();
    let output_names_by_id: HashMap<u32, &str> = config
        .outputs
        .iter()
        .map(|output| (output.id, output.name.as_str()))
        .collect();

    for output in config.outputs.iter().filter(|output| output.connected) {
        let mut state = OutputState::new(output.name.clone());
        state.identity.connector = Some(output.name.clone());
        state.identity.is_virtual = is_virtual_output(&output.name);
        state.enabled = output.enabled;
        state.mode = current_mode_for_output(output);
        state.available_modes = available_modes(output);
        state.position = output.pos;
        state.scale = output.scale;
        state.transform = transform_from_kscreen(output.rotation);
        state.mirror_target = mirror_target_name(output, &output_names_by_id);
        state.backend_data = None;
        outputs.insert(output.name.clone(), state);
    }

    Topology { outputs }
}

pub(super) fn current_mode_for_output(output: &KScreenOutput) -> Option<Mode> {
    output
        .current_mode_id
        .as_deref()
        .and_then(|id| output.modes.iter().find(|mode| mode.id == id))
        .or_else(|| {
            output
                .modes
                .iter()
                .find(|mode| mode.size == output.size && output.enabled)
        })
        .map(|mode| Mode {
            width: mode.size.width,
            height: mode.size.height,
            refresh: round_refresh(mode.refresh_rate),
        })
}

pub(super) fn mirror_target_name(
    output: &KScreenOutput,
    output_names_by_id: &HashMap<u32, &str>,
) -> Option<String> {
    if output.replication_source == 0 {
        return None;
    }

    output_names_by_id
        .get(&output.replication_source)
        .map(|name| (*name).to_string())
}

pub(super) fn round_refresh(refresh_rate: f64) -> u32 {
    let refresh = refresh_rate.round();
    if refresh < 0.0 {
        0
    } else if refresh > f64::from(u32::MAX) {
        u32::MAX
    } else {
        refresh.to_string().parse().unwrap_or_default()
    }
}

fn available_modes(output: &KScreenOutput) -> Vec<Mode> {
    let mut modes: Vec<Mode> = output
        .modes
        .iter()
        .map(|mode| Mode {
            width: mode.size.width,
            height: mode.size.height,
            refresh: round_refresh(mode.refresh_rate),
        })
        .collect();
    modes.sort_by_key(|mode| (mode.width * mode.height, mode.refresh));
    modes.dedup();
    modes
}

fn is_virtual_output(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("virtual") || lower.contains("headless")
}

pub(super) fn transform_from_kscreen(rotation: i32) -> Transform {
    match rotation {
        ROTATION_LEFT => Transform::Rot90,
        ROTATION_INVERTED => Transform::Rot180,
        ROTATION_RIGHT => Transform::Rot270,
        ROTATION_FLIPPED => Transform::Flipped,
        ROTATION_FLIPPED_90 => Transform::Flipped90,
        ROTATION_FLIPPED_180 => Transform::Flipped180,
        ROTATION_FLIPPED_270 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

fn default_scale() -> f64 {
    1.0
}

fn default_rotation() -> i32 {
    ROTATION_NONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_target_name_resolves_replication_source() {
        let output = KScreenOutput {
            name: "HDMI-A-1".to_string(),
            connected: true,
            enabled: true,
            id: 2,
            current_mode_id: None,
            modes: Vec::new(),
            pos: Position::default(),
            scale: 1.0,
            replication_source: 1,
            rotation: ROTATION_NONE,
            size: KScreenSize::default(),
        };
        let names = HashMap::from([(1, "DP-1"), (2, "HDMI-A-1")]);

        assert_eq!(
            mirror_target_name(&output, &names),
            Some("DP-1".to_string())
        );
    }

    #[test]
    fn transform_from_kscreen_maps_known_rotations() {
        assert_eq!(transform_from_kscreen(ROTATION_NONE), Transform::Normal);
        assert_eq!(transform_from_kscreen(ROTATION_LEFT), Transform::Rot90);
        assert_eq!(
            transform_from_kscreen(ROTATION_FLIPPED_270),
            Transform::Flipped270
        );
    }
}
