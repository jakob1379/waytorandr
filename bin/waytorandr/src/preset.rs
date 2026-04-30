use anyhow::{bail, Result};

use waytorandr_core::model::VirtualPreset;

pub fn is_builtin_set_target(name: &str) -> bool {
    matches!(
        name,
        "auto" | "off" | "external" | "common" | "largest" | "mirror" | "horizontal" | "vertical"
    )
}

pub fn resolve_virtual_preset(
    name: &str,
    reverse: bool,
    largest: bool,
) -> Result<Option<VirtualPreset>> {
    let preset = match name {
        "off" => Some(VirtualPreset::Off),
        "external" => Some(VirtualPreset::External),
        "common" => Some(if largest {
            VirtualPreset::Largest
        } else {
            VirtualPreset::Common
        }),
        "largest" => Some(VirtualPreset::Largest),
        "mirror" => Some(VirtualPreset::Mirror),
        "horizontal" => Some(if reverse {
            VirtualPreset::HorizontalReverse
        } else {
            VirtualPreset::Horizontal
        }),
        "vertical" => Some(if reverse {
            VirtualPreset::VerticalReverse
        } else {
            VirtualPreset::Vertical
        }),
        _ => None,
    };

    if reverse
        && !matches!(
            preset,
            Some(
                VirtualPreset::Horizontal
                    | VirtualPreset::HorizontalReverse
                    | VirtualPreset::Vertical
                    | VirtualPreset::VerticalReverse
            )
        )
    {
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

pub fn virtual_completion_candidates(
    current: &str,
) -> Vec<clap_complete::engine::CompletionCandidate> {
    [
        ("auto", "selection"),
        ("off", "virtual"),
        ("external", "virtual"),
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
    fn resolve_virtual_preset_prefers_reverse_horizontal() {
        assert_eq!(
            resolve_virtual_preset("horizontal", true, false).unwrap(),
            Some(VirtualPreset::HorizontalReverse)
        );
    }

    #[test]
    fn resolve_virtual_preset_maps_common_to_largest_when_requested() {
        assert_eq!(
            resolve_virtual_preset("common", false, true).unwrap(),
            Some(VirtualPreset::Largest)
        );
    }

    #[test]
    fn completion_candidates_filter_by_prefix() {
        let names: Vec<_> = virtual_completion_candidates("ver")
            .into_iter()
            .map(|candidate| candidate.get_value().to_str().unwrap().to_string())
            .collect();

        assert_eq!(names, vec!["vertical"]);
    }

    #[test]
    fn completion_candidates_include_auto() {
        let names: Vec<_> = virtual_completion_candidates("au")
            .into_iter()
            .map(|candidate| candidate.get_value().to_str().unwrap().to_string())
            .collect();

        assert_eq!(names, vec!["auto"]);
    }

    #[test]
    fn resolve_virtual_preset_accepts_external() {
        assert_eq!(
            resolve_virtual_preset("external", false, false).unwrap(),
            Some(VirtualPreset::External)
        );
    }

    #[test]
    fn resolve_virtual_preset_rejects_reverse_for_external() {
        let err = resolve_virtual_preset("external", true, false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "--reverse can only be used with virtual 'horizontal' or 'vertical' set targets"
        );
    }
}
