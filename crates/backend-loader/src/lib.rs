use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use waytorandr_core::engine::{ApplyResult, Backend, OutputWatcher, TestResult};
use waytorandr_core::error::{
    BackendConnectionAttempt, BackendConnectionError, CoreError, CoreResult,
};
use waytorandr_core::model::{BackendKind, Capabilities, Topology};

/// Connects to the first available backend.
///
/// # Errors
/// Returns an error if no supported backend can be initialized.
pub fn connect_backend() -> CoreResult<Box<dyn Backend>> {
    #[cfg(debug_assertions)]
    if let Some(backend) = connect_test_backend() {
        return Ok(Box::new(backend));
    }

    let env = SessionEnvironment::from_process();
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".to_string());
    let xdg_runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "<unset>".to_string());
    let display_hint = if wayland_display.contains('/') {
        "; WAYLAND_DISPLAY should be a socket name like 'wayland-0', not a path"
    } else {
        ""
    };
    let mut attempts = Vec::new();

    for label in backend_labels_for_env(&env) {
        let result = match label {
            "gnome" => waytorandr_gnome::backend::GnomeBackend::connect()
                .map(|backend| Box::new(backend) as Box<dyn Backend>),
            "kscreen" => waytorandr_kscreen::backend::KScreenBackend::connect()
                .map(|backend| Box::new(backend) as Box<dyn Backend>),
            "wlroots" => waytorandr_wlroots::backend::WlrootsBackend::connect()
                .map(|backend| Box::new(backend) as Box<dyn Backend>),
            other => Err(CoreError::BackendConnection(
                BackendConnectionError::UnknownBackendLabel {
                    label: other.to_string(),
                },
            )),
        };

        match result {
            Ok(backend) => return Ok(backend),
            Err(
                err @ CoreError::BackendConnection(BackendConnectionError::UnknownBackendLabel {
                    ..
                }),
            ) => return Err(err),
            Err(err) => attempts.push(BackendConnectionAttempt {
                backend: backend_kind_for_label(label),
                error: err.to_string(),
            }),
        }
    }

    Err(CoreError::BackendConnection(
        BackendConnectionError::NoSupportedBackend {
            wayland_display,
            xdg_runtime_dir,
            display_hint: display_hint.to_string(),
            attempts,
        },
    ))
}

const TEST_BACKEND_STATE_ENV: &str = "WAYTORANDR_TEST_BACKEND_STATE";
const TEST_BACKEND_NAME_ENV: &str = "WAYTORANDR_TEST_BACKEND_NAME";
const TEST_BACKEND_SUPPORTS_MIRROR_ENV: &str = "WAYTORANDR_TEST_BACKEND_SUPPORTS_MIRROR";

#[cfg(debug_assertions)]
fn connect_test_backend() -> Option<TestBackend> {
    let path = std::env::var_os(TEST_BACKEND_STATE_ENV)?;

    let path = PathBuf::from(path);
    let backend_name = std::env::var(TEST_BACKEND_NAME_ENV).unwrap_or_else(|_| "test".to_string());
    let backend_kind = BackendKind::from_name(&backend_name).unwrap_or(BackendKind::Test);
    let supports_mirror = std::env::var(TEST_BACKEND_SUPPORTS_MIRROR_ENV)
        .ok()
        .and_then(|value| parse_env_bool(&value))
        .unwrap_or(backend_kind.is_native_mirror_backend());

    Some(TestBackend {
        path,
        capabilities: test_backend_capabilities(backend_kind, supports_mirror),
    })
}

#[cfg(debug_assertions)]
fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(debug_assertions)]
fn test_backend_capabilities(backend: BackendKind, supports_mirror: bool) -> Capabilities {
    let mut capabilities = Capabilities::new(backend);
    capabilities.can_test = true;
    capabilities.supports_mirror = supports_mirror;
    capabilities.supports_largest_mirror =
        capabilities.supports_mirror && capabilities.backend != BackendKind::Gnome;
    capabilities
}

