use waytorandr_core::{
    normalized_identity_value, Mode, OutputState, Position, Topology, Transform,
};

use super::{HeadInfo, ModeInfo, State};
use wayland_client::backend::ObjectId;

pub(super) fn export_protocol_topology(state: &State) -> Topology {
    let mut outputs = std::collections::HashMap::new();
    for head in state.heads.values() {
        let projection = projected_head_from_state(state, head);
        if let Some((name, state)) = output_state_from_projection(&projection) {
            outputs.insert(name, state);
        }
    }
    Topology { outputs }
}

struct ProjectedHead<I> {
    name: Option<String>,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    enabled: bool,
    position: Position,
    transform: Transform,
    scale: f64,
    current_mode: Option<I>,
    modes: Vec<ProjectedMode<I>>,
}

#[derive(Clone)]
struct ProjectedMode<I> {
    id: I,
    width: Option<u32>,
    height: Option<u32>,
    refresh: Option<u32>,
    preferred: bool,
}

fn projected_head_from_state(state: &State, head: &HeadInfo) -> ProjectedHead<ObjectId> {
    ProjectedHead {
        name: head.name.clone(),
        description: head.description.clone(),
        make: head.make.clone(),
        model: head.model.clone(),
        serial: head.serial.clone(),
        enabled: head.enabled,
        position: head.position,
        transform: head.transform,
        scale: head.scale,
        current_mode: head.current_mode.clone(),
        modes: projected_modes_for_head(state, head),
    }
}

fn projected_modes_for_head(state: &State, head: &HeadInfo) -> Vec<ProjectedMode<ObjectId>> {
    head.modes
        .iter()
        .filter_map(|id| {
            state
                .modes
                .get(id)
                .map(|mode| projected_mode(id.clone(), mode))
        })
        .collect()
}

fn projected_mode(id: ObjectId, info: &ModeInfo) -> ProjectedMode<ObjectId> {
    ProjectedMode {
        id,
        width: info.width,
        height: info.height,
        refresh: info.refresh,
        preferred: info.preferred,
    }
}

fn output_state_from_projection<I: Eq>(head: &ProjectedHead<I>) -> Option<(String, OutputState)> {
    let name = head.name.clone()?;
    let mut state = OutputState::new(name.clone());
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
    state.mode = current_mode_from_projection(head.current_mode.as_ref(), &head.modes);
    state.available_modes = available_modes_from_projection(&head.modes);
    state.position = head.position;
    state.scale = head.scale;
    state.transform = head.transform;
    state.mirror_target = None;
    state.backend_data = None;
    Some((name, state))
}

fn mode_for_projection<I>(info: &ProjectedMode<I>) -> Option<Mode> {
    Some(Mode {
        width: info.width?,
        height: info.height?,
        refresh: round_refresh(info.refresh),
    })
}

fn current_mode_from_projection<I: Eq>(
    current_mode: Option<&I>,
    modes: &[ProjectedMode<I>],
) -> Option<Mode> {
    current_mode
        .as_ref()
        .and_then(|id| modes.iter().find(|mode| &mode.id == *id))
        .and_then(mode_for_projection)
        .or_else(|| {
            modes
                .iter()
                .find(|mode| mode.preferred)
                .and_then(mode_for_projection)
        })
        .or_else(|| modes.iter().find_map(mode_for_projection))
}

fn available_modes_from_projection<I>(modes: &[ProjectedMode<I>]) -> Vec<Mode> {
    let mut modes: Vec<Mode> = modes.iter().filter_map(mode_for_projection).collect();
    modes.sort_by_key(|mode| (mode.width * mode.height, mode.refresh));
    modes.dedup();
    modes
}

fn round_refresh(refresh_millihz: Option<u32>) -> u32 {
    refresh_millihz.unwrap_or(0) / 1000
}

fn head_is_enabled(enabled: bool) -> bool {
    enabled
}

fn is_virtual_description(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    lower.contains("virtual") || lower.contains("headless") || lower.contains("x11")
}

