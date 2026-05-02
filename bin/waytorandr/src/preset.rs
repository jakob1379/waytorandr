use anyhow::{bail, Result};

use waytorandr_core::VirtualPreset;

const CLI_SELECTABLE_PRESETS: [VirtualPreset; 8] = [
    VirtualPreset::Off,
    VirtualPreset::External,
    VirtualPreset::Builtin,
    VirtualPreset::Common,
    VirtualPreset::Largest,
    VirtualPreset::Mirror,
    VirtualPreset::Horizontal,
    VirtualPreset::Vertical,
];

fn cli_selectable_presets() -> &'static [VirtualPreset] {
    &CLI_SELECTABLE_PRESETS
}

pub fn is_builtin_set_target(name: &str) -> bool {
    name == "auto" || from_cli_label(name).is_some()
}

pub fn resolve_virtual_preset(name: &str, reverse: bool) -> Result<Option<VirtualPreset>> {
    let preset = from_cli_label(name).map(|preset| match (preset, reverse) {
        (VirtualPreset::Horizontal, true) => VirtualPreset::HorizontalReverse,
        (VirtualPreset::Vertical, true) => VirtualPreset::VerticalReverse,
        _ => preset,
    });

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

    Ok(preset)
}

pub fn virtual_completion_candidates(
    current: &str,
) -> Vec<clap_complete::engine::CompletionCandidate> {
    std::iter::once(("auto", "selection"))
        .chain(
            cli_selectable_presets()
                .iter()
                .copied()
                .map(|preset| (cli_label(preset), "virtual")),
        )
        .filter(|(name, _)| name.starts_with(current))
        .map(|(name, tag)| {
            clap_complete::engine::CompletionCandidate::new(name).tag(Some(tag.into()))
        })
        .collect()
}

fn cli_label(preset: VirtualPreset) -> &'static str {
    preset.as_str()
}

fn from_cli_label(label: &str) -> Option<VirtualPreset> {
    cli_selectable_presets()
        .iter()
        .copied()
        .find(|preset| preset.as_str() == label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_virtual_preset_prefers_reverse_horizontal() {
        assert_eq!(
            resolve_virtual_preset("horizontal", true).unwrap(),
            Some(VirtualPreset::HorizontalReverse)
        );
    }

    #[test]
    fn resolve_virtual_preset_accepts_common() {
        assert_eq!(
            resolve_virtual_preset("common", false).unwrap(),
            Some(VirtualPreset::Common)
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
            resolve_virtual_preset("external", false).unwrap(),
            Some(VirtualPreset::External)
        );
    }

    #[test]
    fn resolve_virtual_preset_rejects_reverse_for_external() {
        let err = resolve_virtual_preset("external", true).unwrap_err();
        assert_eq!(
            err.to_string(),
            "--reverse can only be used with virtual 'horizontal' or 'vertical' set targets"
        );
    }
}
