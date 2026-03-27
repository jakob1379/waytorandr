use anyhow::{bail, Result};

pub(crate) fn resolve_virtual_preset(
    name: &str,
    reverse: bool,
    largest: bool,
) -> Result<Option<String>> {
    let preset = match name {
        "off" => Some(name.to_string()),
        "common" => Some(if largest {
            "common-largest".to_string()
        } else {
            "common".to_string()
        }),
        "mirror" => bail!(mirror_unavailable_message()),
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
        bail!("--largest can only be used with virtual 'common' set targets")
    }

    if largest && !matches!(name, "common") {
        bail!("--largest can only be used with virtual 'common' set targets")
    }

    Ok(preset)
}

pub(crate) fn mirror_unavailable_message() -> &'static str {
    "true display mirroring is not available through generic wlroots output-management today; use 'wl-mirror' for now. See https://github.com/swaywm/wlr-protocols/issues/101"
}

pub(crate) fn virtual_completion_candidates(
    current: &str,
) -> Vec<clap_complete::engine::CompletionCandidate> {
    [
        ("off", "virtual"),
        ("common", "virtual"),
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
            Some("common-largest".to_string())
        );
        assert_eq!(
            resolve_virtual_preset("horizontal", true, false).unwrap(),
            Some("horizontal-reverse".to_string())
        );
        assert_eq!(resolve_virtual_preset("desk", false, false).unwrap(), None);
    }

    #[test]
    fn mirror_preset_returns_guidance_error() {
        let err = resolve_virtual_preset("mirror", false, false).unwrap_err();

        assert!(err.to_string().contains("wl-mirror"));
    }
}
