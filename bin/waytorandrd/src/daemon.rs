use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use waytorandr_core::workflow;
use waytorandr_core::LayoutPlan;
use waytorandr_core::ProfileStore;
use waytorandr_core::{Backend, ConfigFailureKind};
use waytorandr_core::{BackendKind, OutputState, Topology};
use waytorandr_core::{Hooks, Profile};
use waytorandr_core::{ProfileQueryContext, StateStore};

mod persistence;
mod watch;

pub(crate) use persistence::record_daemon_started;
use persistence::{
    daemon_apply_failure_context, record_daemon_apply_outcome, remember_current_topology,
    DaemonApplyFailureContext,
};
pub(crate) use watch::run_watch_loop;

const STABLE_SAMPLES: usize = 2;
const STABLE_INTERVAL: Duration = Duration::from_millis(250);
const STABLE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: usize = 5;

#[cfg(test)]
fn xdg_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum DaemonOutcome {
    Applied,
    NoMatch,
    TopologyChanged,
}

enum TopologyStability {
    Stable(Topology),
    TimedOut(Topology),
}

pub(crate) fn enforce_topology_policy(
    backend: &(impl Backend + ?Sized),
    store: &ProfileStore,
    state_store: &StateStore,
    no_hooks: bool,
) -> Result<()> {
    for attempt in 0..MAX_RETRIES {
        let topology = match wait_for_stable_topology(backend, state_store)? {
            TopologyStability::Stable(topology) => topology,
            TopologyStability::TimedOut(topology) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    total_attempts = MAX_RETRIES,
                    "topology did not stabilize before timeout, proceeding with latest sample"
                );
                topology
            }
        };

        match maybe_apply_matching_profile(backend, store, state_store, &topology, no_hooks)? {
            DaemonOutcome::Applied | DaemonOutcome::NoMatch => return Ok(()),
            DaemonOutcome::TopologyChanged => {
                tracing::warn!(
                    attempt = attempt + 1,
                    total_attempts = MAX_RETRIES,
                    "topology changed during daemon apply, retrying full pass"
                );
            }
        }
    }

    bail!("giving up after repeated topology changes during daemon apply");
}

fn wait_for_stable_topology(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
) -> Result<TopologyStability> {
    wait_for_stable_topology_with(
        backend,
        state_store,
        STABLE_TIMEOUT,
        STABLE_INTERVAL,
        STABLE_SAMPLES,
    )
}

fn wait_for_stable_topology_with(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    timeout: Duration,
    interval: Duration,
    stable_samples_required: usize,
) -> Result<TopologyStability> {
    let deadline = Instant::now() + timeout;
    let mut last_fingerprint = None;
    let mut stable_samples = 0usize;

    loop {
        let topology = workflow::normalized_topology_from_backend(backend, state_store)?;
        let fingerprint = topology.state_fingerprint();

        if last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            stable_samples += 1;
            if stable_samples >= stable_samples_required {
                return Ok(TopologyStability::Stable(topology));
            }
        } else {
            last_fingerprint = Some(fingerprint);
            stable_samples = 1;
        }

        if Instant::now() >= deadline {
            return Ok(TopologyStability::TimedOut(topology));
        }

        std::thread::sleep(interval);
    }
}

