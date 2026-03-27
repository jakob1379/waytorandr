use std::path::PathBuf;

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
    #[error("failed to write file {path:?}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize TOML")]
    SerializeToml(#[from] toml::ser::Error),
    #[error("failed to parse TOML from {path:?}")]
    ParseToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize JSON")]
    SerializeJson(#[source] serde_json::Error),
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
    #[error("profile does not match current topology")]
    ProfileMismatch,
    #[error("backend error")]
    Backend {
        #[source]
        source: anyhow::Error,
    },
    #[error("plan error")]
    Plan {
        #[from]
        source: crate::planner::PlanError,
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
}
