use std::collections::HashMap;
use std::ffi::OsString;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use waytorandr_core::engine::{ApplyResult, Backend, ConfigFailureKind, OutputWatcher, TestResult};
use waytorandr_core::error::{CoreError, CoreResult};
use waytorandr_core::model::{Capabilities, Mode, OutputState, Position, Topology, Transform};
use waytorandr_core::planner::LayoutPlan;

const KSCREEN_DOCTOR: &str = "kscreen-doctor";
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const ROTATION_NONE: i32 = 1;
const ROTATION_LEFT: i32 = 2;
const ROTATION_INVERTED: i32 = 4;
const ROTATION_RIGHT: i32 = 8;
const ROTATION_FLIPPED: i32 = 16;
const ROTATION_FLIPPED_90: i32 = 32;
const ROTATION_FLIPPED_180: i32 = 64;
const ROTATION_FLIPPED_270: i32 = 128;

#[derive(Clone)]
pub struct KScreenBackend {
    command: OsString,
}

impl KScreenBackend {
    pub fn connect() -> Result<Self> {
        let backend = Self {
            command: std::env::var_os("WAYTORANDR_KSCREEN_DOCTOR")
                .unwrap_or_else(|| OsString::from(KSCREEN_DOCTOR)),
        };
        backend
            .load_config()
            .context("failed to query KScreen display configuration")?;
        Ok(backend)
    }

    fn load_config(&self) -> Result<KScreenConfig> {
        let output = self.run_command(&[String::from("--json")])?;
        if !output.status.success() {
            bail!(
                "`{KSCREEN_DOCTOR} --json` failed: {}",
                describe_command_output(&output)
            );
        }

        serde_json::from_slice(&output.stdout)
            .context("failed to parse `kscreen-doctor --json` output")
    }

    fn run_command(&self, args: &[String]) -> Result<Output> {
        let mut command = Command::new(&self.command);
        command.args(args);
        command.output().with_context(|| {
            format!(
                "failed to run `{}` with args {:?}",
                self.command.to_string_lossy(),
                args
            )
        })
    }

    fn export_topology(&self, config: &KScreenConfig) -> Topology {
        let mut outputs = HashMap::new();

        for output in config.outputs.iter().filter(|output| output.connected) {
            let mut state = OutputState::new(output.name.clone());
            state.identity.connector = Some(output.name.clone());
            state.identity.is_virtual = is_virtual_output(&output.name);
            state.enabled = output.enabled;
            state.mode = current_mode(output);
            state.available_modes = available_modes(output);
            state.position = output.pos;
            state.scale = output.scale;
            state.transform = transform_from_kscreen(output.rotation);
            state.mirror_target = None;
            state.backend_data = None;
            outputs.insert(output.name.clone(), state);
        }

        Topology { outputs }
    }

    fn apply_plan(&self, plan: &LayoutPlan) -> Result<ApplyResult> {
        let config = self.load_config()?;
        let args = build_apply_args(plan, &config)?;
        if args.is_empty() {
            let mut result = ApplyResult::default();
            result.success = true;
            result.message = Some("configuration already matches current state".to_string());
            result.applied_state = Some(self.export_topology(&config));
            return Ok(result);
        }

        let output = self.run_command(&args)?;
        if !output.status.success() {
            let mut result = ApplyResult::default();
            result.success = false;
            result.failure = Some(ConfigFailureKind::Rejected);
            result.message = Some(format!(
                "`{KSCREEN_DOCTOR}` rejected the configuration: {}",
                describe_command_output(&output)
            ));
            return Ok(result);
        }

        let applied = self.load_config()?;
        let mut result = ApplyResult::default();
        result.success = true;
        result.message = Some(format!("KScreen applied {} display changes", args.len()));
        result.applied_state = Some(self.export_topology(&applied));
        Ok(result)
    }
}

impl Backend for KScreenBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::named("kscreen");
        capabilities.can_enumerate = true;
        capabilities.can_watch = true;
        capabilities.can_test = false;
        capabilities.can_apply = true;
        capabilities.supports_transforms = true;
        capabilities.supports_scale = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        let config = self
            .load_config()
            .map_err(|source| CoreError::Backend { source })?;
        Ok(self.export_topology(&config))
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        let initial = self.enumerate_outputs()?.fingerprint();
        Ok(Box::new(KScreenWatcher {
            backend: self.clone(),
            last_fingerprint: Some(initial),
        }))
    }

    fn current_state(&self) -> CoreResult<Topology> {
        self.enumerate_outputs()
    }

    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        let mut result = TestResult::default();
        result.success = true;
        result.message = Some(format!(
            "KScreen does not provide a dry-run API; {} output changes were planned",
            plan.outputs.len()
        ));
        Ok(result)
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        self.apply_plan(plan)
            .map_err(|source| CoreError::Backend { source })
    }
}

