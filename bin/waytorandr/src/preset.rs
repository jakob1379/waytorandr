use anyhow::{bail, Result};

pub(crate) fn resolve_virtual_preset(
    name: &str,
    reverse: bool,
    largest: bool,
) -> Result<Option<String>> {
    let preset = match name {
        "off" => Some(name.to_string()),
        "common" => Some(if largest {
            "largest".to_string()
        } else {
            "common".to_string()
        }),
        "largest" => Some("largest".to_string()),
        "mirror" => Some("mirror".to_string()),
        "horizontal" | "vertical" => Some(if reverse {
            format!("{}-reverse", name)
        } else {
            name.to_string()
        }),
        _ => None,
    };

    if reverse && preset.is_none() {
        bail!("--reverse can only be used with virtual 'horizontal' or 'vertical' set targets")
    }

    if largest && preset.is_none() {
        bail!("--largest is deprecated; use the virtual 'largest' set target instead")
    }

    if largest && !matches!(name, "common") {
        bail!("--largest is deprecated; use `waytorandr set largest`")
    }

    Ok(preset)
}

pub(crate) fn mirror_unavailable_message(backend_name: &str) -> String {
    format!(
        "native display mirroring is not available through the `{backend_name}` backend; use 'wl-mirror' for now on this compositor. See https://github.com/swaywm/wlr-protocols/issues/101"
    )
}

pub(crate) fn common_unavailable_message(backend_name: &str) -> String {
    format!(
        "the `common` clone layout is not available through the `{backend_name}` backend on Niri because Niri automatically repositions overlapping outputs instead of keeping them at the same origin; use `wl-mirror` for true mirroring or `horizontal`/`vertical` for compositor-managed layouts"
    )
}

pub(crate) fn virtual_completion_candidates(
    current: &str,
) -> Vec<clap_complete::engine::CompletionCandidate> {
    [
        ("off", "virtual"),
        ("common", "virtual"),
        ("largest", "virtual"),
        ("mirror", "virtual"),
        ("horizontal", "virtual"),
        ("vertical", "virtual"),
    ]
    .into_iter()
    .filter(|(name, _)| name.starts_with(current))
    .map(|(name, tag)| clap_complete::engine::CompletionCandidate::new(name).tag(Some(tag.into())))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_virtual_presets_with_flags() {
        assert_eq!(
            resolve_virtual_preset("common", false, true).unwrap(),
            Some("largest".to_string())
        );
        assert_eq!(
            resolve_virtual_preset("largest", false, false).unwrap(),
            Some("largest".to_string())
        );
        assert_eq!(
            resolve_virtual_preset("mirror", false, false).unwrap(),
            Some("mirror".to_string())
        );
        assert_eq!(
            resolve_virtual_preset("horizontal", true, false).unwrap(),
            Some("horizontal-reverse".to_string())
        );
        assert_eq!(resolve_virtual_preset("desk", false, false).unwrap(), None);
    }

    #[test]
    fn mirror_unavailable_guidance_mentions_backend_and_wl_mirror() {
        let message = mirror_unavailable_message("wlroots");

        assert!(message.contains("wlroots"));
        assert!(message.contains("wl-mirror"));
    }

    #[test]
    fn common_unavailable_guidance_mentions_niri_and_alternatives() {
        let message = common_unavailable_message("wlroots");

        assert!(message.contains("wlroots"));
        assert!(message.contains("Niri"));
        assert!(message.contains("wl-mirror"));
        assert!(message.contains("horizontal"));
        assert!(message.contains("vertical"));
    }
}
