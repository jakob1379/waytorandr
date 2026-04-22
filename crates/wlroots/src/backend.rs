use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{
    event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1;
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_v1::{
    self, ZwlrOutputConfigurationV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_head_v1::{
    self, ZwlrOutputHeadV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_manager_v1::{
    self, ZwlrOutputManagerV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_mode_v1::{
    self, ZwlrOutputModeV1,
};

use waytorandr_core::engine::{
    ApplyResult, Backend, ConfigFailureKind, OutputWatcher, PollingOutputWatcher, TestResult,
};
use waytorandr_core::error::{BackendConnectionError, CoreError, CoreResult};
use waytorandr_core::model::{
    normalized_identity_value, BackendKind, Capabilities, Mode, OutputState, Position, Topology,
    Transform,
};
use waytorandr_core::planner::LayoutPlan;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub struct WlrootsBackend {
    inner: Mutex<WaylandClient>,
}

struct WaylandClient {
    event_queue: EventQueue<State>,
    state: State,
}

#[derive(Default)]
struct State {
    manager: Option<ZwlrOutputManagerV1>,
    serial: Option<u32>,
    heads: HashMap<ObjectId, HeadInfo>,
    modes: HashMap<ObjectId, ModeInfo>,
    config_status: Option<ConfigStatus>,
}

#[derive(Clone)]
struct HeadInfo {
    head: ZwlrOutputHeadV1,
    name: Option<String>,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    enabled: bool,
    position: Position,
    transform: Transform,
    scale: f64,
    current_mode: Option<ObjectId>,
    modes: Vec<ObjectId>,
}

#[derive(Clone)]
struct ModeInfo {
    mode: ZwlrOutputModeV1,
    width: Option<u32>,
    height: Option<u32>,
    refresh: Option<u32>,
    preferred: bool,
    head_id: ObjectId,
}

#[derive(Clone, Copy)]
enum ConfigStatus {
    Succeeded,
    Failed,
    Cancelled,
}

impl WlrootsBackend {
    /// Connects to the wlroots backend.
    ///
    /// # Errors
    /// Returns an error if the Wayland display or output-management protocol is unavailable.
    pub fn connect() -> CoreResult<Self> {
        let connection = Connection::connect_to_env()
            .context("failed to connect to Wayland display")
            .map_err(|source| {
                CoreError::BackendConnection(BackendConnectionError::Initialize {
                    backend: BackendKind::Wlroots,
                    source,
                })
            })?;
        let (globals, event_queue) = registry_queue_init::<State>(&connection)
            .context("failed to initialize Wayland registry")
            .map_err(|source| {
                CoreError::BackendConnection(BackendConnectionError::Initialize {
                    backend: BackendKind::Wlroots,
                    source,
                })
            })?;
        let qh = event_queue.handle();

        let manager = bind_manager(&globals, &qh).map_err(|source| {
            CoreError::BackendConnection(BackendConnectionError::Initialize {
                backend: BackendKind::Wlroots,
                source,
            })
        })?;
        let mut client = WaylandClient {
            event_queue,
            state: State {
                manager: Some(manager),
                ..State::default()
            },
        };
        client.sync().map_err(|source| {
            CoreError::BackendConnection(BackendConnectionError::Initialize {
                backend: BackendKind::Wlroots,
                source,
            })
        })?;
        client.sync().map_err(|source| {
            CoreError::BackendConnection(BackendConnectionError::Initialize {
                backend: BackendKind::Wlroots,
                source,
            })
        })?;

        Ok(Self {
            inner: Mutex::new(client),
        })
    }

    fn enumerate_live_topology(&self) -> CoreResult<Topology> {
        let mut inner = self.inner.lock().map_err(|_| CoreError::Backend {
            source: anyhow!("backend lock poisoned"),
        })?;
        inner
            .sync()
            .map_err(|source| CoreError::Backend { source })?;
        Ok(inner.export_topology())
    }

    fn snapshot_topology() -> CoreResult<Topology> {
        Self::connect()?.enumerate_live_topology()
    }
}

impl Backend for WlrootsBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Wlroots);
        capabilities.can_test = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        // Use a fresh output-manager snapshot for reads so stale head objects do
        // not leak into later polls after disconnect/reconfigure churn.
        Self::snapshot_topology()
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        let initial = self.enumerate_outputs()?;
        Ok(Box::new(PollingOutputWatcher::new(
            WlrootsBackend::connect()?,
            POLL_INTERVAL,
            Some(initial.setup_fingerprint()),
        )))
    }

    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        let mut inner = self.inner.lock().map_err(|_| CoreError::Backend {
            source: anyhow!("backend lock poisoned"),
        })?;
        let status = inner
            .submit_with_retry(plan, true, 3)
            .map_err(|source| CoreError::Backend { source })?;
        let message = Some(match status {
            ConfigStatus::Succeeded => {
                format!("wlroots validated {} output changes", plan.outputs.len())
            }
            ConfigStatus::Failed => "wlroots compositor rejected the configuration".to_string(),
            ConfigStatus::Cancelled => {
                "wlroots compositor cancelled the configuration because topology changed"
                    .to_string()
            }
        });
        Ok(match status {
            ConfigStatus::Succeeded => TestResult::supported(message),
            ConfigStatus::Failed | ConfigStatus::Cancelled => {
                TestResult::rejected(config_failure(status), message)
            }
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let status = {
            let mut inner = self.inner.lock().map_err(|_| CoreError::Backend {
                source: anyhow!("backend lock poisoned"),
            })?;
            inner
                .submit_with_retry(plan, false, 3)
                .map_err(|source| CoreError::Backend { source })?
        };
        let applied_state = Self::snapshot_topology()?;
        let mut result = ApplyResult::default();
        result.success = matches!(status, ConfigStatus::Succeeded);
        result.failure = config_failure(status);
        result.message = Some(match status {
            ConfigStatus::Succeeded => "applied successfully".to_string(),
            ConfigStatus::Failed => "compositor rejected the configuration".to_string(),
            ConfigStatus::Cancelled => {
                "configuration cancelled because topology changed".to_string()
            }
        });
        result.applied_state = Some(applied_state);
        Ok(result)
    }
}

impl WaylandClient {
    fn sync(&mut self) -> Result<()> {
        self.event_queue
            .roundtrip(&mut self.state)
            .context("failed to roundtrip Wayland event queue")?;
        Ok(())
    }

    fn export_topology(&self) -> Topology {
        let mut outputs = HashMap::new();
        for head in self.state.heads.values() {
            let Some(name) = head.name.clone() else {
                continue;
            };

            let mode = preferred_mode_for_head(&self.state, head);

            outputs.insert(name.clone(), {
                let mut state = OutputState::new(name);
                state.identity.edid_hash = None;
                state.identity.make.clone_from(&head.make);
                state.identity.model.clone_from(&head.model);
                state.identity.serial.clone_from(&head.serial);
                state.identity.description.clone_from(&head.description);
                state.identity.is_virtual = head
                    .description
                    .as_deref()
                    .is_some_and(is_virtual_description);
                state.identity.is_ignored = false;
                state.enabled = head_is_enabled(head.enabled);
                state.mode = mode;
                state.available_modes = available_modes_for_head(&self.state, head);
                state.position = head.position;
                state.scale = head.scale;
                state.transform = head.transform;
                state.mirror_target = None;
                state.backend_data = None;
                state
            });
        }
        Topology { outputs }
    }

    fn submit(&mut self, plan: &LayoutPlan, test_only: bool) -> Result<ConfigStatus> {
        let serial = self.state.serial.ok_or_else(|| {
            anyhow!("wlroots compositor did not provide an output-management serial")
        })?;
        let manager = self
            .state
            .manager
            .as_ref()
            .ok_or_else(|| anyhow!("wlroots output manager is unavailable"))?
            .clone();
        let qh = self.event_queue.handle();

        self.state.config_status = None;
        let configuration = manager.create_configuration(serial, &qh, ());

        for head in self.state.heads.values() {
            let Some(name) = head.name.as_deref() else {
                continue;
            };

            if let Some(desired) = plan.outputs.get(name) {
                if !desired.enabled {
                    configuration.disable_head(&head.head);
                    continue;
                }

                let conf_head = configuration.enable_head(&head.head, &qh, ());
                apply_head_config(&self.state, desired, head, &conf_head)?;
            } else {
                configuration.disable_head(&head.head);
            }
        }

        if test_only {
            configuration.test();
        } else {
            configuration.apply();
        }

        for _ in 0..5 {
            self.sync()?;
            if let Some(status) = self.state.config_status.take() {
                return Ok(status);
            }
        }

        bail!("wlroots compositor did not answer configuration request")
    }

    fn submit_with_retry(
        &mut self,
        plan: &LayoutPlan,
        test_only: bool,
        attempts: usize,
    ) -> Result<ConfigStatus> {
        let attempts = attempts.max(1);
        for attempt in 0..attempts {
            self.sync()?;
            let status = self.submit(plan, test_only)?;
            if !matches!(status, ConfigStatus::Cancelled) {
                return Ok(status);
            }

            if attempt + 1 < attempts {
                tracing::warn!(
                    attempt = attempt + 1,
                    total_attempts = attempts,
                    "wlroots configuration cancelled, retrying after refreshing compositor state"
                );
            } else {
                return Ok(status);
            }
        }

        unreachable!("submit_with_retry always returns from inside the retry loop")
    }
}

fn bind_manager(globals: &GlobalList, qh: &QueueHandle<State>) -> Result<ZwlrOutputManagerV1> {
    globals
        .bind::<ZwlrOutputManagerV1, _, _>(qh, 1..=2, ())
        .map_err(|_| {
            anyhow!("wlroots output-management protocol is not available on this compositor")
        })
}

fn mode_from_info(info: &ModeInfo) -> Option<Mode> {
    Some(Mode {
        width: info.width?,
        height: info.height?,
        refresh: info.refresh.unwrap_or(0) / 1000,
    })
}

fn preferred_mode_for_head(state: &State, head: &HeadInfo) -> Option<Mode> {
    head.current_mode
        .as_ref()
        .and_then(|id| state.modes.get(id))
        .and_then(mode_from_info)
        .or_else(|| {
            head.modes
                .iter()
                .filter_map(|id| state.modes.get(id))
                .find(|mode| mode.preferred)
                .and_then(mode_from_info)
        })
        .or_else(|| {
            head.modes
                .iter()
                .filter_map(|id| state.modes.get(id))
                .find_map(mode_from_info)
        })
}

fn available_modes_for_head(state: &State, head: &HeadInfo) -> Vec<Mode> {
    let mut modes: Vec<Mode> = head
        .modes
        .iter()
        .filter_map(|id| state.modes.get(id))
        .filter_map(mode_from_info)
        .collect();
    modes.sort_by_key(|mode| (mode.width * mode.height, mode.refresh));
    modes.dedup();
    modes
}

fn head_is_enabled(enabled: bool) -> bool {
    enabled
}

fn config_failure(status: ConfigStatus) -> Option<ConfigFailureKind> {
    match status {
        ConfigStatus::Succeeded => None,
        ConfigStatus::Failed => Some(ConfigFailureKind::Rejected),
        ConfigStatus::Cancelled => Some(ConfigFailureKind::TopologyChanged),
    }
}

fn apply_head_config(
    state: &State,
    desired: &OutputState,
    head: &HeadInfo,
    conf_head: &ZwlrOutputConfigurationHeadV1,
) -> Result<()> {
    if let Some(mode) = desired.mode {
        if let Some(existing_mode) =
            head.modes
                .iter()
                .filter_map(|id| state.modes.get(id))
                .find(|candidate| {
                    candidate.width == Some(mode.width)
                        && candidate.height == Some(mode.height)
                        && candidate.refresh.unwrap_or(0) / 1000 == mode.refresh
                })
        {
            conf_head.set_mode(&existing_mode.mode);
        } else {
            conf_head.set_custom_mode(
                i32::try_from(mode.width).context("wlroots mode width does not fit in i32")?,
                i32::try_from(mode.height).context("wlroots mode height does not fit in i32")?,
                i32::try_from(mode.refresh.saturating_mul(1000))
                    .context("wlroots mode refresh does not fit in i32")?,
            );
        }
    }

    conf_head.set_position(desired.position.x, desired.position.y);
    conf_head.set_scale(desired.scale);
    conf_head.set_transform(transform_to_wl(desired.transform));

    if desired.mirror_target.is_some() {
        bail!("wlroots mirroring is not implemented in this backend")
    }

    if head.name.is_none() {
        bail!("attempted to configure unnamed output")
    }

    Ok(())
}

fn transform_to_wl(transform: Transform) -> wl_output::Transform {
    match transform {
        Transform::Normal => wl_output::Transform::Normal,
        Transform::Rot90 => wl_output::Transform::_90,
        Transform::Rot180 => wl_output::Transform::_180,
        Transform::Rot270 => wl_output::Transform::_270,
        Transform::Flipped => wl_output::Transform::Flipped,
        Transform::Flipped90 => wl_output::Transform::Flipped90,
        Transform::Flipped180 => wl_output::Transform::Flipped180,
        Transform::Flipped270 => wl_output::Transform::Flipped270,
    }
}

fn transform_from_wl(transform: WEnum<wl_output::Transform>) -> Transform {
    match transform {
        WEnum::Value(wl_output::Transform::_90) => Transform::Rot90,
        WEnum::Value(wl_output::Transform::_180) => Transform::Rot180,
        WEnum::Value(wl_output::Transform::_270) => Transform::Rot270,
        WEnum::Value(wl_output::Transform::Flipped) => Transform::Flipped,
        WEnum::Value(wl_output::Transform::Flipped90) => Transform::Flipped90,
        WEnum::Value(wl_output::Transform::Flipped180) => Transform::Flipped180,
        WEnum::Value(wl_output::Transform::Flipped270) => Transform::Flipped270,
        WEnum::Value(_) | WEnum::Unknown(_) => Transform::Normal,
    }
}

fn is_virtual_description(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    lower.contains("virtual") || lower.contains("headless") || lower.contains("x11")
}

fn update_identity_field(field: &mut Option<String>, value: &str) {
    if let Some(value) = normalized_identity_value(Some(value)) {
        *field = Some(value);
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _manager: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Done { serial } => state.serial = Some(serial),
            zwlr_output_manager_v1::Event::Finished => state.manager = None,
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputHeadV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let entry = state.heads.entry(head.id()).or_insert_with(|| HeadInfo {
            head: head.clone(),
            name: None,
            description: None,
            make: None,
            model: None,
            serial: None,
            enabled: false,
            position: Position::default(),
            transform: Transform::Normal,
            scale: 1.0,
            current_mode: None,
            modes: Vec::new(),
        });

        match event {
            zwlr_output_head_v1::Event::Name { name } => entry.name = Some(name),
            zwlr_output_head_v1::Event::Description { description } => {
                update_identity_field(&mut entry.description, &description);
            }
            zwlr_output_head_v1::Event::Make { make } => {
                update_identity_field(&mut entry.make, &make);
            }
            zwlr_output_head_v1::Event::Model { model } => {
                update_identity_field(&mut entry.model, &model);
            }
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                update_identity_field(&mut entry.serial, &serial_number);
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => entry.enabled = enabled != 0,
            zwlr_output_head_v1::Event::Position { x, y } => entry.position = Position { x, y },
            zwlr_output_head_v1::Event::Scale { scale } => entry.scale = scale,
            zwlr_output_head_v1::Event::Transform { transform } => {
                entry.transform = transform_from_wl(transform);
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                let mode_id = mode.id();
                if !entry.modes.contains(&mode_id) {
                    entry.modes.push(mode_id.clone());
                }
                state.modes.entry(mode_id).or_insert_with(|| ModeInfo {
                    mode,
                    width: None,
                    height: None,
                    refresh: None,
                    preferred: false,
                    head_id: head.id(),
                });
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                entry.current_mode = Some(mode.id());
            }
            zwlr_output_head_v1::Event::Finished => {
                state.heads.remove(&head.id());
                state.modes.retain(|_, mode| mode.head_id != head.id());
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputModeV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
        zwlr_output_head_v1::EVT_CURRENT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.modes.get_mut(&mode.id()) else {
            return;
        };

        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                entry.width = u32::try_from(width).ok();
                entry.height = u32::try_from(height).ok();
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                entry.refresh = u32::try_from(refresh).ok();
            }
            zwlr_output_mode_v1::Event::Preferred => entry.preferred = true,
            zwlr_output_mode_v1::Event::Finished => {
                let head_id = entry.head_id.clone();
                state.modes.remove(&mode.id());
                if let Some(head) = state.heads.get_mut(&head_id) {
                    head.modes.retain(|id| id != &mode.id());
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        config: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.config_status = Some(match event {
            zwlr_output_configuration_v1::Event::Succeeded => ConfigStatus::Succeeded,
            zwlr_output_configuration_v1::Event::Failed => ConfigStatus::Failed,
            zwlr_output_configuration_v1::Event::Cancelled => ConfigStatus::Cancelled,
            _ => return,
        });
        config.destroy();
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _state: &mut Self,
        _head: &ZwlrOutputConfigurationHeadV1,
        _event: wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_head_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::{head_is_enabled, update_identity_field};

    #[test]
    fn disabled_head_stays_disabled_even_if_mode_lingers() {
        assert!(!head_is_enabled(false));
        assert!(head_is_enabled(true));
    }

    #[test]
    fn update_identity_field_keeps_existing_value_for_unknown_placeholder() {
        let mut field = Some("Microstep".to_string());
        update_identity_field(&mut field, "Unknown");
        assert_eq!(field.as_deref(), Some("Microstep"));
    }
}
