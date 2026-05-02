use super::VirtualPreset;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Gnome,
    KScreen,
    Wlroots,
    Test,
    #[default]
    Unknown,
}

impl BackendKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gnome => "gnome",
            Self::KScreen => "kscreen",
            Self::Wlroots => "wlroots",
            Self::Test => "test",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub const fn is_native_mirror_backend(self) -> bool {
        matches!(self, Self::Gnome | Self::KScreen)
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.trim().to_ascii_lowercase();
        match lowered.as_str() {
            "gnome" => Some(Self::Gnome),
            "kscreen" => Some(Self::KScreen),
            "wlroots" => Some(Self::Wlroots),
            "test" => Some(Self::Test),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Capabilities {
    #[serde(rename = "can_test")]
    pub can_validate: bool,
    pub supports_mirror: bool,
    pub supports_largest_mirror: bool,
    pub backend: BackendKind,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            can_validate: false,
            supports_mirror: false,
            supports_largest_mirror: false,
            backend: BackendKind::Unknown,
        }
    }
}

impl Capabilities {
    #[must_use]
    pub fn new(backend: BackendKind) -> Self {
        Self {
            backend,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.backend.as_str()
    }

    #[must_use]
    pub fn virtual_preset_unavailable_message(&self, preset: VirtualPreset) -> Option<String> {
        match preset {
            VirtualPreset::Mirror if !self.supports_mirror => {
                Some(format!(
                    "native display mirroring is not available through the `{}` backend; use 'wl-mirror' for now on this compositor. See https://github.com/swaywm/wlr-protocols/issues/101",
                    self.backend
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_parses_and_formats_public_labels() {
        assert_eq!(
            BackendKind::from_name(" kscreen "),
            Some(BackendKind::KScreen)
        );
        assert_eq!(
            BackendKind::from_name("wlroots"),
            Some(BackendKind::Wlroots)
        );
        assert_eq!(BackendKind::from_name("missing"), None);
        assert_eq!(BackendKind::Gnome.to_string(), "gnome");
    }

    #[test]
    fn capabilities_report_native_mirror_policy() {
        let capabilities = Capabilities::new(BackendKind::Wlroots);

        assert_eq!(capabilities.backend_name(), "wlroots");
        assert!(capabilities
            .virtual_preset_unavailable_message(VirtualPreset::Mirror)
            .is_some());
    }
}
