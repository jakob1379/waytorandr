use std::collections::HashMap;
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

use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::{
    ApplyResult, Backend, Capabilities, Mode, OutputIdentity, OutputState, OutputWatcher, Position,
    TestResult, Topology, Transform,
};

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

impl Default for HeadInfo {
    fn default() -> Self {
        panic!("HeadInfo::default should not be used")
    }
}

impl WlrootsBackend {
    pub fn connect() -> Result<Self> {
        let connection =
            Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let (globals, event_queue) = registry_queue_init::<State>(&connection)
            .context("failed to initialize Wayland registry")?;
        let qh = event_queue.handle();

        let manager = bind_manager(&globals, &qh)?;
        let mut client = WaylandClient {
            event_queue,
            state: State {
                manager: Some(manager),
                ..State::default()
            },
        };
        client.sync()?;
        client.sync()?;

        Ok(Self {
            inner: Mutex::new(client),
        })
    }
}

impl Backend for WlrootsBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            can_enumerate: true,
            can_watch: true,
            can_test: true,
            can_apply: true,
            supports_transforms: true,
            supports_scale: true,
            supports_mirror: false,
            supports_brightness: false,
            supports_gamma: false,
            backend_name: "wlroots".to_string(),
        }
    }

    fn enumerate_outputs(&self) -> Result<Topology> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("backend lock poisoned"))?;
        inner.sync()?;
        Ok(inner.export_topology())
    }

    fn watch_outputs(&self) -> Result<Box<dyn OutputWatcher>> {
        let initial = self.enumerate_outputs()?.fingerprint();
        Ok(Box::new(WlrootsWatcher {
            backend: WlrootsBackend::connect()?,
            last_fingerprint: Some(initial),
        }))
    }

    fn current_state(&self) -> Result<Topology> {
        self.enumerate_outputs()
    }

    fn test(&self, plan: &LayoutPlan) -> Result<TestResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("backend lock poisoned"))?;
        let status = inner.submit_with_retry(plan, true, 3)?;
        Ok(TestResult {
            success: matches!(status, ConfigStatus::Succeeded),
            message: Some(match status {
                ConfigStatus::Succeeded => {
                    format!("wlroots validated {} output changes", plan.outputs.len())
                }
                ConfigStatus::Failed => "wlroots compositor rejected the configuration".to_string(),
                ConfigStatus::Cancelled => {
                    "wlroots compositor cancelled the configuration because topology changed"
                        .to_string()
                }
            }),
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> Result<ApplyResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("backend lock poisoned"))?;
        let status = inner.submit_with_retry(plan, false, 3)?;
        inner.sync()?;
        let applied_state = inner.export_topology();
        Ok(ApplyResult {
            success: matches!(status, ConfigStatus::Succeeded),
            message: Some(match status {
                ConfigStatus::Succeeded => "applied successfully".to_string(),
                ConfigStatus::Failed => "compositor rejected the configuration".to_string(),
                ConfigStatus::Cancelled => {
                    "configuration cancelled because topology changed".to_string()
                }
            }),
            applied_state: Some(applied_state),
        })
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

            outputs.insert(
                name.clone(),
                OutputState {
                    identity: OutputIdentity {
                        edid_hash: None,
                        make: head.make.clone(),
                        model: head.model.clone(),
                        serial: head.serial.clone(),
                        connector: Some(name),
                        description: head.description.clone(),
                        is_virtual: head
                            .description
                            .as_deref()
                            .map(is_virtual_description)
                            .unwrap_or(false),
                        is_ignored: false,
                    },
                    enabled: head_is_enabled(head.enabled, head.current_mode.as_ref()),
                    mode,
                    position: head.position,
                    scale: head.scale,
                    transform: head.transform,
                    mirror_target: None,
                    backend_data: None,
                },
            );
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
                    "wlroots configuration cancelled, retrying with refreshed serial"
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

fn head_is_enabled(enabled: bool, current_mode: Option<&ObjectId>) -> bool {
    enabled || current_mode.is_some()
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
                mode.width as i32,
                mode.height as i32,
                (mode.refresh * 1000) as i32,
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
        WEnum::Value(wl_output::Transform::Normal) => Transform::Normal,
        WEnum::Value(wl_output::Transform::_90) => Transform::Rot90,
        WEnum::Value(wl_output::Transform::_180) => Transform::Rot180,
        WEnum::Value(wl_output::Transform::_270) => Transform::Rot270,
        WEnum::Value(wl_output::Transform::Flipped) => Transform::Flipped,
        WEnum::Value(wl_output::Transform::Flipped90) => Transform::Flipped90,
        WEnum::Value(wl_output::Transform::Flipped180) => Transform::Flipped180,
        WEnum::Value(wl_output::Transform::Flipped270) => Transform::Flipped270,
        WEnum::Value(_) => Transform::Normal,
        WEnum::Unknown(_) => Transform::Normal,
    }
}

fn is_virtual_description(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    lower.contains("virtual") || lower.contains("headless") || lower.contains("x11")
}

struct WlrootsWatcher {
    backend: WlrootsBackend,
    last_fingerprint: Option<String>,
}

impl OutputWatcher for WlrootsWatcher {
    fn poll_changed(&mut self) -> Result<Option<Topology>> {
        std::thread::sleep(Duration::from_millis(500));
        let topology = self.backend.enumerate_outputs()?;
        let fingerprint = topology.fingerprint();
        if self.last_fingerprint.as_ref() == Some(&fingerprint) {
            return Ok(None);
        }
        self.last_fingerprint = Some(fingerprint);
        Ok(Some(topology))
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
        _: &(),
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
        _: &(),
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
                entry.description = Some(description)
            }
            zwlr_output_head_v1::Event::Make { make } => entry.make = Some(make),
            zwlr_output_head_v1::Event::Model { model } => entry.model = Some(model),
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                entry.serial = Some(serial_number)
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => entry.enabled = enabled != 0,
            zwlr_output_head_v1::Event::Position { x, y } => entry.position = Position { x, y },
            zwlr_output_head_v1::Event::Scale { scale } => entry.scale = scale,
            zwlr_output_head_v1::Event::Transform { transform } => {
                entry.transform = transform_from_wl(transform)
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
                entry.current_mode = Some(mode.id())
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
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.modes.get_mut(&mode.id()) else {
            return;
        };

        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                entry.width = Some(width as u32);
                entry.height = Some(height as u32);
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => entry.refresh = Some(refresh as u32),
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
        _: &(),
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
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

pub fn probe_backend() -> Option<Box<dyn Backend>> {
    WlrootsBackend::connect()
        .ok()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_mode_marks_head_enabled() {
        assert!(head_is_enabled(false, Some(&ObjectId::null())));
        assert!(!head_is_enabled(false, None));
    }
}