pub(super) fn update_identity_field(field: &mut Option<String>, value: &str) {
    if let Some(value) = normalized_identity_value(Some(value)) {
        *field = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        head_is_enabled, is_virtual_description, output_state_from_projection,
        update_identity_field, ProjectedHead, ProjectedMode,
    };
    use waytorandr_core::{Mode, Position, Transform};

    fn projected_head(
        current_mode: Option<&'static str>,
        modes: Vec<ProjectedMode<&'static str>>,
    ) -> ProjectedHead<&'static str> {
        ProjectedHead {
            name: Some("DP-1".to_string()),
            description: Some("Headless output".to_string()),
            make: Some("Acme".to_string()),
            model: Some("Panel".to_string()),
            serial: Some("1234".to_string()),
            enabled: false,
            position: Position { x: 10, y: 20 },
            transform: Transform::Rot90,
            scale: 1.5,
            current_mode,
            modes,
        }
    }

    fn projected_mode(
        id: &'static str,
        width: u32,
        height: u32,
        refresh: u32,
        preferred: bool,
    ) -> ProjectedMode<&'static str> {
        ProjectedMode {
            id,
            width: Some(width),
            height: Some(height),
            refresh: Some(refresh),
            preferred,
        }
    }

    #[test]
    fn disabled_head_stays_disabled_even_if_mode_lingers() {
        assert!(!head_is_enabled(false));
        assert!(head_is_enabled(true));
    }

    #[test]
    fn virtual_descriptions_mark_outputs_virtual() {
        assert!(is_virtual_description("Headless output"));
        assert!(is_virtual_description("X11 bridge"));
        assert!(!is_virtual_description("Dell U2720Q"));
    }

    #[test]
    fn update_identity_field_keeps_existing_value_for_unknown_placeholder() {
        let mut field = Some("Microstep".to_string());
        update_identity_field(&mut field, "Unknown");
        assert_eq!(field.as_deref(), Some("Microstep"));
    }

    #[test]
    fn projected_head_skips_unnamed_outputs() {
        let mut head = projected_head(None, Vec::new());
        head.name = None;

        assert!(output_state_from_projection(&head).is_none());
    }

    #[test]
    fn projected_head_exports_identity_state_and_transform() -> anyhow::Result<()> {
        let Some((name, output)) = output_state_from_projection(&projected_head(
            None,
            vec![projected_mode("fallback", 1920, 1080, 60_000, false)],
        )) else {
            anyhow::bail!("named head should project");
        };

        assert_eq!(name, "DP-1");
        assert_eq!(output.identity.connector.as_deref(), Some("DP-1"));
        assert_eq!(output.identity.make.as_deref(), Some("Acme"));
        assert_eq!(output.identity.model.as_deref(), Some("Panel"));
        assert_eq!(output.identity.serial.as_deref(), Some("1234"));
        assert_eq!(
            output.identity.description.as_deref(),
            Some("Headless output")
        );
        assert!(output.identity.is_virtual);
        assert!(!output.enabled);
        assert_eq!(output.position, Position { x: 10, y: 20 });
        assert_eq!(output.scale, 1.5);
        assert_eq!(output.transform, Transform::Rot90);
        assert_eq!(output.mirror_target, None);
        Ok(())
    }

    #[test]
    fn projected_head_prefers_current_then_preferred_then_first_mode() -> anyhow::Result<()> {
        let modes = vec![
            projected_mode("first", 1280, 720, 60_000, false),
            projected_mode("preferred", 1920, 1080, 60_000, true),
            projected_mode("current", 2560, 1440, 144_000, false),
        ];

        let current = output_state_from_projection(&projected_head(Some("current"), modes.clone()))
            .ok_or_else(|| anyhow::anyhow!("current projection"))?
            .1
            .mode;
        let preferred = output_state_from_projection(&projected_head(None, modes.clone()))
            .ok_or_else(|| anyhow::anyhow!("preferred projection"))?
            .1
            .mode;
        let first = output_state_from_projection(&projected_head(
            None,
            vec![
                projected_mode("first", 1280, 720, 60_000, false),
                projected_mode("second", 1920, 1080, 60_000, false),
            ],
        ))
        .ok_or_else(|| anyhow::anyhow!("first projection"))?
        .1
        .mode;

        assert_eq!(
            current,
            Some(Mode {
                width: 2560,
                height: 1440,
                refresh: 144,
            })
        );
        assert_eq!(
            preferred,
            Some(Mode {
                width: 1920,
                height: 1080,
                refresh: 60,
            })
        );
        assert_eq!(
            first,
            Some(Mode {
                width: 1280,
                height: 720,
                refresh: 60,
            })
        );
        Ok(())
    }

    #[test]
    fn projected_head_sorts_and_deduplicates_available_modes() -> anyhow::Result<()> {
        let output = output_state_from_projection(&projected_head(
            None,
            vec![
                projected_mode("large", 2560, 1440, 144_000, false),
                projected_mode("small", 1920, 1080, 60_000, false),
                projected_mode("small-duplicate", 1920, 1080, 60_000, false),
            ],
        ))
        .ok_or_else(|| anyhow::anyhow!("mode projection"))?
        .1;

        assert_eq!(
            output.available_modes,
            vec![
                Mode {
                    width: 1920,
                    height: 1080,
                    refresh: 60,
                },
                Mode {
                    width: 2560,
                    height: 1440,
                    refresh: 144,
                },
            ]
        );
        Ok(())
    }
}