fn maybe_apply_matching_profile(
    backend: &(impl Backend + ?Sized),
    store: &ProfileStore,
    state_store: &StateStore,
    topology: &Topology,
    no_hooks: bool,
) -> Result<DaemonOutcome> {
    let setup_fingerprint = topology.setup_fingerprint();
    let state = state_store.load_state()?.unwrap_or_default();
    let query_context = ProfileQueryContext::from_state(&state);
    let settings = store.settings()?;
    let setup_profiles = store.profiles_for_setup(&setup_fingerprint, &query_context)?;

    match workflow::select_profile_application_target(topology, &setup_profiles, &settings, &state)
    {
        workflow::ProfileSelectionDecision::SetupDefault(profile) => {
            tracing::info!(profile = %profile.name, "selected explicit default profile for current topology");
            apply_profile(
                backend,
                state_store,
                &profile,
                topology,
                Some(&profile.name),
                no_hooks,
            )
        }
        workflow::ProfileSelectionDecision::ExactMatch(profile) => {
            tracing::info!(profile = %profile.name, "selected matching profile for current topology");
            apply_profile(
                backend,
                state_store,
                &profile,
                topology,
                Some(&profile.name),
                no_hooks,
            )
        }
        workflow::ProfileSelectionDecision::RememberedLayout(remembered) => {
            tracing::info!(fingerprint = %setup_fingerprint, "using remembered layout for current topology");
            let remembered_profile = workflow::profile_from_topology("__remembered__", &remembered);
            apply_profile(
                backend,
                state_store,
                &remembered_profile,
                topology,
                None,
                no_hooks,
            )
        }
        workflow::ProfileSelectionDecision::NoMatch => {
            if settings.setup_default_profile(&setup_fingerprint).is_some() {
                tracing::warn!(
                    fingerprint = %setup_fingerprint,
                    "configured setup default profile was not found for current topology"
                );
            }

            if state
                .remembered_topology_for_setup(&setup_fingerprint)
                .is_some()
            {
                tracing::warn!(
                    fingerprint = %setup_fingerprint,
                    "skipping remembered layout because it would leave all real outputs disabled"
                );
            }

            let outcome =
                remember_current_topology(state_store, backend.capabilities().backend, topology)?;
            tracing::info!(
                fingerprint = %setup_fingerprint,
                "no explicit default or remembered layout for current topology; remembered current setup"
            );

            Ok(outcome)
        }
    }
}

fn apply_profile(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    profile: &Profile,
    topology: &Topology,
    recorded_profile_name: Option<&str>,
    no_hooks: bool,
) -> Result<DaemonOutcome> {
    let backend_kind = backend.capabilities().backend;
    let prepared =
        workflow::prepare_profile_application(profile, topology).map_err(anyhow::Error::from)?;
    let plan_matches = plan_matches_topology(prepared.plan(), topology);
    let planned_outputs = plan_outputs_summary(prepared.plan());
    let daemon_apply = DaemonApplyWorkflow::new(
        state_store,
        profile,
        recorded_profile_name,
        backend_kind,
        topology,
        planned_outputs,
    );
    tracing::info!(
        profile = %profile.name,
        current_fingerprint = %topology.fingerprint(),
        current_setup = %topology.setup_fingerprint(),
        current_outputs = %topology_outputs_summary(topology),
        planned_outputs = %daemon_apply.planned_outputs,
        "evaluated daemon profile plan"
    );
    if plan_matches {
        return daemon_apply.finish_already_matching();
    }

    let empty_hooks;
    let hooks = if no_hooks {
        empty_hooks = Hooks::default();
        &empty_hooks
    } else {
        &profile.hooks
    };
    let execution = workflow::apply_prepared_profile_workflow(backend, hooks, prepared)
        .map_err(anyhow::Error::from)?;

    daemon_apply.finish_execution(execution)
}

struct DaemonApplyWorkflow<'a> {
    state_store: &'a StateStore,
    profile: &'a Profile,
    recorded_profile_name: Option<&'a str>,
    backend_kind: BackendKind,
    current_topology: &'a Topology,
    planned_outputs: String,
}

impl<'a> DaemonApplyWorkflow<'a> {
    fn new(
        state_store: &'a StateStore,
        profile: &'a Profile,
        recorded_profile_name: Option<&'a str>,
        backend_kind: BackendKind,
        current_topology: &'a Topology,
        planned_outputs: String,
    ) -> Self {
        Self {
            state_store,
            profile,
            recorded_profile_name,
            backend_kind,
            current_topology,
            planned_outputs,
        }
    }

    fn finish_already_matching(&self) -> Result<DaemonOutcome> {
        self.record_success(self.current_topology, true)
    }

