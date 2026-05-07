use std::thread;
use std::time::Duration;

use crate::error::{CoreError, CoreResult};
use crate::model::{Capabilities, Topology};
use crate::planner::LayoutPlan;
use crate::profile::{Hook, Hooks};
use crate::terminal::escape_terminal_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPolicy {
    Enabled,
    Disabled,
}

impl Default for HookPolicy {
    fn default() -> Self {
        Self::Enabled
    }
}

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
    /// Validation outcomes are returned as `TestResult` values.
    ///
    /// # Errors
    /// Returns an error only if backend validation transport fails.
    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult>;

    /// Applies a layout plan.
    ///
    /// # Errors
    /// Returns an error if the backend cannot apply the plan.
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

    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        (*self).test(plan)
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
        topology
            .validate_limits()
            .map_err(CoreError::InvalidTopology)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    Supported,
    Rejected,
    Unsupported,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TestResult {
    pub success: bool,
    pub status: ValidationStatus,
    pub failure: Option<ConfigFailureKind>,
    pub message: Option<String>,
}

impl Default for TestResult {
    fn default() -> Self {
        Self {
            success: false,
            status: ValidationStatus::Unsupported,
            failure: None,
            message: None,
        }
    }
}

impl TestResult {
    #[must_use]
    pub fn supported(message: Option<String>) -> Self {
        Self {
            success: true,
            status: ValidationStatus::Supported,
            failure: None,
            message,
        }
    }

    #[must_use]
    pub fn rejected(failure: Option<ConfigFailureKind>, message: Option<String>) -> Self {
        Self {
            success: false,
            status: ValidationStatus::Rejected,
            failure,
            message,
        }
    }

    #[must_use]
    pub fn unsupported(message: Option<String>) -> Self {
        Self {
            success: false,
            status: ValidationStatus::Unsupported,
            failure: None,
            message,
        }
    }

