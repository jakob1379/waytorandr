use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

use waytorandr_core::engine::{ApplyResult, Backend, ConfigFailureKind, OutputWatcher, TestResult};
use waytorandr_core::error::{CoreError, CoreResult};
use waytorandr_core::model::{Capabilities, Mode, OutputState, Position, Topology, Transform};
use waytorandr_core::planner::LayoutPlan;

const DISPLAY_CONFIG_DESTINATION: &str = "org.gnome.Mutter.DisplayConfig";
const DISPLAY_CONFIG_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const DISPLAY_CONFIG_INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const METHOD_VERIFY: u32 = 0;
const METHOD_TEMPORARY: u32 = 1;
const FLOAT_EPSILON: f64 = 0.000_1;

type PropertyMap = HashMap<String, OwnedValue>;
type MonitorTuple = (
    (String, String, String, String),
    Vec<(String, i32, i32, f64, f64, Vec<f64>, PropertyMap)>,
    PropertyMap,
);
type LogicalMonitorTuple = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<(String, String, String, String)>,
    PropertyMap,
);
type CurrentStateReply = (
    u32,
    Vec<MonitorTuple>,
    Vec<LogicalMonitorTuple>,
    PropertyMap,
);
type ApplyMonitorTuple = (String, String, PropertyMap);
type ApplyLogicalMonitorTuple = (i32, i32, f64, u32, bool, Vec<ApplyMonitorTuple>);

#[derive(Clone)]
pub struct GnomeBackend;

impl GnomeBackend {
    pub fn connect() -> Result<Self> {
        let backend = Self;
        backend
            .load_state()
            .context("failed to query Mutter DisplayConfig state")?;
        Ok(backend)
    }

    fn load_state(&self) -> Result<CurrentState> {
        let connection = Connection::session().context("failed to connect to the session bus")?;
        let proxy = Proxy::new(
            &connection,
            DISPLAY_CONFIG_DESTINATION,
            DISPLAY_CONFIG_PATH,
            DISPLAY_CONFIG_INTERFACE,
        )
        .context("failed to create Mutter DisplayConfig proxy")?;
        let reply: CurrentStateReply = proxy
            .call("GetCurrentState", &())
            .context("failed to call GetCurrentState on org.gnome.Mutter.DisplayConfig")?;
        Ok(CurrentState::from_reply(reply))
    }

    fn export_topology(&self, state: &CurrentState) -> Topology {
        let logical_by_connector = state.logical_by_connector();
        let mut outputs = HashMap::new();

        for monitor in &state.monitors {
            let logical = logical_by_connector.get(monitor.connector.as_str());
            let mut output = OutputState::new(monitor.connector.clone());
            output.identity.make = non_empty(&monitor.vendor);
            output.identity.model = non_empty(&monitor.product);
            output.identity.serial = non_empty(&monitor.serial);
            output.identity.description = display_name(&monitor.properties);
            output.identity.is_virtual =
                is_virtual_output(&monitor.connector, output.identity.description.as_deref());
            output.enabled = logical.is_some();
            output.mode = current_mode_for_monitor(monitor);
            output.position = logical.map(|value| value.position).unwrap_or_default();
            output.scale = logical.map(|value| value.scale).unwrap_or(1.0);
            output.transform = logical
                .map(|value| transform_from_gnome(value.transform))
                .unwrap_or_default();
            output.backend_data = None;
            outputs.insert(monitor.connector.clone(), output);
        }

        Topology { outputs }
    }

    fn build_apply_config(
        &self,
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
        for (name, desired) in enabled_outputs {
            if desired.mirror_target.is_some() {
                bail!("GNOME mirroring is not implemented in this backend");
            }

            let monitor = state
                .monitor(name)
                .ok_or_else(|| anyhow!("output `{name}` is not connected on this GNOME session"))?;
            let mode = resolve_mode(monitor, desired.mode)
                .with_context(|| format!("failed to resolve mode for output `{name}`"))?;

            logical_monitors.push((
                desired.position.x,
                desired.position.y,
                select_scale(mode, desired.scale),
                transform_to_gnome(desired.transform),
                primary_connector.as_deref() == Some(name.as_str()),
                vec![(
                    name.clone(),
                    mode.id.clone(),
                    monitor_apply_properties(monitor),
                )],
            ));
        }

        Ok((
            state.serial,
            logical_monitors,
            layout_properties(&state.properties),
        ))
    }

    fn submit(&self, plan: &LayoutPlan, method: u32) -> Result<()> {
        let state = self.load_state()?;
        let (serial, logical_monitors, properties) = self.build_apply_config(&state, plan)?;
        let connection = Connection::session().context("failed to connect to the session bus")?;
        let proxy = Proxy::new(
            &connection,
            DISPLAY_CONFIG_DESTINATION,
            DISPLAY_CONFIG_PATH,
            DISPLAY_CONFIG_INTERFACE,
        )
        .context("failed to create Mutter DisplayConfig proxy")?;
        proxy
            .call(
                "ApplyMonitorsConfig",
                &(serial, method, logical_monitors, properties),
            )
            .context("failed to call ApplyMonitorsConfig on org.gnome.Mutter.DisplayConfig")
    }
}

