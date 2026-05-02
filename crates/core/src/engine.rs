use std::thread;
use std::time::Duration;

use crate::error::CoreResult;
use crate::model::{Capabilities, Topology};
use crate::planning::LayoutPlan;
use crate::profile::Hooks;

mod hooks;

use hooks::{format_hook_failure, run_hooks};

pub trait Backend {
    #[must_use]
    /// Returns backend capabilities.
    fn capabilities(&self) -> Capabilities;

    /// Enumerates the current outputs.
    ///
    /// # Errors
    /// Returns an error if the backend cannot read the current output state.
    fn enumerate_outputs(&self) -> CoreResult<Topology>;

    /// Returns an output watcher.
    ///
    /// # Errors
    /// Returns an error if watch mode is unavailable.
    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>>;

    /// Validates a layout plan.
    ///
    /// Validation outcomes are returned as `ValidationResult` values.
    ///
    /// # Errors
    /// Returns an error only if backend validation transport fails.
    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult>;

    /// Applies a layout plan.
    ///
    /// Apply outcomes are returned as `ApplyResult` values. Planned
    /// configuration rejections should be reported with `ApplyResult::failed`;
    /// transport or backend execution failures should be returned as errors.
    ///
    /// # Errors
    /// Returns an error only if backend apply transport or execution fails.
    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult>;
}

pub trait OutputWatcher {
    /// Polls for physical output changes.
    ///
    /// # Errors
    /// Returns an error if the backend watcher fails.
    fn poll_changed(&mut self) -> CoreResult<Option<Topology>>;
}

impl<B: Backend + ?Sized> Backend for &B {
    fn capabilities(&self) -> Capabilities {
        (*self).capabilities()
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        (*self).enumerate_outputs()
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        (*self).watch_outputs()
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        (*self).validate(plan)
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        (*self).apply(plan)
    }
}

pub struct PollingOutputWatcher<B> {
    backend: B,
    interval: Duration,
    last_setup_fingerprint: Option<String>,
}

impl<B> PollingOutputWatcher<B> {
    #[must_use]
    pub fn new(backend: B, interval: Duration, last_setup_fingerprint: Option<String>) -> Self {
        Self {
            backend,
            interval,
            last_setup_fingerprint,
        }
    }
}