struct KScreenWatcher {
    backend: KScreenBackend,
    last_fingerprint: Option<String>,
}

impl OutputWatcher for KScreenWatcher {
    fn poll_changed(&mut self) -> CoreResult<Option<Topology>> {
        thread::sleep(POLL_INTERVAL);
        let topology = self.backend.enumerate_outputs()?;
        let fingerprint = topology.fingerprint();
        if self.last_fingerprint.as_ref() == Some(&fingerprint) {
            return Ok(None);
        }
        self.last_fingerprint = Some(fingerprint);
        Ok(Some(topology))
    }
}

fn build_apply_args(plan: &LayoutPlan, config: &KScreenConfig) -> Result<Vec<String>> {
    let outputs_by_name: HashMap<&str, &KScreenOutput> = config
        .outputs
        .iter()
        .filter(|output| output.connected)
        .map(|output| (output.name.as_str(), output))
        .collect();

    let mut disable_args = Vec::new();
    let mut enable_args = Vec::new();

    let mut current_outputs: Vec<&KScreenOutput> = outputs_by_name.values().copied().collect();
    current_outputs.sort_by(|left, right| left.name.cmp(&right.name));

    for output in current_outputs {
        let disable = match plan.outputs.get(&output.name) {
            Some(desired) => !desired.enabled,
            None => true,
        };

        if disable && output.enabled {
            disable_args.push(format!("output.{}.disable", output.name));
        }
    }

    let mut desired_outputs: Vec<(&String, &OutputState)> = plan
        .outputs
        .iter()
        .filter(|(_, desired)| desired.enabled)
        .collect();
    desired_outputs.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (name, desired) in desired_outputs {
        if desired.mirror_target.is_some() {
            bail!("KScreen mirroring is not implemented in this backend");
        }

        let current = outputs_by_name
            .get(name.as_str())
            .copied()
            .ok_or_else(|| anyhow!("output `{name}` is not connected on this KScreen session"))?;

        if !current.enabled {
            enable_args.push(format!("output.{name}.enable"));
        }

        if current_mode(current) != desired.mode {
            let mode_arg = mode_command_for_output(current, desired.mode)
                .with_context(|| format!("failed to resolve mode for output `{name}`"))?;
            if let Some(mode_arg) = mode_arg {
                enable_args.push(format!("output.{name}.mode.{mode_arg}"));
            }
        }

        if current.pos != desired.position {
            enable_args.push(format!(
                "output.{name}.position.{},{}",
                desired.position.x, desired.position.y
            ));
        }

        if !float_eq(current.scale, desired.scale) {
            enable_args.push(format!(
                "output.{name}.scale.{}",
                format_scale(desired.scale)
            ));
        }

        let current_transform = transform_from_kscreen(current.rotation);
        if current_transform != desired.transform {
            enable_args.push(format!(
                "output.{name}.rotation.{}",
                rotation_query(desired.transform)
            ));
        }
    }

    disable_args.extend(enable_args);
    Ok(disable_args)
}

fn mode_command_for_output(
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
    let left_matches = rounded_refresh(left.refresh_rate) == desired_refresh;
    let right_matches = rounded_refresh(right.refresh_rate) == desired_refresh;

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
    (refresh_rate - desired_refresh as f64).abs()
}

fn current_mode(output: &KScreenOutput) -> Option<Mode> {
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
            refresh: rounded_refresh(mode.refresh_rate),
        })
}

fn available_modes(output: &KScreenOutput) -> Vec<Mode> {
    let mut modes: Vec<Mode> = output
        .modes
        .iter()
        .map(|mode| Mode {
            width: mode.size.width,
            height: mode.size.height,
            refresh: rounded_refresh(mode.refresh_rate),
        })
        .collect();
    modes.sort_by_key(|mode| (mode.width * mode.height, mode.refresh));
    modes.dedup();
    modes
}

fn rounded_refresh(refresh_rate: f64) -> u32 {
    refresh_rate.round().max(0.0) as u32
}

fn float_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.000_1
}