#[cfg(debug_assertions)]
#[derive(Debug)]
struct TestBackend {
    path: PathBuf,
    capabilities: Capabilities,
}

#[cfg(debug_assertions)]
#[derive(Debug, Serialize, Deserialize)]
struct TestBackendState {
    topology: Topology,
}

#[cfg(debug_assertions)]
impl TestBackend {
    fn load_state(&self) -> CoreResult<TestBackendState> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|source| CoreError::ReadFile {
                path: self.path.clone(),
                source,
            })?;
        serde_json::from_str(&content).map_err(|source| CoreError::ParseJson {
            path: self.path.clone(),
            source,
        })
    }

    fn save_state(&self, state: &TestBackendState) -> CoreResult<()> {
        let content = serde_json::to_string_pretty(state).map_err(CoreError::SerializeJson)?;
        std::fs::write(&self.path, format!("{content}\n")).map_err(|source| CoreError::WriteFile {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(debug_assertions)]
impl Backend for TestBackend {
    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        Ok(self.load_state()?.topology)
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow!("test backend does not support watch mode"),
        })
    }

    fn test(&self, _plan: &waytorandr_core::planner::LayoutPlan) -> CoreResult<TestResult> {
        Ok(TestResult::supported(None))
    }

    fn apply(&self, plan: &waytorandr_core::planner::LayoutPlan) -> CoreResult<ApplyResult> {
        let topology = Topology {
            outputs: plan.outputs.clone(),
        };
        self.save_state(&TestBackendState {
            topology: topology.clone(),
        })?;

        let mut result = ApplyResult::default();
        result.success = true;
        result.applied_state = Some(topology);
        Ok(result)
    }
}

fn backend_labels_for_env(env: &SessionEnvironment) -> Vec<&'static str> {
    if env.is_kde_session() {
        vec!["kscreen", "wlroots", "gnome"]
    } else if env.is_gnome_session() {
        vec!["gnome", "wlroots", "kscreen"]
    } else {
        vec!["wlroots", "kscreen", "gnome"]
    }
}

fn backend_kind_for_label(label: &str) -> BackendKind {
    match label {
        "gnome" => BackendKind::Gnome,
        "kscreen" => BackendKind::KScreen,
        "wlroots" => BackendKind::Wlroots,
        _ => BackendKind::Unknown,
    }
}

#[derive(Clone, Debug, Default)]
struct SessionEnvironment {
    current_desktop: Option<String>,
    session_desktop: Option<String>,
    desktop_session: Option<String>,
}

impl SessionEnvironment {
    fn from_process() -> Self {
        Self {
            current_desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
            session_desktop: std::env::var("XDG_SESSION_DESKTOP").ok(),
            desktop_session: std::env::var("DESKTOP_SESSION").ok(),
        }
    }

    fn is_gnome_session(&self) -> bool {
        self.values().any(|value| value.contains("gnome"))
    }

    fn is_kde_session(&self) -> bool {
        self.values()
            .any(|value| value.contains("kde") || value.contains("plasma"))
    }

    fn values(&self) -> impl Iterator<Item = String> + '_ {
        [
            self.current_desktop.as_deref(),
            self.session_desktop.as_deref(),
            self.desktop_session.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_ascii_lowercase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_sessions_prefer_gnome_backend() {
        let env = SessionEnvironment {
            current_desktop: Some("GNOME".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backend_labels_for_env(&env),
            vec!["gnome", "wlroots", "kscreen"]
        );
    }

    #[test]
    fn kde_sessions_prefer_kscreen_backend() {
        let env = SessionEnvironment {
            session_desktop: Some("plasma".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backend_labels_for_env(&env),
            vec!["kscreen", "wlroots", "gnome"]
        );
    }

    #[test]
    fn unknown_sessions_prefer_wlroots_backend() {
        let env = SessionEnvironment {
            desktop_session: Some("niri".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backend_labels_for_env(&env),
            vec!["wlroots", "kscreen", "gnome"]
        );
    }
}
