use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use waytorandr_core::{
    ApplyResult, Backend, BackendKind, Capabilities, CoreError, CoreResult, LayoutPlan,
    OutputWatcher, Topology, ValidationResult,
};

const TEST_BACKEND_STATE_ENV: &str = "WAYTORANDR_TEST_BACKEND_STATE";
const TEST_BACKEND_NAME_ENV: &str = "WAYTORANDR_TEST_BACKEND_NAME";
const TEST_BACKEND_SUPPORTS_MIRROR_ENV: &str = "WAYTORANDR_TEST_BACKEND_SUPPORTS_MIRROR";

pub(super) fn connect() -> Option<TestBackend> {
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

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn test_backend_capabilities(backend: BackendKind, supports_mirror: bool) -> Capabilities {
    let mut capabilities = Capabilities::new(backend);
    capabilities.can_validate = true;
    capabilities.supports_mirror = supports_mirror;
    capabilities.supports_largest_mirror =
        capabilities.supports_mirror && capabilities.backend != BackendKind::Gnome;
    capabilities
}

#[derive(Debug)]
pub(super) struct TestBackend {
    path: PathBuf,
    capabilities: Capabilities,
}

#[derive(Debug, Serialize, Deserialize)]
struct TestBackendState {
    topology: Topology,
}

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
        let content =
            serde_json::to_string_pretty(state).map_err(|source| CoreError::SerializeJson {
                path: self.path.clone(),
                source,
            })?;
        std::fs::write(&self.path, format!("{content}\n")).map_err(|source| CoreError::WriteFile {
            path: self.path.clone(),
            source,
        })
    }
}

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

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        Ok(ValidationResult::supported(None))
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        let topology = Topology {
            outputs: plan.outputs.clone(),
        };
        self.save_state(&TestBackendState {
            topology: topology.clone(),
        })?;

        Ok(ApplyResult::applied(None, Some(topology)))
    }
}