    fn finish_execution(&self, execution: workflow::ApplyExecution) -> Result<DaemonOutcome> {
        if execution.failure_kind() == Some(ConfigFailureKind::TopologyChanged) {
            return Ok(self.retry_after_topology_change());
        }

        match execution {
            workflow::ApplyExecution::Applied {
                applied_topology, ..
            } => self.finish_applied_topology(&applied_topology),
            workflow::ApplyExecution::ApplyFailed { apply_result, .. } => self.fail_apply(
                "apply failed",
                apply_result.failure(),
                apply_result.message(),
                "backend failed to apply configuration",
            ),
            workflow::ApplyExecution::Rejected { validation, .. } => self.fail_apply(
                "rejected",
                validation.failure(),
                validation.message.as_deref(),
                "backend rejected configuration",
            ),
            workflow::ApplyExecution::Unsupported { validation, .. } => self.fail_apply(
                "unsupported",
                validation.failure(),
                validation.message.as_deref(),
                "backend validation is unsupported",
            ),
        }
    }

    fn retry_after_topology_change(&self) -> DaemonOutcome {
        tracing::warn!(
            profile = %self.profile.name,
            "backend reported topology changed while applying daemon profile"
        );
        DaemonOutcome::TopologyChanged
    }

    fn finish_applied_topology(&self, applied_topology: &Topology) -> Result<DaemonOutcome> {
        tracing::info!(
            profile = %self.profile.name,
            applied_fingerprint = %applied_topology.fingerprint(),
            applied_setup = %applied_topology.setup_fingerprint(),
            applied_outputs = %topology_outputs_summary(applied_topology),
            "backend reported daemon profile applied"
        );
        if !applied_topology.has_enabled_real_outputs() {
            tracing::warn!(
                profile = %self.profile.name,
                current_outputs = %topology_outputs_summary(self.current_topology),
                planned_outputs = %self.planned_outputs,
                applied_outputs = %topology_outputs_summary(applied_topology),
                "backend reported a successful apply but the resulting topology has no enabled real outputs; retrying full daemon pass"
            );
            return Ok(DaemonOutcome::TopologyChanged);
        }

        self.record_success(applied_topology, false)
    }

    fn record_success(&self, topology: &Topology, already_matching: bool) -> Result<DaemonOutcome> {
        record_daemon_apply_outcome(
            self.state_store,
            self.recorded_profile_name,
            self.backend_kind,
            topology,
            already_matching,
        )?;
        Ok(DaemonOutcome::Applied)
    }

    fn fail_apply(
        &self,
        outcome: &str,
        failure_kind: Option<ConfigFailureKind>,
        failure_message: Option<&str>,
        fallback_message: &str,
    ) -> Result<DaemonOutcome> {
        bail!(
            "{}",
            daemon_apply_failure_context(&DaemonApplyFailureContext {
                profile: self.profile,
                backend_kind: self.backend_kind,
                topology: self.current_topology,
                planned_outputs: &self.planned_outputs,
                outcome,
                failure_kind,
                failure_message,
                fallback_message,
            })
        )
    }
}

fn plan_matches_topology(plan: &LayoutPlan, topology: &Topology) -> bool {
    topology
        .outputs
        .iter()
        .filter(|(_, output)| !output.identity.is_ignored && !output.identity.is_virtual)
        .all(|(name, current)| match plan.outputs.get(name) {
            Some(desired) => desired.same_layout_as(current),
            None => !current.enabled,
        })
}

fn topology_outputs_summary(topology: &Topology) -> String {
    outputs_summary(&topology.outputs)
}

fn plan_outputs_summary(plan: &LayoutPlan) -> String {
    outputs_summary(&plan.outputs)
}

fn outputs_summary(outputs: &HashMap<String, OutputState>) -> String {
    let mut outputs: Vec<String> = outputs
        .iter()
        .filter(|(_, output)| !output.identity.is_ignored && !output.identity.is_virtual)
        .map(|(name, output)| {
            let enabled = if output.enabled { "on" } else { "off" };
            let mode = output.mode.map_or_else(
                || "unknown".to_string(),
                |mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh),
            );
            format!(
                "{name}:{enabled}:{mode}@{},{}",
                output.position.x, output.position.y
            )
        })
        .collect();
    outputs.sort();
    if outputs.is_empty() {
        "<none>".to_string()
    } else {
        outputs.join(",")
    }
}

#[cfg(test)]
mod tests;
