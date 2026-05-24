use std::collections::HashMap;
use std::convert::TryFrom;

use wayland_client::backend::ObjectId;
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};
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

use waytorandr_core::{Position, Topology, Transform};

mod projection;

use projection::{export_protocol_topology, update_identity_field};

#[derive(Default)]
pub(super) struct State {
    pub(super) manager: Option<ZwlrOutputManagerV1>,
    pub(super) serial: Option<u32>,
    pub(super) heads: HashMap<ObjectId, HeadInfo>,
    pub(super) modes: HashMap<ObjectId, ModeInfo>,
    pub(super) config_status: Option<ConfigStatus>,
}

#[derive(Clone)]
pub(super) struct HeadInfo {
    pub(super) head: ZwlrOutputHeadV1,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) make: Option<String>,
    pub(super) model: Option<String>,
    pub(super) serial: Option<String>,
    pub(super) enabled: bool,
    pub(super) position: Position,
    pub(super) transform: Transform,
    pub(super) scale: f64,
    pub(super) current_mode: Option<ObjectId>,
    pub(super) modes: Vec<ObjectId>,
}

#[derive(Clone)]
pub(super) struct ModeInfo {
    pub(super) mode: ZwlrOutputModeV1,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
    pub(super) refresh: Option<u32>,
    pub(super) preferred: bool,
    pub(super) head_id: ObjectId,
}

#[derive(Clone, Copy)]
pub(super) enum ConfigStatus {
    Succeeded,
    Failed,
    Cancelled,
}

impl ConfigStatus {
    pub(super) const fn as_label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl State {
    pub(super) fn to_topology(&self) -> Topology {
        export_protocol_topology(self)
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
    use wayland_client::protocol::wl_output;
    use wayland_client::WEnum;
    use waytorandr_core::Transform;

    use super::transform_from_wl;

    #[test]
    fn transform_from_wl_maps_known_rotations() {
        assert_eq!(
            transform_from_wl(WEnum::Value(wl_output::Transform::_90)),
            Transform::Rot90
        );
        assert_eq!(
            transform_from_wl(WEnum::Value(wl_output::Transform::Flipped270)),
            Transform::Flipped270
        );
    }

    #[test]
    fn transform_from_wl_defaults_unknown_values_to_normal() {
        assert_eq!(transform_from_wl(WEnum::Unknown(999)), Transform::Normal);
    }
}