fn format_scale(scale: f64) -> String {
    let mut formatted = scale.to_string();
    if formatted.contains('e') || formatted.contains('E') {
        formatted = format!("{scale:.4}");
    }
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn is_virtual_output(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("virtual") || lower.contains("headless")
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

fn transform_from_kscreen(rotation: i32) -> Transform {
    match rotation {
        ROTATION_LEFT => Transform::Rot90,
        ROTATION_INVERTED => Transform::Rot180,
        ROTATION_RIGHT => Transform::Rot270,
        ROTATION_FLIPPED => Transform::Flipped,
        ROTATION_FLIPPED_90 => Transform::Flipped90,
        ROTATION_FLIPPED_180 => Transform::Flipped180,
        ROTATION_FLIPPED_270 => Transform::Flipped270,
        ROTATION_NONE => Transform::Normal,
        _ => Transform::Normal,
    }
}

fn describe_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("exit status {}", output.status),
        (false, true) => format!("stdout: {stdout}"),
        (true, false) => format!("stderr: {stderr}"),
        (false, false) => format!("stdout: {stdout}; stderr: {stderr}"),
    }
}

#[derive(Debug, Deserialize)]
struct KScreenConfig {
    #[serde(default)]
    outputs: Vec<KScreenOutput>,
}

#[derive(Debug, Deserialize)]
struct KScreenOutput {
    name: String,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    enabled: bool,
    #[serde(rename = "currentModeId")]
    current_mode_id: Option<String>,
    #[serde(default)]
    modes: Vec<KScreenMode>,
    #[serde(default)]
    pos: Position,
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default = "default_rotation")]
    rotation: i32,
    #[serde(default)]
    size: KScreenSize,
}

#[derive(Debug, Deserialize)]
struct KScreenMode {
    id: String,
    #[serde(rename = "refreshRate")]
    refresh_rate: f64,
    size: KScreenSize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
struct KScreenSize {
    width: u32,
    height: u32,
}

fn default_scale() -> f64 {
    1.0
}

fn default_rotation() -> i32 {
    ROTATION_NONE
}

pub fn probe_backend() -> Option<Box<dyn Backend>> {
    KScreenBackend::connect()
        .ok()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use waytorandr_core::model::OutputIdentity;

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
                    }
                ],
                "name": "DVI-I-1",
                "pos": { "x": 1920, "y": 0 },
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

    fn sample_config() -> KScreenConfig {
        serde_json::from_str(SAMPLE_CONFIG).unwrap()
    }

    fn output(connector: &str, enabled: bool) -> OutputState {
        let mut state = OutputState::new(connector);
        state.identity = OutputIdentity::new(connector);
        state.enabled = enabled;
        state
    }

    #[test]
    fn export_topology_uses_connected_outputs_only() {
        let backend = KScreenBackend {
            command: OsString::from(KSCREEN_DOCTOR),
        };

        let topology = backend.export_topology(&sample_config());

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
        assert_eq!(topology.outputs["DVI-I-1"].scale, 1.25);
        assert!(!topology.outputs.contains_key("HDMI-A-2"));
    }

    #[test]
    fn build_apply_args_disables_missing_outputs_and_updates_enabled_ones() {
        let config = sample_config();
        let mut laptop = output("eDP-1", true);
        laptop.mode = Some(Mode {
            width: 1920,
            height: 1080,
            refresh: 50,
        });
        laptop.position = Position { x: 0, y: 0 };

        let mut external = output("DVI-I-1", true);
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

        let args = build_apply_args(&plan, &config).unwrap();

        assert_eq!(
            args,
            vec![
                "output.DVI-I-1.enable".to_string(),
                "output.DVI-I-1.scale.1".to_string(),
                "output.DVI-I-1.rotation.normal".to_string(),
                "output.eDP-1.mode.2".to_string(),
            ]
        );
    }

    #[test]
    fn build_apply_args_can_disable_outputs_not_in_plan() {
        let config = sample_config();
        let mut laptop = output("eDP-1", true);
        laptop.mode = Some(Mode {
            width: 1920,
            height: 1080,
            refresh: 60,
        });

        let plan = LayoutPlan::new(HashMap::from([("eDP-1".to_string(), laptop)]));

        let args = build_apply_args(&plan, &config).unwrap();

        assert!(args.is_empty());
    }

    #[test]
    fn mode_command_falls_back_to_nearest_refresh_for_matching_resolution() {
        let config = sample_config();
        let output = config
            .outputs
            .iter()
            .find(|output| output.name == "DVI-I-1")
            .unwrap();

        let mode = mode_command_for_output(
            output,
            Some(Mode {
                width: 2560,
                height: 1440,
                refresh: 59,
            }),
        )
        .unwrap();

        assert_eq!(mode.as_deref(), Some("4"));
    }

    #[test]
    fn format_scale_trims_redundant_zeroes() {
        assert_eq!(format_scale(1.0), "1");
        assert_eq!(format_scale(1.25), "1.25");
        assert_eq!(format_scale(1.5), "1.5");
    }
}
