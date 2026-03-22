use crate::model::{Topology, Capabilities};
use crate::profile::{Profile, Hook};
use crate::matcher::{Matcher, MatchResult};
use crate::planner::{Planner, LayoutPlan};

pub trait Backend {
    fn capabilities(&self) -> Capabilities;
    fn enumerate_outputs(&self) -> anyhow::Result<Topology>;
    fn watch_outputs(&self) -> anyhow::Result<Box<dyn OutputWatcher>>;
    fn current_state(&self) -> anyhow::Result<Topology>;
    fn test(&self, plan: &LayoutPlan) -> anyhow::Result<TestResult>;
    fn apply(&self, plan: &LayoutPlan) -> anyhow::Result<ApplyResult>;
}

pub trait OutputWatcher {
    fn poll_changed(&mut self) -> anyhow::Result<Option<Topology>>;
}

#[derive(Debug)]
pub struct TestResult {
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug)]
pub struct ApplyResult {
    pub success: bool,
    pub message: Option<String>,
    pub applied_state: Option<Topology>,
}

pub struct Engine<B: Backend> {
    backend: B,
}

impl<B: Backend> Engine<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    pub fn current_topology(&self) -> anyhow::Result<Topology> {
        self.backend.current_state()
    }

    pub fn detect_topology(&self) -> anyhow::Result<Topology> {
        self.backend.enumerate_outputs()
    }

    pub fn find_matching_profile(
        &self,
        topology: &Topology,
        profiles: &[Profile],
    ) -> Option<MatchResult> {
        Matcher::match_profile(topology, profiles)
    }

    pub fn test_plan(&self, plan: &LayoutPlan) -> anyhow::Result<TestResult> {
        if !self.capabilities().can_test {
            return Ok(TestResult {
                success: true,
                message: Some("Backend does not support test mode".to_string()),
            });
        }
        self.backend.test(plan)
    }

    pub fn apply_plan(&self, plan: &LayoutPlan, hooks: &[Hook]) -> anyhow::Result<ApplyResult> {
        tracing::info!("Applying plan for {} outputs", plan.outputs.len());

        for hook in hooks {
            tracing::debug!("Running pre-apply hook: {}", hook.command);
        }

        let pre_results = self.run_hooks(hooks, "pre_apply")?;
        if !pre_results.iter().all(|r| r.success) {
            return Ok(ApplyResult {
                success: false,
                message: Some("Pre-apply hooks failed".to_string()),
                applied_state: None,
            });
        }

        let result = self.backend.apply(plan)?;

        if result.success {
            let post_results = self.run_hooks(hooks, "post_apply")?;
            tracing::debug!("Post-apply hook results: {:?}", post_results);
        } else {
            let failure_results = self.run_hooks(hooks, "on_failure")?;
            tracing::debug!("Failure hook results: {:?}", failure_results);
        }

        Ok(result)
    }

    pub fn execute_profile(
        &self,
        profile: &Profile,
        topology: &Topology,
    ) -> anyhow::Result<ApplyResult> {
        let match_result = self.find_matching_profile(topology, std::slice::from_ref(profile))
            .ok_or_else(|| anyhow::anyhow!("Profile does not match current topology"))?;

        let plan = Planner::plan_from_profile(&match_result, topology)?;

        self.apply_plan(&plan, &profile.hooks.pre_apply)?;

        let result = self.backend.apply(&plan)?;

        Ok(result)
    }

    fn run_hooks(&self, hooks: &[Hook], _phase: &str) -> anyhow::Result<Vec<HookResult>> {
        let mut results = Vec::new();
        for hook in hooks {
            let result = self.execute_hook(hook)?;
            results.push(result);
        }
        Ok(results)
    }

    fn execute_hook(&self, hook: &Hook) -> anyhow::Result<HookResult> {
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
            Ok(mut child) => {
                loop {
                    match child.try_wait()? {
                        Some(status) => {
                            let elapsed = start.elapsed();
                            return Ok(HookResult {
                                success: status.success(),
                                exit_code: status.code(),
                                elapsed_secs: elapsed.as_secs_f64(),
                                stdout: String::new(),
                                stderr: String::new(),
                            });
                        }
                        None => {
                            if start.elapsed() > timeout {
                                child.kill()?;
                                return Ok(HookResult {
                                    success: false,
                                    exit_code: None,
                                    elapsed_secs: timeout.as_secs_f64(),
                                    stdout: String::new(),
                                    stderr: String::from("Hook timed out"),
                                });
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                }
            }
            Err(e) => Ok(HookResult {
                success: false,
                exit_code: None,
                elapsed_secs: 0.0,
                stdout: String::new(),
                stderr: format!("Failed to spawn: {}", e),
            }),
        }
    }
}

#[derive(Debug)]
pub struct HookResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub elapsed_secs: f64,
    pub stdout: String,
    pub stderr: String,
}
