use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

use waytorandr_core::{Mode, OutputState, Position, Topology, Transform};

pub(super) type PropertyMap = HashMap<String, OwnedValue>;
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
pub(super) type CurrentStateReply = (
    u32,
    Vec<MonitorTuple>,
    Vec<LogicalMonitorTuple>,
    PropertyMap,
);

const FLOAT_EPSILON: f64 = 0.000_1;

#[derive(Debug)]
pub(super) struct CurrentState {
    pub(super) serial: u32,
    pub(super) monitors: Vec<MonitorConfig>,
    pub(super) logical_monitors: Vec<LogicalMonitorConfig>,
    pub(super) properties: PropertyMap,
}

impl CurrentState {
    pub(super) fn from_reply(reply: CurrentStateReply) -> Self {
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

    pub(super) fn monitor(&self, connector: &str) -> Option<&MonitorConfig> {
        self.monitors
            .iter()
            .find(|monitor| monitor.connector == connector)
    }

    pub(super) fn logical_by_connector(&self) -> HashMap<&str, LogicalMonitorSnapshot> {
        let mut by_connector = HashMap::new();
        for logical in &self.logical_monitors {
            let mirror_root = logical.connectors.first().cloned();
            for connector in &logical.connectors {
                by_connector.insert(
                    connector.as_str(),
                    LogicalMonitorSnapshot {
                        position: logical.position,
                        scale: logical.scale,
                        transform: logical.transform,
                        primary: logical.primary,
                        mirror_target: if logical.connectors.len() > 1
                            && mirror_root.as_deref() != Some(connector.as_str())
                        {
                            mirror_root.clone()
                        } else {
                            None
                        },
                    },
                );
            }
        }
        by_connector
    }
}

#[derive(Debug)]
pub(super) struct MonitorConfig {
    pub(super) connector: String,
    vendor: String,
    product: String,
    serial: String,
    pub(super) modes: Vec<MonitorMode>,
    pub(super) properties: PropertyMap,
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
pub(super) struct MonitorMode {
    pub(super) id: String,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) refresh: f64,
    pub(super) preferred_scale: f64,
    pub(super) supported_scales: Vec<f64>,
    properties: PropertyMap,
}

impl MonitorMode {
    fn from_tuple(tuple: (String, i32, i32, f64, f64, Vec<f64>, PropertyMap)) -> Self {
        let (id, width, height, refresh, preferred_scale, supported_scales, properties) = tuple;
        Self {
            id,
            width: u32::try_from(width.max(0)).unwrap_or_default(),
            height: u32::try_from(height.max(0)).unwrap_or_default(),
            refresh,
            preferred_scale,
            supported_scales,
            properties,
        }
    }
}

#[derive(Debug)]
pub(super) struct LogicalMonitorConfig {
    pub(super) position: Position,
    pub(super) scale: f64,
    pub(super) transform: u32,
    pub(super) primary: bool,
    pub(super) connectors: Vec<String>,
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

#[derive(Clone, Debug)]
pub(super) struct LogicalMonitorSnapshot {
    pub(super) position: Position,
    pub(super) scale: f64,
    pub(super) transform: u32,
    pub(super) primary: bool,
    pub(super) mirror_target: Option<String>,
}

pub(super) fn export_current_state_topology(state: &CurrentState) -> Topology {
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
        output.available_modes = available_modes_for_monitor(monitor);
        output.position = logical.map_or_else(Position::default, |value| value.position);
        output.scale = logical.map_or(1.0, |value| value.scale);
        output.transform = logical.map_or_else(Transform::default, |value| {
            transform_from_gnome(value.transform)
        });
        output.mirror_target = logical.and_then(|value| value.mirror_target.clone());
        output.backend_data = None;
        outputs.insert(monitor.connector.clone(), output);
    }

    Topology { outputs }
}

pub(super) fn current_monitor_mode(monitor: &MonitorConfig) -> Option<&MonitorMode> {
    monitor
        .modes
        .iter()
        .find(|mode| property_as_bool(&mode.properties, "is-current") == Some(true))
        .or_else(|| {
            monitor
                .modes
                .iter()
                .find(|mode| property_as_bool(&mode.properties, "is-preferred") == Some(true))
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

fn available_modes_for_monitor(monitor: &MonitorConfig) -> Vec<Mode> {
    let mut modes: Vec<Mode> = monitor
        .modes
        .iter()
        .map(|mode| Mode {
            width: mode.width,
            height: mode.height,
            refresh: round_refresh(mode.refresh),
        })
        .collect();
    modes.sort_by_key(|mode| (mode.width * mode.height, mode.refresh));
    modes.dedup();
    modes
}

pub(super) fn property_as_bool(properties: &PropertyMap, key: &str) -> Option<bool> {
    properties
        .get(key)
        .and_then(|value| bool::try_from(value).ok())
}

pub(super) fn property_as_u32(properties: &PropertyMap, key: &str) -> Option<u32> {
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

pub(super) fn round_refresh(refresh: f64) -> u32 {
    let refresh = refresh.round();
    if refresh < 0.0 {
        0
    } else if refresh > f64::from(u32::MAX) {
        u32::MAX
    } else {
        refresh as u32
    }
}

pub(super) fn float_eq(left: f64, right: f64) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_refresh_clamps_negative_values_to_zero() {
        assert_eq!(round_refresh(-59.9), 0);
    }

    #[test]
    fn round_refresh_casts_bounded_values() {
        assert_eq!(round_refresh(59.6), 60);
        assert_eq!(round_refresh(f64::from(u32::MAX) + 1.0), u32::MAX);
    }

    #[test]
    fn virtual_output_detection_uses_connector_or_description() {
        assert!(is_virtual_output("HEADLESS-1", None));
        assert!(is_virtual_output("DP-1", Some("Virtual monitor")));
        assert!(!is_virtual_output("DP-1", Some("Dell display")));
    }
}
