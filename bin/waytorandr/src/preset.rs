use anyhow::{bail, Result};

use waytorandr_core::model::VirtualPreset;

pub fn resolve_virtual_preset(
    name: &str,
    reverse: bool,
    largest: bool,
) -> Result<Option<VirtualPreset>> {
    let preset = match name {
        "off" => Some(VirtualPreset::Off),
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

pub fn virtual_completion_candidates(
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
