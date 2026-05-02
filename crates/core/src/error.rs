use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BackendConnectionError {
    #[error("unknown backend label `{label}`")]
    UnknownBackendLabel { label: String },
    #[error("failed to initialize {backend} backend: {source}")]
    Initialize {
        backend: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "failed to connect to a supported backend (WAYLAND_DISPLAY={wayland_display}, XDG_RUNTIME_DIR={xdg_runtime_dir}{display_hint}); attempts: {}", format_backend_connection_attempts(attempts)
    )]
    NoSupportedBackend {
        wayland_display: String,
        xdg_runtime_dir: String,
        display_hint: String,
        attempts: Vec<BackendConnectionAttempt>,
    },
}

#[non_exhaustive]
#[derive(Debug)]
pub struct BackendConnectionAttempt {
    pub backend: &'static str,
    pub error: anyhow::Error,
}

#[derive(Debug)]
pub enum PlanError {
    MissingOutput(String),
    InvalidConfiguration(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOutput(output) => write!(f, "Missing output: {output}"),
            Self::InvalidConfiguration(message) => {
                write!(f, "Invalid configuration: {message}")
            }
        }
    }
}

impl std::error::Error for PlanError {}

impl BackendConnectionAttempt {
    #[must_use]
    pub fn new(backend: &'static str, error: impl Into<anyhow::Error>) -> Self {
        Self {
            backend,
            error: error.into(),
        }
    }
}

impl std::fmt::Display for BackendConnectionAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.backend, self.error)
    }
}

struct BackendConnectionAttemptsDisplay<'a>(&'a [BackendConnectionAttempt]);

impl std::fmt::Display for BackendConnectionAttemptsDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut attempts = self.0.iter();
        let Some(first) = attempts.next() else {
            return f.write_str("none");
        };

        write!(f, "{first}")?;
        for attempt in attempts {
            write!(f, "; {attempt}")?;
        }
        Ok(())
    }
}

fn format_backend_connection_attempts(
    attempts: &[BackendConnectionAttempt],
) -> BackendConnectionAttemptsDisplay<'_> {
    BackendConnectionAttemptsDisplay(attempts)
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("cannot determine config directory")]
    MissingConfigDirectory,
    #[error("cannot determine state directory")]
    MissingStateDirectory,
    #[error("cannot determine state directory path")]
    MissingStateDirectoryPath,
    #[error("failed to create directory {path:?}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read directory {path:?}")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read file {path:?}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("refusing to read {path:?}: file is {actual_bytes} bytes, limit is {max_bytes} bytes")]
    FileTooLarge {
        path: PathBuf,
        actual_bytes: u64,
        max_bytes: u64,
    },
    #[error("failed to write file {path:?}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize TOML for {path:?}")]
    SerializeToml {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
    #[error("failed to parse TOML from {path:?}")]
    ParseToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize JSON for {path:?}")]
    SerializeJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse JSON from {path:?}")]
    ParseJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "profile '{0}' is ambiguous across setup fingerprints; use the matching hardware setup"
    )]
    AmbiguousProfile(String),
    #[error(
        "legacy profile migration conflict for '{name}' between {legacy_path:?} and {setup_path:?}"
    )]
    LegacyProfileConflict {
        name: String,
        legacy_path: PathBuf,
        setup_path: PathBuf,
    },
    #[error("too many legacy profiles in {path:?}: {actual} found, limit is {max}")]
    TooManyLegacyProfiles {
        path: PathBuf,
        actual: usize,
        max: usize,
    },
    #[error("invalid profile name '{name}': {reason}")]
    InvalidProfileName { name: String, reason: String },
    #[error("refusing hook-bearing profiles from untrusted profile store {path:?}: {reason}")]
    UntrustedProfileStore { path: PathBuf, reason: String },
    #[error("invalid backend topology: {0}")]
    InvalidTopology(String),
    #[error("profile does not match current topology")]
    ProfileMismatch,
    #[error("backend error: {source}")]
    Backend {
        #[source]
        source: anyhow::Error,
    },
    #[error(transparent)]
    BackendConnection(#[from] BackendConnectionError),
    #[error("plan error: {source}")]
    Plan {
        #[from]
        source: PlanError,
    },
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_profile_error_is_readable() {
        let error = CoreError::AmbiguousProfile("desk".to_string());

        assert!(error.to_string().contains("desk"));
    }

    #[test]
    fn backend_error_display_includes_source() {
        let error = CoreError::Backend {
            source: anyhow::anyhow!("dbus call failed"),
        };

        assert_eq!(error.to_string(), "backend error: dbus call failed");
    }

    #[test]
    fn plan_error_display_includes_source() {
        let error = CoreError::from(PlanError::MissingOutput("DP-1".to_string()));

        assert_eq!(error.to_string(), "plan error: Missing output: DP-1");
    }

    #[test]
    fn no_supported_backend_display_uses_stable_attempt_format() {
        let error = BackendConnectionError::NoSupportedBackend {
            wayland_display: "<unset>".to_string(),
            xdg_runtime_dir: "/run/user/1000".to_string(),
            display_hint: String::new(),
            attempts: vec![
                BackendConnectionAttempt::new("gnome", anyhow::anyhow!("dbus unavailable")),
                BackendConnectionAttempt::new("wlroots", anyhow::anyhow!("protocol missing")),
            ],
        };

        assert_eq!(
            error.to_string(),
            "failed to connect to a supported backend (WAYLAND_DISPLAY=<unset>, XDG_RUNTIME_DIR=/run/user/1000); attempts: gnome: dbus unavailable; wlroots: protocol missing"
        );
    }
}
