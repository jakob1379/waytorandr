use crate::error::CoreResult;
use crate::model::{Capabilities, Topology};
use crate::planner::LayoutPlan;
use crate::profile::{Hook, Hooks};

pub trait Backend {
    fn capabilities(&self) -> Capabilities;
    fn enumerate_outputs(&self) -> CoreResult<Topology>;
    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>>;
    fn current_state(&self) -> CoreResult<Topology>;
    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult>;
    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult>;
}

pub trait OutputWatcher {
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

    fn current_state(&self) -> CoreResult<Topology> {
        (*self).current_state()
    }

    fn test(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        (*self).test(plan)
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        (*self).apply(plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFailureKind {
    Rejected,
    TopologyChanged,
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct TestResult {
    pub success: bool,
    pub failure: Option<ConfigFailureKind>,
    pub message: Option<String>,
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
    pub(crate) fn new(backend: B) -> Self {
        Self { backend }
    }

    pub(crate) fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    pub(crate) fn test_plan(&self, plan: &LayoutPlan) -> CoreResult<TestResult> {
        if !self.capabilities().can_test {
            return Ok(TestResult {
                success: true,
                failure: None,
                message: Some("Backend does not support test mode".to_string()),
            });
        }
        self.backend.test(plan)
    }

    pub(crate) fn apply_plan(&self, plan: &LayoutPlan, hooks: &Hooks) -> CoreResult<ApplyResult> {
        tracing::info!("Applying plan for {} outputs", plan.outputs.len());

        for hook in &hooks.pre_apply {
            tracing::debug!("Running pre-apply hook: {}", hook.command);
        }

        let pre_results = self.run_hooks(&hooks.pre_apply, "pre-apply");
        if !pre_results.iter().all(|r| r.success) {
            let message = pre_results
                .iter()
                .find(|result| !result.success)
                .map(format_hook_failure)
                .unwrap_or_else(|| "Pre-apply hooks failed".to_string());
            return Ok(ApplyResult {
                success: false,
                failure: Some(ConfigFailureKind::Rejected),
                message: Some(message),
                applied_state: None,
            });
        }

        let result = self.backend.apply(plan)?;

        if result.success {
            let post_results = self.run_hooks(&hooks.post_apply, "post-apply");
            tracing::debug!(
                ran = post_results.len(),
                failed = post_results.iter().filter(|result| !result.success).count(),
                "Post-apply hooks completed"
            );
        } else {
            let failure_results = self.run_hooks(&hooks.on_failure, "failure");
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

    fn run_hooks(&self, hooks: &[Hook], phase: &str) -> Vec<HookResult> {
        let mut results = Vec::new();
        for hook in hooks {
            let result = self.execute_hook(hook, phase);
            results.push(result);
        }
        results
    }

    fn execute_hook(&self, hook: &Hook, phase: &str) -> HookResult {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let timeout = Duration::from_secs(hook.timeout_secs);

        let mut cmd = Command::new(&hook.command);
        cmd.args(&hook.args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

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
                            let timeout_message = match child.kill() {
                                Ok(()) => "Hook timed out".to_string(),
                                Err(err) => {
                                    format!("Hook timed out and could not be killed: {err}")
                                }
                            };
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
            Err(e) => HookResult {
                success: false,
                exit_code: None,
                elapsed_secs: 0.0,
                stdout: String::new(),
                stderr: format!("Failed to spawn: {}", e),
                phase: Some(phase.to_string()),
                command: Some(hook.command.clone()),
            },
        }
    }
}

fn format_hook_failure(result: &HookResult) -> String {
    let phase = result.phase.as_deref().unwrap_or("hook");
    let command = result.command.as_deref().unwrap_or("<unknown>");
    if result.stderr.is_empty() {
        format!("{phase} hook '{command}' failed")
    } else {
        format!("{phase} hook '{command}' failed: {}", result.stderr)
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
    use crate::model::OutputState;
    use crate::profile::{Hook, Hooks};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct TestBackend {
        apply_calls: Arc<Mutex<usize>>,
    }

    impl Backend for TestBackend {
        fn capabilities(&self) -> Capabilities {
            let mut capabilities = Capabilities::named("test");
            capabilities.can_enumerate = true;
            capabilities.can_test = true;
            capabilities.can_apply = true;
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

        fn current_state(&self) -> CoreResult<Topology> {
            Ok(Topology::default())
        }

        fn test(&self, _plan: &LayoutPlan) -> CoreResult<TestResult> {
            let mut result = TestResult::default();
            result.success = true;
            Ok(result)
        }

        fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            *self.apply_calls.lock().unwrap() += 1;
            let mut result = ApplyResult::default();
            result.success = true;
            result.applied_state = Some(Topology {
                outputs: plan.outputs.clone(),
            });
            Ok(result)
        }
    }

    fn test_hooks(log_path: &std::path::Path) -> Hooks {
        let hook = |label: &str| {
            let mut hook = Hook::new("sh");
            hook.args = vec![
                "-c".to_string(),
                format!("printf '%s\\n' {} >> {}", label, log_path.display()),
            ];
            hook.timeout_secs = 5;
            hook
        };

        let mut hooks = Hooks::default();
        hooks.pre_apply = vec![hook("pre")];
        hooks.post_apply = vec![hook("post")];
        hooks.on_failure = vec![hook("failure")];
        hooks
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

        let result = engine.apply_plan(&plan, &hooks).unwrap();

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
        let mut hooks = Hooks::default();
        hooks.pre_apply = vec![Hook::new("definitely-not-a-real-hook-command")];
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let result = engine.apply_plan(&plan, &hooks).unwrap();

        assert!(!result.success);
        assert_eq!(result.failure, Some(ConfigFailureKind::Rejected));
        assert!(result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("pre-apply hook")));
        assert_eq!(*backend.apply_calls.lock().unwrap(), 0);
    }

    #[test]
    fn test_plan_short_circuits_when_backend_cannot_test() {
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
            let mut state = OutputState::new("DP-1");
            state.enabled = true;
            state
        })]));

        let mut no_test_capabilities = Capabilities::named("test");
        no_test_capabilities.can_apply = true;
        let backend = NoTestBackend {
            capabilities: no_test_capabilities,
        };
        let engine = Engine::new(backend);
        let result = engine.test_plan(&plan).unwrap();

        assert!(result.success);
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

        fn current_state(&self) -> CoreResult<Topology> {
            Ok(Topology::default())
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
}