impl<B: Backend> OutputWatcher for PollingOutputWatcher<B> {
    fn poll_changed(&mut self) -> CoreResult<Option<Topology>> {
        thread::sleep(self.interval);
        let topology = self.backend.enumerate_outputs()?;
        let setup_fingerprint = topology.setup_fingerprint();
        if self.last_setup_fingerprint.as_ref() == Some(&setup_fingerprint) {
            return Ok(None);
        }
        self.last_setup_fingerprint = Some(setup_fingerprint);
        Ok(Some(topology))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFailureKind {
    Rejected,
    TopologyChanged,
}

impl ConfigFailureKind {
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::TopologyChanged => "topology_changed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    Supported,
    Rejected,
    Unsupported,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ValidationResult {
    pub status: ValidationStatus,
    failure: Option<ConfigFailureKind>,
    pub message: Option<String>,
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self {
            status: ValidationStatus::Unsupported,
            failure: None,
            message: None,
        }
    }
}

impl ValidationResult {
    #[must_use]
    pub fn supported(message: Option<String>) -> Self {
        Self {
            status: ValidationStatus::Supported,
            failure: None,
            message,
        }
    }

    #[must_use]
    pub fn rejected(failure: Option<ConfigFailureKind>, message: Option<String>) -> Self {
        Self {
            status: ValidationStatus::Rejected,
            failure,
            message,
        }
    }

    #[must_use]
    pub fn unsupported(message: Option<String>) -> Self {
        Self {
            status: ValidationStatus::Unsupported,
            failure: None,
            message,
        }
    }

    #[must_use]
    pub fn is_accepted(&self) -> bool {
        self.status == ValidationStatus::Supported
    }

    #[must_use]
    pub fn failure(&self) -> Option<ConfigFailureKind> {
        match self.status {
            ValidationStatus::Supported | ValidationStatus::Unsupported => None,
            ValidationStatus::Rejected => self.failure,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyStatus {
    Applied,
    Failed { failure: Option<ConfigFailureKind> },
}

impl Default for ApplyStatus {
    fn default() -> Self {
        Self::Failed { failure: None }
    }
}

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ApplyResult {
    status: ApplyStatus,
    message: Option<String>,
    applied_state: Option<Topology>,
}

impl ApplyResult {
    #[must_use]
    pub fn applied(message: Option<String>, applied_state: Option<Topology>) -> Self {
        Self {
            status: ApplyStatus::Applied,
            message,
            applied_state,
        }
    }

    #[must_use]
    pub fn failed(failure: Option<ConfigFailureKind>, message: Option<String>) -> Self {
        Self {
            status: ApplyStatus::Failed { failure },
            message,
            applied_state: None,
        }
    }

    #[must_use]
    pub fn is_applied(&self) -> bool {
        self.status == ApplyStatus::Applied
    }

    #[must_use]
    pub const fn status(&self) -> ApplyStatus {
        self.status
    }

    #[must_use]
    pub fn failure(&self) -> Option<ConfigFailureKind> {
        match self.status {
            ApplyStatus::Applied => None,
            ApplyStatus::Failed { failure } => failure,
        }
    }

    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    #[must_use]
    pub const fn applied_state(&self) -> Option<&Topology> {
        self.applied_state.as_ref()
    }
}

pub struct Engine<B: Backend> {
    backend: B,
}

impl<B: Backend> Engine<B> {
    #[must_use]
    pub(crate) fn new(backend: B) -> Self {
        Self { backend }
    }

    #[must_use]
    pub(crate) fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    pub(crate) fn validate_plan(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        if !self.capabilities().can_validate {
            return Ok(ValidationResult::unsupported(Some(
                "Backend does not support validation".to_string(),
            )));
        }
        self.backend.validate(plan)
    }

    /// Applies a plan and runs the configured hooks.
    ///
    /// # Errors
    /// Returns an error only if backend apply transport fails. Backend
    /// rejections and hook failures are returned in the `ApplyResult`.
    pub(crate) fn apply_plan(&self, plan: &LayoutPlan, hooks: &Hooks) -> CoreResult<ApplyResult> {
        let count = plan.outputs.len();
        tracing::info!("Applying plan for {count} outputs");

        for hook in &hooks.pre_apply {
            let command = &hook.command;
            tracing::debug!("Running pre-apply hook: {command}");
        }

        let pre_results = run_hooks(&hooks.pre_apply, "pre-apply");
        if !pre_results.iter().all(|r| r.success) {
            let message = pre_results
                .iter()
                .find(|result| !result.success)
                .map_or_else(|| "Pre-apply hooks failed".to_string(), format_hook_failure);
            return Ok(ApplyResult::failed(
                Some(ConfigFailureKind::Rejected),
                Some(message),
            ));
        }

        let mut result = self.backend.apply(plan)?;

        if result.is_applied() {
            let post_results = run_hooks(&hooks.post_apply, "post-apply");
            if let Some(failure) = first_hook_failure(&post_results) {
                append_apply_message(&mut result, format_hook_failure(failure));
            }
            tracing::debug!(
                ran = post_results.len(),
                failed = post_results.iter().filter(|result| !result.success).count(),
                "Post-apply hooks completed"
            );
        } else {
            let failure_results = run_hooks(&hooks.on_failure, "failure");
            if let Some(failure) = first_hook_failure(&failure_results) {
                append_apply_message(&mut result, format_hook_failure(failure));
            }
            tracing::debug!(
                ran = failure_results.len(),
                failed = failure_results
                    .iter()
                    .filter(|result| !result.success)
                    .count(),
                "Failure hooks completed"
            );
        }

        Ok(result)
    }
}

fn first_hook_failure(results: &[HookResult]) -> Option<&HookResult> {
    results.iter().find(|result| !result.success)
}

fn append_apply_message(result: &mut ApplyResult, message: String) {
    result.message = Some(match result.message.take() {
        Some(existing) if !existing.is_empty() => format!("{existing}; {message}"),
        _ => message,
    });
}

pub use hooks::HookResult;

#[cfg(test)]
mod tests;