    #[must_use]
    pub fn is_accepted(&self) -> bool {
        self.status == ValidationStatus::Supported
    }
}

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ApplyResult {
    pub success: bool,
    pub failure: Option<ConfigFailureKind>,
    pub message: Option<String>,
    pub applied_state: Option<Topology>,
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

    /// Validates a plan against the backend.
    ///
    /// Validation outcomes are returned as `TestResult` values.
    ///
    /// # Errors
    /// Returns an error only if backend validation transport fails.
    pub(crate) fn test_plan(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        if !self.capabilities().can_test {
            return Ok(TestResult::unsupported(Some(
                "Backend does not support test mode".to_string(),
            )));
        }
        self.backend.test(plan)
    }

    /// Applies a plan and runs the configured hooks.
    ///
    /// # Errors
    /// Returns an error if the backend apply step fails.
    pub(crate) fn apply_plan(
        &self,
        plan: &LayoutPlan,
        hooks: &Hooks,
        hook_policy: HookPolicy,
    ) -> CoreResult<ApplyResult> {
        let count = plan.outputs.len();
        tracing::info!("Applying plan for {count} outputs");

        if hook_policy == HookPolicy::Disabled {
            tracing::debug!("Skipping hooks because hook execution is disabled");
            return self.backend.apply(plan);
        }

        for hook in &hooks.pre_apply {
            let command = escape_terminal_text(&hook.command);
            tracing::debug!("Running pre-apply hook: {command}");
        }

        let pre_results = Self::run_hooks(&hooks.pre_apply, "pre-apply");
        if !pre_results.iter().all(|r| r.success) {
            let message = pre_results
                .iter()
                .find(|result| !result.success)
                .map_or_else(|| "Pre-apply hooks failed".to_string(), format_hook_failure);
            return Ok(ApplyResult {
                success: false,
                failure: Some(ConfigFailureKind::Rejected),
                message: Some(message),
                applied_state: None,
            });
        }

        let result = self.backend.apply(plan)?;

        if result.success {
            let post_results = Self::run_hooks(&hooks.post_apply, "post-apply");
            tracing::debug!(
                ran = post_results.len(),
                failed = post_results.iter().filter(|result| !result.success).count(),
                "Post-apply hooks completed"
            );
        } else {
            let failure_results = Self::run_hooks(&hooks.on_failure, "failure");
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

    fn run_hooks(hooks: &[Hook], phase: &str) -> Vec<HookResult> {
        let mut results = Vec::new();
        for hook in hooks {
            let result = Self::execute_hook(hook, phase);
            results.push(result);
        }
        results
    }

    fn execute_hook(hook: &Hook, phase: &str) -> HookResult {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let timeout = Duration::from_secs(hook.effective_timeout_secs());

        let mut cmd = Command::new(&hook.command);
        cmd.args(&hook.args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        prepare_hook_command(&mut cmd);

        match cmd.spawn() {
            Ok(mut child) => loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let elapsed = start.elapsed();
                        return HookResult {
                            success: status.success(),
                            exit_code: status.code(),
                            elapsed_secs: elapsed.as_secs_f64(),
                            stdout: String::new(),
                            stderr: String::new(),
                            phase: Some(phase.to_string()),
                            command: Some(hook.command.clone()),
                        };
                    }
                    Ok(None) => {
                        if start.elapsed() > timeout {
                            let timeout_message = match kill_hook_child(&mut child) {
                                Ok(()) => "Hook timed out".to_string(),
                                Err(err) => {
                                    format!("Hook timed out and could not be killed: {err}")
                                }
                            };
                            let _ = child.wait();
                            return HookResult {
                                success: false,
                                exit_code: None,
                                elapsed_secs: timeout.as_secs_f64(),
                                stdout: String::new(),
                                stderr: timeout_message,
                                phase: Some(phase.to_string()),
                                command: Some(hook.command.clone()),
                            };
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(err) => {
                        return HookResult {
                            success: false,
                            exit_code: None,
                            elapsed_secs: start.elapsed().as_secs_f64(),
                            stdout: String::new(),
                            stderr: format!("Hook wait failed: {err}"),
                            phase: Some(phase.to_string()),
                            command: Some(hook.command.clone()),
                        };
                    }
                }
            },
            Err(err) => HookResult {
                success: false,
                exit_code: None,
                elapsed_secs: 0.0,
                stdout: String::new(),
                stderr: format!("Failed to spawn: {err}"),
                phase: Some(phase.to_string()),
                command: Some(hook.command.clone()),
            },
        }
    }
}

fn format_hook_failure(result: &HookResult) -> String {
    let phase = result.phase.as_deref().unwrap_or("hook");
    let command = escape_terminal_text(result.command.as_deref().unwrap_or("<unknown>"));
    if result.stderr.is_empty() {
        format!("{phase} hook '{command}' failed")
    } else {
        let stderr = &result.stderr;
        format!("{phase} hook '{command}' failed: {stderr}")
    }
}

fn prepare_hook_command(command: &mut std::process::Command) {
    prepare_hook_command_platform(command);
}

#[cfg(unix)]
fn prepare_hook_command_platform(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if unix_process::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn prepare_hook_command_platform(_command: &mut std::process::Command) {}

fn kill_hook_child(child: &mut std::process::Child) -> std::io::Result<()> {
    kill_hook_child_platform(child)
}

#[cfg(unix)]
fn kill_hook_child_platform(child: &mut std::process::Child) -> std::io::Result<()> {
    let pid = i32::try_from(child.id()).unwrap_or(i32::MAX);
    let killed_group = unsafe { unix_process::kill(-pid, unix_process::SIGKILL) };
    if killed_group == -1 {
        let group_error = std::io::Error::last_os_error();
        child.kill()?;
        return Err(group_error);
    }
    Ok(())
}

#[cfg(not(unix))]
fn kill_hook_child_platform(child: &mut std::process::Child) -> std::io::Result<()> {
    child.kill()
}

#[cfg(unix)]
mod unix_process {
    pub const SIGKILL: i32 = 9;

    extern "C" {
        pub fn setsid() -> i32;
        pub fn kill(pid: i32, sig: i32) -> i32;
    }
}

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct HookResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub elapsed_secs: f64,
    pub stdout: String,
    pub stderr: String,
    pub phase: Option<String>,
    pub command: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use crate::model::{BackendKind, OutputState};
    use crate::profile::{Hook, Hooks};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct TestBackend {
        apply_calls: Arc<Mutex<usize>>,
    }

    impl Backend for TestBackend {
        fn capabilities(&self) -> Capabilities {
            let mut capabilities = Capabilities::new(BackendKind::Test);
            capabilities.can_test = true;
            capabilities
        }

        fn enumerate_outputs(&self) -> CoreResult<Topology> {
            Ok(Topology::default())
        }

        fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }

        fn test(&self, _plan: &LayoutPlan) -> CoreResult<TestResult> {
            Ok(TestResult::supported(None))
        }

        fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            *self.apply_calls.lock().unwrap() += 1;
            Ok(ApplyResult {
                success: true,
                applied_state: Some(Topology {
                    outputs: plan.outputs.clone(),
                }),
                ..ApplyResult::default()
            })
        }
    }

    fn test_hooks(log_path: &std::path::Path) -> Hooks {
        let hook = |label: &str| {
            let mut hook = Hook::new("sh");
            let log_path = log_path.display();
            hook.args = vec![
                "-c".to_string(),
                format!("printf '%s\\n' {label} >> {log_path}"),
            ];
            hook.timeout_secs = 5;
            hook
        };

        Hooks {
            pre_apply: vec![hook("pre")],
            post_apply: vec![hook("post")],
            on_failure: vec![hook("failure")],
        }
    }

    #[test]
    fn apply_plan_applies_once_and_runs_phase_specific_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("hooks.log");
        let backend = TestBackend {
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let engine = Engine::new(backend.clone());
        let hooks = test_hooks(&log_path);
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let result = engine
            .apply_plan(&plan, &hooks, HookPolicy::Enabled)
            .unwrap();

        assert!(result.success);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);

        let log = std::fs::read_to_string(log_path).unwrap();
        assert!(log.contains("pre"));
        assert!(log.contains("post"));
        assert!(!log.contains("failure"));
    }

    #[test]
    fn apply_plan_rejects_when_pre_hook_cannot_start() {
        let backend = TestBackend {
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let engine = Engine::new(backend.clone());
        let hooks = Hooks {
            pre_apply: vec![Hook::new("definitely-not-a-real-hook-command")],
            ..Hooks::default()
        };
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let result = engine
            .apply_plan(&plan, &hooks, HookPolicy::Enabled)
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.failure, Some(ConfigFailureKind::Rejected));
        assert!(result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("pre-apply hook")));
        assert_eq!(*backend.apply_calls.lock().unwrap(), 0);
    }

    #[test]
    fn apply_plan_skips_hooks_when_policy_disables_them() {
        let backend = TestBackend {
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let engine = Engine::new(backend.clone());
        let hooks = Hooks {
            pre_apply: vec![Hook::new("definitely-not-a-real-hook-command")],
            ..Hooks::default()
        };
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let result = engine
            .apply_plan(&plan, &hooks, HookPolicy::Disabled)
            .unwrap();

        assert!(result.success);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);
    }

    #[test]
    fn noisy_hook_output_does_not_block_on_pipes() {
        let backend = TestBackend {
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let engine = Engine::new(backend.clone());
        let mut hook = Hook::new("sh");
        hook.args = vec![
            "-c".to_string(),
            "dd if=/dev/zero bs=1024 count=512 2>/dev/null".to_string(),
        ];
        hook.timeout_secs = 5;
        let hooks = Hooks {
            pre_apply: vec![hook],
            ..Hooks::default()
        };
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let result = engine
            .apply_plan(&plan, &hooks, HookPolicy::Enabled)
            .unwrap();

        assert!(result.success);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);
    }

    #[test]
    fn test_plan_short_circuits_when_backend_cannot_test() {
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let no_test_capabilities = Capabilities::new(BackendKind::Test);
        let backend = NoTestBackend {
            capabilities: no_test_capabilities,
        };
        let engine = Engine::new(backend);
        let result = engine.test_plan(&plan).unwrap();

        assert!(!result.success);
        assert_eq!(result.status, ValidationStatus::Unsupported);
        assert_eq!(
            result.message.as_deref(),
            Some("Backend does not support test mode")
        );
    }

    struct NoTestBackend {
        capabilities: Capabilities,
    }

    impl Backend for NoTestBackend {
        fn capabilities(&self) -> Capabilities {
            self.capabilities.clone()
        }

        fn enumerate_outputs(&self) -> CoreResult<Topology> {
            Ok(Topology::default())
        }

        fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }

        fn test(&self, _plan: &LayoutPlan) -> CoreResult<TestResult> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("should not be called"),
            })
        }

        fn apply(&self, _plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }
    }

    #[derive(Clone)]
    struct SequenceBackend {
        states: Arc<Mutex<Vec<Topology>>>,
    }

    impl Backend for SequenceBackend {
        fn capabilities(&self) -> Capabilities {
            Capabilities::new(BackendKind::Test)
        }

        fn enumerate_outputs(&self) -> CoreResult<Topology> {
            let mut states = self.states.lock().unwrap();
            if states.len() > 1 {
                Ok(states.remove(0))
            } else {
                Ok(states.first().cloned().unwrap_or_default())
            }
        }

        fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }

        fn test(&self, _plan: &LayoutPlan) -> CoreResult<TestResult> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }

        fn apply(&self, _plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }
    }

    fn topology_with_output(enabled: bool, x: i32) -> Topology {
        let mut output = OutputState::new("DP-1");
        output.enabled = enabled;
        output.position.x = x;
        Topology {
            outputs: HashMap::from([("DP-1".to_string(), output)]),
        }
    }

    #[test]
    fn polling_output_watcher_ignores_non_physical_layout_changes() {
        let initial = topology_with_output(true, 0);
        let moved = topology_with_output(true, 640);
        let backend = SequenceBackend {
            states: Arc::new(Mutex::new(vec![moved])),
        };
        let mut watcher = PollingOutputWatcher::new(
            backend,
            Duration::from_millis(0),
            Some(initial.setup_fingerprint()),
        );

        let changed = watcher.poll_changed().unwrap();

        assert!(changed.is_none());
    }

    #[test]
    fn polling_output_watcher_reports_physical_setup_changes() {
        let initial = topology_with_output(true, 0);
        let mut changed_topology = Topology::default();
        changed_topology.outputs.insert("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output
        });
        changed_topology.outputs.insert("HDMI-A-1".to_string(), {
            let mut output = OutputState::new("HDMI-A-1");
            output.enabled = true;
            output
        });
        let backend = SequenceBackend {
            states: Arc::new(Mutex::new(vec![changed_topology.clone()])),
        };
        let mut watcher = PollingOutputWatcher::new(
            backend,
            Duration::from_millis(0),
            Some(initial.setup_fingerprint()),
        );

        let changed = watcher.poll_changed().unwrap();

        assert_eq!(changed, Some(changed_topology));
    }

    #[test]
    fn polling_output_watcher_reports_blank_setup_changes() {
        let mut initial = Topology::default();
        initial.outputs.insert("eDP-1".to_string(), {
            let mut output = OutputState::new("eDP-1");
            output.enabled = true;
            output
        });
        initial.outputs.insert("DP-1".to_string(), {
            let mut output = OutputState::new("DP-1");
            output.enabled = true;
            output
        });

        let blank = topology_with_output(false, 0);
        let recovered = topology_with_output(true, 0);
        let backend = SequenceBackend {
            states: Arc::new(Mutex::new(vec![blank.clone(), recovered.clone()])),
        };
        let mut watcher = PollingOutputWatcher::new(
            backend,
            Duration::from_millis(0),
            Some(initial.setup_fingerprint()),
        );

        assert_eq!(watcher.poll_changed().unwrap(), Some(blank));
        assert!(watcher.poll_changed().unwrap().is_none());
    }
}