impl Backend for GnomeBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::named("gnome");
        capabilities.can_enumerate = true;
        capabilities.can_watch = true;
        capabilities.can_test = true;
        capabilities.can_apply = true;
        capabilities.supports_transforms = true;
        capabilities.supports_scale = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        let state = self
            .load_state()
            .map_err(|source| CoreError::Backend { source })?;
        Ok(self.export_topology(&state))
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        let initial = self.enumerate_outputs()?.fingerprint();
        Ok(Box::new(GnomeWatcher {
            backend: self.clone(),
            last_fingerprint: Some(initial),
        }))
    }

    fn current_state(&self) -> CoreResult<Topology> {
        self.enumerate_outputs()
    }

    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        match self.submit(plan, METHOD_VERIFY) {
            Ok(()) => {
                let mut result = TestResult::default();
                result.success = true;
                result.message = Some(format!(
                    "GNOME validated {} output changes",
                    plan.outputs.len()
                ));
                Ok(result)
            }
            Err(source) => {
                let mut result = TestResult::default();
                result.success = false;
                result.failure = classify_apply_failure(&source);
                result.message = Some(format!("GNOME rejected the configuration: {source:#}"));
                Ok(result)
            }
        }
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        match self.submit(plan, METHOD_TEMPORARY) {
            Ok(()) => {
                let applied_state = self.enumerate_outputs()?;
                let mut result = ApplyResult::default();
                result.success = true;
                result.message = Some("GNOME applied the configuration".to_string());
                result.applied_state = Some(applied_state);
                Ok(result)
            }
            Err(source) => {
                let mut result = ApplyResult::default();
                result.success = false;
                result.failure = classify_apply_failure(&source);
                result.message = Some(format!(
                    "GNOME failed to apply the configuration: {source:#}"
                ));
                Ok(result)
            }
        }
    }
}

struct GnomeWatcher {
    backend: GnomeBackend,
    last_fingerprint: Option<String>,
}

impl OutputWatcher for GnomeWatcher {
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

#[derive(Debug)]
struct CurrentState {
    serial: u32,
    monitors: Vec<MonitorConfig>,
    logical_monitors: Vec<LogicalMonitorConfig>,
    properties: PropertyMap,
}

impl CurrentState {
    fn from_reply(reply: CurrentStateReply) -> Self {
        let (serial, monitors, logical_monitors, properties) = reply;
        Self {
            serial,
            monitors: monitors
                .into_iter()
                .map(MonitorConfig::from_tuple)
                .collect(),
            logical_monitors: logical_monitors
                .into_iter()
                .map(LogicalMonitorConfig::from_tuple)
                .collect(),
            properties,
        }
    }

    fn monitor(&self, connector: &str) -> Option<&MonitorConfig> {
        self.monitors
            .iter()
            .find(|monitor| monitor.connector == connector)
    }

