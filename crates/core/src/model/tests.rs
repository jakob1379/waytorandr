use super::*;

#[test]
fn primary_key_ignores_unknown_identity_fields() {
    let mut identity = OutputIdentity::new("DP-4");
    identity.make = Some("Unknown".to_string());
    identity.model = Some("Unknown".to_string());
    identity.description = Some("Unknown - Unknown - DP-4".to_string());

    assert_eq!(identity.primary_key(), "conn:DP-4");
}

#[test]
fn same_layout_as_ignores_mode_inventory_and_backend_metadata() {
    let mut left = OutputState::new("DP-1");
    left.enabled = true;
    left.mode = Some(Mode::new(1920, 1080, 60));
    left.available_modes = vec![Mode::new(1920, 1080, 60)];
    left.backend_data = Some(serde_json::json!({"side": "left"}));

    let mut right = left.clone();
    right.available_modes = vec![Mode::new(1280, 720, 60), Mode::new(1920, 1080, 60)];
    right.backend_data = Some(serde_json::json!({"side": "right"}));

    assert!(left.same_layout_as(&right));
}

#[test]
fn virtual_preset_policy_is_centralized() {
    let capabilities = Capabilities {
        can_validate: true,
        supports_mirror: false,
        supports_largest_mirror: false,
        backend: BackendKind::Wlroots,
    };

    assert!(capabilities
        .virtual_preset_unavailable_message(VirtualPreset::Mirror)
        .is_some());
    assert!(capabilities
        .virtual_preset_unavailable_message(VirtualPreset::Largest)
        .is_none());
    assert!(capabilities
        .virtual_preset_unavailable_message(VirtualPreset::Common)
        .is_none());
    assert!(capabilities
        .virtual_preset_unavailable_message(VirtualPreset::Horizontal)
        .is_none());
}
