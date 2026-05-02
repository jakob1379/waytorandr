use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, OnceLock};

use tempfile::TempDir;
use waytorandr_core::{ApplyResult, Backend, ConfigFailureKind, OutputWatcher, ValidationResult};
use waytorandr_core::{BackendKind, Capabilities, OutputIdentity, OutputState, Position, Topology};
use waytorandr_core::{CoreError, CoreResult, LayoutPlan};
use waytorandr_core::{OutputMatcher, Profile};

pub(crate) type TestResult = Result<(), Box<dyn std::error::Error>>;

fn xdg_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn scoped_env_var(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> ScopedEnvVar {
    let previous = std::env::var_os(key);
    std::env::set_var(key, value);
    ScopedEnvVar { key, previous }
}

pub(crate) fn with_test_dirs<T>(
    f: impl FnOnce(&TempDir) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let _guard = xdg_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir()?;
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    std::fs::create_dir_all(&config_home)?;
    std::fs::create_dir_all(&state_home)?;

    let _config_home = scoped_env_var("XDG_CONFIG_HOME", &config_home);
    let _state_home = scoped_env_var("XDG_STATE_HOME", &state_home);

    f(&temp)
}

pub(crate) fn public_workflow_output_state(connector: &str) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = true;
    state.position = Position::new(0, 0);
    state
}

pub(crate) fn public_workflow_profile(name: &str, connector: &str) -> Profile {
    Profile::new(
        name,
        0,
        vec![OutputMatcher::new(
            OutputIdentity::new(connector),
            true,
            Some(Position::new(0, 0)),
        )],
        HashMap::from([(
            connector.to_string(),
            public_workflow_output_state(connector).into(),
        )]),
    )
}

pub(crate) fn config_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path()
        .join("config")
        .join("waytorandr")
        .join("waytorandr.json")
}

pub(crate) fn legacy_config_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path()
        .join("config")
        .join("waytorandr")
        .join("profiles.json")
}

pub(crate) fn state_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path()
        .join("state")
        .join("waytorandr")
        .join("state.toml")
}

#[derive(Clone)]
pub(crate) struct TestBackend {
    pub(crate) topology: Topology,
    pub(crate) enumerate_calls: Arc<Mutex<usize>>,
    pub(crate) can_validate: bool,
    pub(crate) validation_result: ValidationResult,
    pub(crate) apply_calls: Arc<Mutex<usize>>,
    pub(crate) apply_success: bool,
    pub(crate) apply_failure: Option<ConfigFailureKind>,
    pub(crate) apply_message: Option<String>,
}

impl Backend for TestBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Test);
        capabilities.can_validate = self.can_validate;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        *self
            .enumerate_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Ok(self.topology.clone())
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        Ok(self.validation_result.clone())
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        *self
            .apply_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        let message = self.apply_message.clone();
        let applied_state = Some(Topology {
            outputs: plan.outputs.clone(),
        });
        Ok(if self.apply_success {
            ApplyResult::applied(message, applied_state)
        } else {
            ApplyResult::failed(self.apply_failure, message)
        })
    }
}