    fn logical_by_connector(&self) -> HashMap<&str, LogicalMonitorSnapshot> {
        let mut by_connector = HashMap::new();
        for logical in &self.logical_monitors {
            for connector in &logical.connectors {
                by_connector.insert(
                    connector.as_str(),
                    LogicalMonitorSnapshot {
                        position: logical.position,
                        scale: logical.scale,
                        transform: logical.transform,
                        primary: logical.primary,
                    },
                );
            }
        }
        by_connector
    }
}

#[derive(Debug)]
struct MonitorConfig {
    connector: String,
    vendor: String,
    product: String,
    serial: String,
    modes: Vec<MonitorMode>,
    properties: PropertyMap,
}

impl MonitorConfig {
    fn from_tuple(tuple: MonitorTuple) -> Self {
        let ((connector, vendor, product, serial), modes, properties) = tuple;
        Self {
            connector,
            vendor,
            product,
            serial,
            modes: modes.into_iter().map(MonitorMode::from_tuple).collect(),
            properties,
        }
    }
}

#[derive(Debug)]
struct MonitorMode {
    id: String,
    width: u32,
    height: u32,
    refresh: f64,
    preferred_scale: f64,
    supported_scales: Vec<f64>,
    properties: PropertyMap,
}

impl MonitorMode {
    fn from_tuple(tuple: (String, i32, i32, f64, f64, Vec<f64>, PropertyMap)) -> Self {
        let (id, width, height, refresh, preferred_scale, supported_scales, properties) = tuple;
        Self {
            id,
            width: width.max(0) as u32,
            height: height.max(0) as u32,
            refresh,
            preferred_scale,
            supported_scales,
            properties,
        }
    }
}

#[derive(Debug)]
struct LogicalMonitorConfig {
    position: Position,
    scale: f64,
    transform: u32,
    primary: bool,
    connectors: Vec<String>,
}

impl LogicalMonitorConfig {
    fn from_tuple(tuple: LogicalMonitorTuple) -> Self {
        let (x, y, scale, transform, primary, monitors, _) = tuple;
        Self {
            position: Position::new(x, y),
            scale,
            transform,
            primary,
            connectors: monitors
                .into_iter()
                .map(|(connector, _, _, _)| connector)
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LogicalMonitorSnapshot {
    position: Position,
    scale: f64,
    transform: u32,
    primary: bool,
}

fn current_monitor_mode(monitor: &MonitorConfig) -> Option<&MonitorMode> {
    monitor
        .modes
        .iter()
        .find(|mode| property_as_bool(&mode.properties, "is-current").unwrap_or(false))
        .or_else(|| {
            monitor
                .modes
                .iter()
                .find(|mode| property_as_bool(&mode.properties, "is-preferred").unwrap_or(false))
        })
        .or_else(|| monitor.modes.first())
}

fn current_mode_for_monitor(monitor: &MonitorConfig) -> Option<Mode> {
    current_monitor_mode(monitor).map(|mode| Mode {
        width: mode.width,
        height: mode.height,
        refresh: round_refresh(mode.refresh),
    })
}

fn resolve_mode<'a>(monitor: &'a MonitorConfig, desired: Option<Mode>) -> Result<&'a MonitorMode> {
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

fn layout_properties(properties: &PropertyMap) -> PropertyMap {
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

fn property_as_bool(properties: &PropertyMap, key: &str) -> Option<bool> {
    properties
        .get(key)
        .and_then(|value| bool::try_from(value).ok())
}

fn property_as_u32(properties: &PropertyMap, key: &str) -> Option<u32> {
    properties
        .get(key)
        .and_then(|value| u32::try_from(value).ok())
}

fn property_as_str(properties: &PropertyMap, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
}

fn display_name(properties: &PropertyMap) -> Option<String> {
    property_as_str(properties, "display-name").and_then(|value| non_empty(&value))
}

fn select_scale(mode: &MonitorMode, desired: f64) -> f64 {
    if mode.supported_scales.is_empty() {
        return desired;
    }
    mode.supported_scales
        .iter()
        .copied()
        .find(|scale| float_eq(*scale, desired))
        .unwrap_or(desired)
}

fn refresh_distance(refresh: f64, desired_refresh: u32) -> f64 {
    (refresh - desired_refresh as f64).abs()
}

fn round_refresh(refresh: f64) -> u32 {
    refresh.round().max(0.0) as u32
}

fn float_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < FLOAT_EPSILON
}

fn transform_from_gnome(value: u32) -> Transform {
    match value {
        1 => Transform::Rot90,
        2 => Transform::Rot180,
        3 => Transform::Rot270,
        4 => Transform::Flipped,
        5 => Transform::Flipped90,
        6 => Transform::Flipped180,
        7 => Transform::Flipped270,
        _ => Transform::Normal,
    }
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

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn is_virtual_output(connector: &str, description: Option<&str>) -> bool {
    let connector = connector.to_ascii_lowercase();
    let description = description.unwrap_or_default().to_ascii_lowercase();
    connector.contains("virtual")
        || connector.contains("headless")
        || description.contains("virtual")
        || description.contains("headless")
}

fn classify_apply_failure(error: &anyhow::Error) -> Option<ConfigFailureKind> {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("stale")
        || message.contains("serial")
        || message.contains("changed")
        || message.contains("out of date")
    {
        Some(ConfigFailureKind::TopologyChanged)
    } else {
        Some(ConfigFailureKind::Rejected)
    }
}

pub fn probe_backend() -> Option<Box<dyn Backend>> {
    GnomeBackend::connect()
        .ok()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let mut properties = PropertyMap::new();
        properties.insert("layout-mode".to_string(), OwnedValue::from(2u32));
        properties.insert(
            "supports-changing-layout-mode".to_string(),
            OwnedValue::from(true),
        );

        CurrentState::from_reply((
            7,
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
                    ],
                    external_props,
                ),
            ],
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
            ],
            properties,
        ))
    }

    #[test]
    fn export_topology_marks_enabled_outputs_from_logical_monitors() {
        let backend = GnomeBackend;
        let topology = backend.export_topology(&sample_state());

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
    }

    #[test]
    fn build_apply_config_preserves_current_primary_and_layout_mode() {
        let backend = GnomeBackend;
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

        let (serial, logical_monitors, properties) =
            backend.build_apply_config(&sample_state(), &plan).unwrap();

        assert_eq!(serial, 7);
        assert_eq!(logical_monitors.len(), 2);
        assert!(logical_monitors[0].4);
        assert_eq!(logical_monitors[1].5[0].1, "2560x1440@60");
        assert_eq!(u32::try_from(&properties["layout-mode"]).ok(), Some(2));
    }

    #[test]
    fn build_apply_config_rejects_mirroring_requests() {
        let backend = GnomeBackend;
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output.mirror_target = Some("eDP-1".to_string());
            output
        })]));

        let error = backend
            .build_apply_config(&sample_state(), &plan)
            .unwrap_err();

        assert!(error.to_string().contains("mirroring"));
    }

    #[test]
    fn layout_properties_omits_layout_mode_when_mutter_does_not_allow_it() {
        let mut properties = PropertyMap::new();
        properties.insert("layout-mode".to_string(), OwnedValue::from(2u32));

        assert!(layout_properties(&properties).is_empty());
    }
}
