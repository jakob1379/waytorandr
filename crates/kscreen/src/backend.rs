use std::ffi::OsString;
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use waytorandr_core::LayoutPlan;
use waytorandr_core::{
    ApplyResult, Backend, ConfigFailureKind, OutputWatcher, PollingOutputWatcher, ValidationResult,
};
use waytorandr_core::{BackendConnectionError, CoreError, CoreResult};
use waytorandr_core::{BackendKind, Capabilities, Topology};

mod apply;
mod state;

use apply::build_apply_args_or_rejection;
use state::{export_kscreen_topology, KScreenConfig};

const KSCREEN_DOCTOR: &str = "kscreen-doctor";
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct KScreenBackend {
    command: OsString,
}

impl KScreenBackend {
    /// Connects to the `KScreen` backend.
    ///
    /// # Errors
    /// Returns an error if `kscreen-doctor` is unavailable or the current session cannot be queried.
    pub fn connect() -> CoreResult<Self> {
        let backend = Self {
            command: std::env::var_os("WAYTORANDR_KSCREEN_DOCTOR")
                .unwrap_or_else(|| OsString::from(KSCREEN_DOCTOR)),
        };
        backend
            .load_config()
            .context("failed to query KScreen display configuration")
            .map_err(|source| {
                CoreError::BackendConnection(BackendConnectionError::Initialize {
                    backend: BackendKind::KScreen.as_str(),
                    source,
                })
            })?;
        Ok(backend)
    }

    fn load_config(&self) -> Result<KScreenConfig> {
        let output = self.run_command(&[String::from("--json")])?;
        if !output.status.success() {
            bail!(
                "`{} --json` failed: {}",
                self.command_label(),
                describe_command_output(&output)
            );
        }

        serde_json::from_slice(&output.stdout)
            .with_context(|| format!("failed to parse `{} --json` output", self.command_label()))
    }

    fn run_command(&self, args: &[String]) -> Result<Output> {
        let mut command = Command::new(&self.command);
        command.args(args);
        command.output().with_context(|| {
            format!(
                "failed to run `{}` with args {:?}",
                self.command.to_string_lossy(),
                args
            )
        })
    }

    fn apply_plan(&self, plan: &LayoutPlan) -> Result<ApplyResult> {
        let config = self.load_config()?;
        let args = match build_apply_args_or_rejection(plan, &config) {
            Ok(args) => args,
            Err(result) => return Ok(result),
        };
        if args.is_empty() {
            return Ok(ApplyResult::applied(
                Some("configuration already matches current state".to_string()),
                Some(export_kscreen_topology(&config)),
            ));
        }

        let output = self.run_command(&args)?;
        if !output.status.success() {
            return Ok(ApplyResult::failed(
                Some(ConfigFailureKind::Rejected),
                Some(format!(
                    "`{}` rejected the configuration: {}",
                    self.command_label(),
                    describe_command_output(&output)
                )),
            ));
        }

        let applied = self.load_config()?;
        Ok(ApplyResult::applied(
            Some(format!("KScreen applied {} display changes", args.len())),
            Some(export_kscreen_topology(&applied)),
        ))
    }

    fn command_label(&self) -> String {
        self.command.to_string_lossy().into_owned()
    }
}

impl Backend for KScreenBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::KScreen);
        capabilities.can_validate = false;
        capabilities.supports_mirror = true;
        capabilities.supports_largest_mirror = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        let config = self
            .load_config()
            .map_err(|source| CoreError::Backend { source })?;
        Ok(export_kscreen_topology(&config))
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        let initial = self.enumerate_outputs()?;
        Ok(Box::new(PollingOutputWatcher::new(
            self.clone(),
            POLL_INTERVAL,
            Some(initial.setup_fingerprint()),
        )))
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        Ok(ValidationResult::unsupported(Some(format!(
            "KScreen does not provide a dry-run API; {} output changes were planned",
            plan.outputs.len()
        ))))
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        self.apply_plan(plan)
            .map_err(|source| CoreError::Backend { source })
    }
}

fn describe_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("exit status {}", output.status),
        (false, true) => format!("stdout: {stdout}"),
        (true, false) => format!("stderr: {stderr}"),
        (false, false) => format!("stdout: {stdout}; stderr: {stderr}"),
    }
}

#[cfg(test)]
mod tests;
