use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use waytorandr_core::engine::{Backend, ConfigFailureKind, HookPolicy};
use waytorandr_core::matcher::Matcher;
use waytorandr_core::model::{BackendKind, Topology, VirtualPreset};
use waytorandr_core::planner::{topology_is_blank_internal_only, LayoutPlan, Planner};
use waytorandr_core::profile::{Hooks, Profile};
use waytorandr_core::state::StateStore;
use waytorandr_core::store::ProfileStore;
use waytorandr_core::terminal::escape_terminal_text;
use waytorandr_core::workflow;

const STABLE_SAMPLES: usize = 2;
const STABLE_INTERVAL: Duration = Duration::from_millis(250);
const STABLE_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_BACKOFF: Duration = Duration::from_millis(100);
const MAX_RETRIES: usize = 5;

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
    hook_policy: HookPolicy,
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

        match maybe_apply_matching_profile(backend, store, state_store, &topology, hook_policy)? {
            DaemonOutcome::Applied | DaemonOutcome::NoMatch => return Ok(()),
            DaemonOutcome::TopologyChanged => {
                let retry = attempt + 1;
                if retry == 1 {
                    tracing::warn!(
                        attempt = retry,
                        total_attempts = MAX_RETRIES,
                        "topology changed during daemon apply, retrying full pass"
                    );
                } else if retry == MAX_RETRIES {
                    tracing::warn!(
                        attempt = retry,
                        total_attempts = MAX_RETRIES,
                        "topology changed during daemon apply, reached max attempts, giving up"
                    );
                } else {
                    tracing::debug!(
                        attempt = retry,
                        total_attempts = MAX_RETRIES,
                        "topology changed during daemon apply, suppressing duplicate retry warning"
                    );
                }
                if retry < MAX_RETRIES {
                    std::thread::sleep(RETRY_BACKOFF);
                }
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
    hook_policy: HookPolicy,
) -> Result<DaemonOutcome> {
    let setup_fingerprint = topology.setup_fingerprint();
    let settings = store.settings()?;
    if topology_is_blank_internal_only(topology, settings.builtin_output.as_ref()) {
        tracing::warn!(
            fingerprint = %setup_fingerprint,
            "applying immutable built-in fallback for blank internal-only topology"
        );
        return apply_builtin_fallback(
            backend,
            state_store,
            topology,
            settings.builtin_output.as_ref(),
        );
    }

    if !topology.has_strong_setup_identity() {
        remember_current_topology(state_store, backend.capabilities().backend, topology)?;
        tracing::warn!(
            fingerprint = %setup_fingerprint,
            "skipping daemon auto-apply because the current topology only has weak output identity"
        );
        return Ok(DaemonOutcome::NoMatch);
    }

    let state = state_store.load_state()?.unwrap_or_default();
    let stored_profiles = store.list(state_store)?;
    let setup_profiles: Vec<_> = stored_profiles
        .iter()
        .filter(|stored| stored.setup_fingerprint == setup_fingerprint)
        .map(|stored| stored.profile.clone())
        .collect();

    if let Some(default_name) = settings.setup_default_profile(&setup_fingerprint) {
        if let Some(profile) = setup_profiles
            .iter()
            .find(|profile| profile.name == default_name)
        {
            tracing::info!(profile = %escape_terminal_text(&profile.name), "selected explicit default profile for current topology");
            return apply_profile(
                backend,
                state_store,
                profile,
                topology,
                Some(&profile.name),
                hook_policy,
            );
        }
        tracing::warn!(
            profile = %escape_terminal_text(default_name),
            fingerprint = %setup_fingerprint,
            "configured setup default profile was not found for current topology"
        );
    }

    if let Some(matched) = Matcher::match_profile_exact(topology, &setup_profiles) {
        tracing::info!(profile = %escape_terminal_text(&matched.profile.name), "selected matching profile for current topology");
        return apply_profile(
            backend,
            state_store,
            &matched.profile,
            topology,
            Some(&matched.profile.name),
            hook_policy,
        );
    }

    let default_candidates: Vec<_> = stored_profiles
        .iter()
        .filter(|stored| stored.setup_fingerprint != setup_fingerprint)
        .filter(|stored| {
            settings.setup_default_profile(&stored.setup_fingerprint)
                == Some(stored.profile.name.as_str())
        })
        .map(|stored| stored.profile.clone())
        .collect();
    let matched_defaults = Matcher::matching_profiles_exact(topology, &default_candidates);
    if matched_defaults.len() == 1 {
        let matched = &matched_defaults[0];
        tracing::info!(profile = %escape_terminal_text(&matched.profile.name), "selected explicit default profile from another setup fingerprint");
        return apply_profile(
            backend,
            state_store,
            &matched.profile,
            topology,
            Some(&matched.profile.name),
            hook_policy,
        );
    }
    if matched_defaults.len() > 1 {
        tracing::warn!(
            matches = matched_defaults.len(),
            "multiple explicit default profiles from other setup fingerprints matched current topology; skipping cross-setup default fallback"
        );
    }

    if let Some(remembered) = state.remembered_topology_for_setup(&setup_fingerprint) {
        if remembered.has_enabled_real_outputs() {
            tracing::info!(fingerprint = %setup_fingerprint, "using remembered layout for current topology");
            let remembered_profile = workflow::profile_from_topology("__remembered__", remembered);
            return apply_profile(
                backend,
                state_store,
                &remembered_profile,
                topology,
                None,
                hook_policy,
            );
        }

        tracing::warn!(
            fingerprint = %setup_fingerprint,
            "skipping remembered layout because it would leave all real outputs disabled"
        );
        state_store.update_state(|state| {
            state.remembered_setups.remove(&setup_fingerprint);
            Ok(())
        })?;
    }

    remember_current_topology(state_store, backend.capabilities().backend, topology)?;
    tracing::info!(
        fingerprint = %setup_fingerprint,
        "no explicit default or remembered layout for current topology; remembered current setup"
    );

    Ok(DaemonOutcome::NoMatch)
}

fn apply_builtin_fallback(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    topology: &Topology,
    builtin_output: Option<&waytorandr_core::model::OutputIdentity>,
) -> Result<DaemonOutcome> {
    let Some((pre_apply_topology, pre_apply_plan)) =
        plan_builtin_fallback_from_current(backend, builtin_output).map_err(anyhow::Error::from)?
    else {
        tracing::warn!("topology changed before built-in fallback apply; retrying full pass");
        return Ok(DaemonOutcome::TopologyChanged);
    };
    if pre_apply_topology.setup_fingerprint() != topology.setup_fingerprint() {
        tracing::warn!("topology changed before built-in fallback apply; retrying full pass");
        return Ok(DaemonOutcome::TopologyChanged);
    }
    let pre_apply_validation =
        workflow::validate_plan(backend, &pre_apply_plan).map_err(anyhow::Error::from)?;
    if pre_apply_validation.failure == Some(ConfigFailureKind::TopologyChanged) {
        return Ok(DaemonOutcome::TopologyChanged);
    }
    match pre_apply_validation.status {
        waytorandr_core::engine::ValidationStatus::Supported => {}
        waytorandr_core::engine::ValidationStatus::Unsupported
        | waytorandr_core::engine::ValidationStatus::Rejected => {
            bail!(
                "{}",
                pre_apply_validation
                    .message
                    .as_deref()
                    .unwrap_or("backend rejected built-in fallback")
            );
        }
    }

    let hooks = Hooks::default();
    let apply_result = workflow::apply_plan(backend, &hooks, HookPolicy::Disabled, &pre_apply_plan)
        .map_err(anyhow::Error::from)?;
    if apply_result.failure == Some(ConfigFailureKind::TopologyChanged) {
        return Ok(DaemonOutcome::TopologyChanged);
    }
    if !apply_result.success {
        bail!(
            "{}",
            apply_result
                .message
                .as_deref()
                .unwrap_or("backend failed to apply built-in fallback")
        );
    }

    let applied_topology = if let Some(applied_state) = apply_result.applied_state {
        applied_state
    } else {
        workflow::bounded_topology_from_backend(backend).map_err(anyhow::Error::from)?
    };
    if applied_topology.setup_fingerprint() != pre_apply_topology.setup_fingerprint()
        || !applied_topology.has_enabled_real_outputs()
        || !workflow::topology_matches_plan(&applied_topology, &pre_apply_plan)
    {
        return Ok(DaemonOutcome::TopologyChanged);
    }
    persist_runtime_state(
        state_store,
        None,
        backend.capabilities().backend,
        &applied_topology,
    )?;
    tracing::info!(
        topology = %topology_outputs_summary(&applied_topology),
        "applied immutable built-in fallback"
    );
    Ok(DaemonOutcome::Applied)
}

fn plan_builtin_fallback_from_current(
    backend: &(impl Backend + ?Sized),
    builtin_output: Option<&waytorandr_core::model::OutputIdentity>,
) -> waytorandr_core::error::CoreResult<Option<(Topology, LayoutPlan)>> {
    let topology = workflow::bounded_topology_from_backend(backend)?;
    if !topology_is_blank_internal_only(&topology, builtin_output) {
        return Ok(None);
    }

    let plan = Planner::plan_from_preset(VirtualPreset::Builtin, &topology, builtin_output, None)?;
    Ok(Some((topology, plan)))
}

fn apply_profile(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    profile: &Profile,
    topology: &Topology,
    recorded_profile_name: Option<&str>,
    hook_policy: HookPolicy,
) -> Result<DaemonOutcome> {
    let backend_kind = backend.capabilities().backend;
    let plan =
        workflow::plan_profile_for_topology(profile, topology).map_err(anyhow::Error::from)?;
    tracing::info!(
        profile = %escape_terminal_text(&profile.name),
        current_fingerprint = %topology.fingerprint(),
        current_setup = %topology.setup_fingerprint(),
        current_outputs = %topology_outputs_summary(topology),
        planned_outputs = %plan_outputs_summary(&plan),
        "evaluated daemon profile plan"
    );
    if !plan.has_enabled_real_outputs() {
        tracing::warn!(
            profile = %escape_terminal_text(&profile.name),
            current_outputs = %topology_outputs_summary(topology),
            planned_outputs = %plan_outputs_summary(&plan),
            "skipping daemon profile because it would leave all real outputs disabled"
        );
        return Ok(DaemonOutcome::NoMatch);
    }
    if workflow::topology_matches_plan(topology, &plan) {
        persist_runtime_state(state_store, recorded_profile_name, backend_kind, topology)?;
        if let Some(profile_name) = recorded_profile_name {
            tracing::info!(
                profile = %escape_terminal_text(profile_name),
                topology = %topology_outputs_summary(topology),
                "profile already matches current topology"
            );
        } else {
            tracing::info!(
                topology = %topology_outputs_summary(topology),
                "remembered layout already matches current topology"
            );
        }
        return Ok(DaemonOutcome::Applied);
    }

    let latest_topology = workflow::normalized_topology_from_backend(backend, state_store)
        .map_err(anyhow::Error::from)?;
    if !latest_topology.has_strong_setup_identity()
        || latest_topology.setup_fingerprint() != topology.setup_fingerprint()
    {
        tracing::warn!(
            profile = %escape_terminal_text(&profile.name),
            "topology identity changed before daemon apply; retrying full pass"
        );
        return Ok(DaemonOutcome::TopologyChanged);
    }

    let plan = workflow::plan_profile_for_topology(profile, &latest_topology)
        .map_err(anyhow::Error::from)?;

    let validation = workflow::validate_plan(backend, &plan).map_err(anyhow::Error::from)?;
    if validation.failure == Some(ConfigFailureKind::TopologyChanged) {
        tracing::warn!(
            profile = %escape_terminal_text(&profile.name),
            "backend reported topology changed while applying daemon profile"
        );
        return Ok(DaemonOutcome::TopologyChanged);
    }

    match validation.status {
        waytorandr_core::engine::ValidationStatus::Supported => {}
        waytorandr_core::engine::ValidationStatus::Unsupported
        | waytorandr_core::engine::ValidationStatus::Rejected => {
            bail!(
                "{}",
                validation
                    .message
                    .as_deref()
                    .unwrap_or("backend rejected configuration")
            );
        }
    }

    if hook_policy == HookPolicy::Enabled && profile.has_hooks() {
        tracing::warn!(
            profile = %escape_terminal_text(&profile.name),
            "daemon profile contains hooks and will execute commands as the current user"
        );
    }

    let apply_result = workflow::apply_plan(backend, &profile.hooks, hook_policy, &plan)
        .map_err(anyhow::Error::from)?;
    if apply_result.failure == Some(ConfigFailureKind::TopologyChanged) {
        tracing::warn!(
            profile = %escape_terminal_text(&profile.name),
            "backend reported topology changed while applying daemon profile"
        );
        return Ok(DaemonOutcome::TopologyChanged);
    }

    if apply_result.success {
        let applied_topology = apply_result.applied_state.clone().unwrap_or_else(|| {
            // Re-enumerate to get post-apply topology if backend didn't provide it
            workflow::normalized_topology_from_backend(backend, state_store).unwrap_or_else(|e| {
                tracing::error!(
                    error = %e,
                    "failed to enumerate topology after apply"
                );
                latest_topology.clone()
            })
        });
        if applied_topology.validate_limits().is_err() {
            return Ok(DaemonOutcome::TopologyChanged);
        }
        tracing::info!(
            profile = %escape_terminal_text(&profile.name),
            applied_fingerprint = %applied_topology.fingerprint(),
            applied_setup = %applied_topology.setup_fingerprint(),
            applied_outputs = %topology_outputs_summary(&applied_topology),
            "backend reported daemon profile applied"
        );
        if !applied_topology.has_enabled_real_outputs() {
            tracing::warn!(
                profile = %escape_terminal_text(&profile.name),
                current_outputs = %topology_outputs_summary(topology),
                planned_outputs = %plan_outputs_summary(&plan),
                applied_outputs = %topology_outputs_summary(&applied_topology),
                "backend reported a successful apply but the resulting topology has no enabled real outputs; retrying full daemon pass"
            );
            return Ok(DaemonOutcome::TopologyChanged);
        }

        if !workflow::topology_matches_plan(&applied_topology, &plan) {
            tracing::warn!(
                profile = %escape_terminal_text(&profile.name),
                current_outputs = %topology_outputs_summary(topology),
                planned_outputs = %plan_outputs_summary(&plan),
                applied_outputs = %topology_outputs_summary(&applied_topology),
                "backend reported a successful apply but the resulting topology does not match the intended plan; retrying full daemon pass"
            );
            return Ok(DaemonOutcome::TopologyChanged);
        }

        persist_runtime_state(
            state_store,
            recorded_profile_name,
            backend_kind,
            &applied_topology,
        )?;

        if let Some(profile_name) = recorded_profile_name {
            tracing::info!(
                profile = %escape_terminal_text(profile_name),
                topology = %topology_outputs_summary(&applied_topology),
                "applied profile"
            );
        } else {
            tracing::info!(
                topology = %topology_outputs_summary(&applied_topology),
                "applied remembered layout"
            );
        }
        Ok(DaemonOutcome::Applied)
    } else {
        bail!(
            "{}",
            apply_result
                .message
                .as_deref()
                .unwrap_or("backend failed to apply configuration")
        );
    }
}

fn persist_runtime_state(
    state_store: &StateStore,
    profile_name: Option<&str>,
    backend: BackendKind,
    topology: &Topology,
) -> Result<()> {
    workflow::persist_daemon_runtime_state(state_store, profile_name, backend, topology)
        .map_err(Into::into)
}

fn remember_current_topology(
    state_store: &StateStore,
    backend: BackendKind,
    topology: &Topology,
) -> Result<()> {
    if !topology.has_enabled_real_outputs() {
        tracing::warn!(
            fingerprint = %topology.setup_fingerprint(),
            "skipping remembered layout update because current topology has no enabled real outputs"
        );
        workflow::record_daemon_started_in_store(state_store, backend)?;
        return Ok(());
    }

    persist_runtime_state(state_store, None, backend, topology)
}

fn topology_outputs_summary(topology: &Topology) -> String {
    let mut outputs: Vec<String> = topology
        .outputs
        .iter()
        .filter(|(_, output)| !output.identity.is_ignored && !output.identity.is_virtual)
        .map(|(name, output)| {
            let enabled = if output.enabled { "on" } else { "off" };
            let mode = output.mode.map_or_else(
                || "unknown".to_string(),
                |mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh),
            );
            format!(
                "{}:{enabled}:{mode}@{},{}",
                escape_terminal_text(name),
                output.position.x,
                output.position.y
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

fn plan_outputs_summary(plan: &LayoutPlan) -> String {
    let mut outputs: Vec<String> = plan
        .outputs
        .iter()
        .filter(|(_, output)| !output.identity.is_ignored && !output.identity.is_virtual)
        .map(|(name, output)| {
            let enabled = if output.enabled { "on" } else { "off" };
            let mode = output.mode.map_or_else(
                || "unknown".to_string(),
                |mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh),
            );
            format!(
                "{}:{enabled}:{mode}@{},{}",
                escape_terminal_text(name),
                output.position.x,
                output.position.y
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
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::error::Error;
    use std::ffi::OsString;
    use std::sync::{Arc, Mutex, OnceLock};
    use waytorandr_core::engine::{ApplyResult, OutputWatcher, TestResult};
    use waytorandr_core::error::CoreError;
    use waytorandr_core::model::{Capabilities, OutputIdentity, OutputState, Position};
    use waytorandr_core::profile::{OutputMatcher, Profile};

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

    fn xdg_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_test_state_dir<T>(
        f: impl FnOnce() -> Result<T, Box<dyn Error>>,
    ) -> Result<T, Box<dyn Error>> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let unique = format!(
            "waytorandrd-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let state_home = root.join("state");
        let config_home = root.join("config");
        std::fs::create_dir_all(&state_home)?;
        std::fs::create_dir_all(&config_home)?;

        let _state_home = scoped_env_var("XDG_STATE_HOME", &state_home);
        let _config_home = scoped_env_var("XDG_CONFIG_HOME", &config_home);

        let result = f();
        let _ = std::fs::remove_dir_all(root);
        result
    }

    fn output(connector: &str, enabled: bool) -> OutputState {
        let mut state = OutputState::new(connector);
        state.identity.make = Some("Test".to_string());
        state.identity.model = Some(connector.to_string());
        state.enabled = enabled;
        state
    }

    fn weak_output(connector: &str, enabled: bool) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = enabled;
        state
    }

    fn profile(name: &str, connector: &str, enabled: bool) -> Profile {
        Profile::new(
            name,
            0,
            vec![OutputMatcher::new(
                output(connector, enabled).identity,
                true,
                Some(Position::default()),
            )],
            HashMap::from([(connector.to_string(), output(connector, enabled).into())]),
        )
    }

    fn shifted_profile(name: &str, connector: &str, enabled: bool) -> Profile {
        let mut profile = profile(name, connector, enabled);
        profile.layout.get_mut(connector).unwrap().state.position = Position::new(100, 0);
        profile
    }

    fn external_only_profile(
        name: &str,
        internal_identity: OutputIdentity,
        external_identity: OutputIdentity,
    ) -> Profile {
        Profile::new(
            name,
            0,
            vec![
                OutputMatcher::new(internal_identity, false, None),
                OutputMatcher::new(external_identity, true, None),
            ],
            HashMap::from([
                ("eDP-1".to_string(), output("eDP-1", false).into()),
                ("DP-1".to_string(), output("DP-1", true).into()),
            ]),
        )
    }

    #[test]
    fn default_profile_from_different_setup_is_applied_exactly() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([
                    ("eDP-1".to_string(), output("eDP-1", true)),
                    ("DP-1".to_string(), output("DP-1", true)),
                ]),
            };
            let profile = external_only_profile(
                "external-only",
                weak_output("eDP-1", false).identity,
                output("DP-1", true).identity,
            );
            store.save(&profile, &state_store)?;
            let stored_setup = store.list(&state_store)?[0].setup_fingerprint.clone();
            assert_ne!(stored_setup, current.setup_fingerprint());
            store.set_setup_default_profile(&stored_setup, &profile.name)?;
            let apply_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: Arc::new(Mutex::new(0)),
            };

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store.load_state()?.unwrap_or_default();
            let remembered = state
                .remembered_topology_for_setup(&current.setup_fingerprint())
                .ok_or_else(|| std::io::Error::other("remembered topology missing"))?;

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert!(!remembered.outputs["eDP-1"].enabled);
            assert!(remembered.outputs["DP-1"].enabled);
            assert_eq!(state.last_profile.as_deref(), Some("external-only"));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn non_default_profile_from_different_setup_is_not_applied() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([
                    ("eDP-1".to_string(), output("eDP-1", true)),
                    ("DP-1".to_string(), output("DP-1", true)),
                ]),
            };
            let profile = external_only_profile(
                "external-only",
                weak_output("eDP-1", false).identity,
                output("DP-1", true).identity,
            );
            store.save(&profile, &state_store)?;
            assert_ne!(
                store.list(&state_store)?[0].setup_fingerprint,
                current.setup_fingerprint()
            );
            let apply_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: Arc::new(Mutex::new(0)),
            };

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store.load_state()?.unwrap_or_default();

            assert!(matches!(outcome, DaemonOutcome::NoMatch));
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(state.last_profile, None);
            assert!(state
                .remembered_topology_for_setup(&current.setup_fingerprint())
                .is_some_and(Topology::has_enabled_real_outputs));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn ambiguous_default_profiles_from_other_setups_are_not_applied() -> Result<(), Box<dyn Error>>
    {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([
                    ("eDP-1".to_string(), output("eDP-1", true)),
                    ("DP-1".to_string(), output("DP-1", true)),
                ]),
            };
            let profile_a = external_only_profile(
                "external-only-a",
                weak_output("eDP-1", false).identity,
                output("DP-1", true).identity,
            );
            let profile_b = external_only_profile(
                "external-only-b",
                output("eDP-1", true).identity,
                weak_output("DP-1", true).identity,
            );
            store.save(&profile_a, &state_store)?;
            store.save(&profile_b, &state_store)?;
            for stored in store.list(&state_store)? {
                assert_ne!(stored.setup_fingerprint, current.setup_fingerprint());
                store.set_setup_default_profile(&stored.setup_fingerprint, &stored.profile.name)?;
            }
            let apply_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: Arc::new(Mutex::new(0)),
            };

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store.load_state()?.unwrap_or_default();

            assert!(matches!(outcome, DaemonOutcome::NoMatch));
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(state.last_profile, None);
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn blank_internal_only_topology_applies_builtin_fallback() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), weak_output("eDP-1", false))]),
            };
            let applied = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), weak_output("eDP-1", true))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: Some(applied),
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store.load_state()?.unwrap_or_default();

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(state.last_profile, None);
            assert!(state
                .remembered_topology_for_setup(&current.setup_fingerprint())
                .is_some_and(Topology::has_enabled_real_outputs));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn blank_internal_only_fallback_preempts_saved_blank_default() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), weak_output("eDP-1", false))]),
            };
            let applied = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), weak_output("eDP-1", true))]),
            };
            let blank_profile = profile("blank", "eDP-1", false);
            store.save(&blank_profile, &state_store)?;
            store.set_setup_default_profile(&current.setup_fingerprint(), &blank_profile.name)?;
            let apply_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: Some(applied),
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: Arc::new(Mutex::new(0)),
            };

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store.load_state()?.unwrap_or_default();

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(*apply_calls.lock().unwrap(), 1);
            assert_eq!(state.last_profile, None);
            Ok(())
        })?;
        Ok(())
    }

    struct StubBackend {
        enumerated_topology: Topology,
        applied_topology: Option<Topology>,
        test_success: bool,
        test_failure: Option<ConfigFailureKind>,
        test_message: Option<String>,
        apply_calls: Arc<Mutex<usize>>,
        test_calls: Arc<Mutex<usize>>,
    }

    impl Backend for StubBackend {
        fn capabilities(&self) -> Capabilities {
            let mut capabilities = Capabilities::new(BackendKind::Test);
            capabilities.can_test = true;
            capabilities
        }

        fn enumerate_outputs(&self) -> waytorandr_core::error::CoreResult<Topology> {
            if *self
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                > 0
            {
                return Ok(self
                    .applied_topology
                    .clone()
                    .unwrap_or_else(|| self.enumerated_topology.clone()));
            }

            Ok(self.enumerated_topology.clone())
        }

        fn watch_outputs(&self) -> waytorandr_core::error::CoreResult<Box<dyn OutputWatcher>> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used in tests"),
            })
        }

        fn test(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<TestResult> {
            *self
                .test_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            Ok(if self.test_success {
                TestResult::supported(self.test_message.clone())
            } else {
                TestResult::rejected(self.test_failure, self.test_message.clone())
            })
        }

        fn apply(&self, plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<ApplyResult> {
            *self
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            let mut result = ApplyResult::default();
            result.success = true;
            result.message = Some("applied".to_string());
            result.applied_state =
                Some(self.applied_topology.clone().unwrap_or_else(|| Topology {
                    outputs: plan.outputs.clone(),
                }));
            Ok(result)
        }
    }

    #[test]
    fn plan_match_ignores_virtual_outputs() {
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1", true))]));
        let topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), output("DP-1", true)),
                ("HEADLESS-1".to_string(), {
                    let mut state = OutputState::new("HEADLESS-1");
                    state.identity.is_virtual = true;
                    state.enabled = true;
                    state
                }),
            ]),
        };

        assert!(workflow::topology_matches_plan(&topology, &plan));
    }

    #[test]
    fn plan_match_requires_missing_enabled_outputs_to_be_disabled() {
        let plan = LayoutPlan::new(HashMap::new());
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
        };

        assert!(!workflow::topology_matches_plan(&topology, &plan));
    }

    #[test]
    fn plan_match_ignores_mode_inventory_changes() {
        let mut planned = output("DP-1", true);
        planned.available_modes = vec![waytorandr_core::model::Mode::new(1920, 1080, 60)];
        planned.mode = Some(waytorandr_core::model::Mode::new(1920, 1080, 60));

        let mut current = planned.clone();
        current.available_modes = vec![
            waytorandr_core::model::Mode::new(1280, 720, 60),
            waytorandr_core::model::Mode::new(1920, 1080, 60),
        ];

        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), planned)]));
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), current)]),
        };

        assert!(workflow::topology_matches_plan(&topology, &plan));
    }

    #[test]
    fn apply_profile_returns_topology_changed_when_backend_rejects_test_due_to_change(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                enumerated_topology: topology.clone(),
                applied_topology: None,
                test_success: false,
                test_failure: Some(ConfigFailureKind::TopologyChanged),
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };
            let profile = shifted_profile("desk", "DP-1", true);

            let outcome = apply_profile(
                &backend,
                &state_store,
                &profile,
                &topology,
                Some(&profile.name),
                HookPolicy::Enabled,
            )?;

            assert!(matches!(outcome, DaemonOutcome::TopologyChanged));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn wait_for_stable_topology_reports_stable_when_samples_stop_changing(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                enumerated_topology: topology.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: Arc::new(Mutex::new(0)),
                test_calls: Arc::new(Mutex::new(0)),
            };

            let outcome = wait_for_stable_topology_with(
                &backend,
                &state_store,
                Duration::from_millis(1),
                Duration::from_millis(0),
                2,
            )?;

            assert!(matches!(outcome, TopologyStability::Stable(_)));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn wait_for_stable_topology_reports_timeout_without_claiming_stability(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                enumerated_topology: topology.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: Arc::new(Mutex::new(0)),
                test_calls: Arc::new(Mutex::new(0)),
            };

            let outcome = wait_for_stable_topology_with(
                &backend,
                &state_store,
                Duration::from_millis(0),
                Duration::from_millis(0),
                2,
            )?;

            assert!(matches!(outcome, TopologyStability::TimedOut(_)));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn enforce_topology_policy_leaves_blank_topologies_unapplied_without_defaults(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", false))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: topology,
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };

            enforce_topology_policy(&backend, &store, &state_store, HookPolicy::Enabled)?;

            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            let state = state_store
                .load_state()?
                .ok_or_else(|| std::io::Error::other("state should exist"))?;
            assert!(state.daemon_enabled);
            assert_eq!(state.last_profile, None);
            assert!(state.remembered_setups.is_empty());
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn enforce_topology_policy_returns_error_after_repeated_topology_changes(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                enumerated_topology: topology.clone(),
                applied_topology: None,
                test_success: false,
                test_failure: Some(ConfigFailureKind::TopologyChanged),
                test_message: None,
                apply_calls: Arc::new(Mutex::new(0)),
                test_calls: Arc::new(Mutex::new(0)),
            };
            let profile = shifted_profile("desk", "DP-1", true);
            store.save(&profile, &state_store)?;

            store.set_setup_default_profile(&topology.setup_fingerprint(), &profile.name)?;

            let Err(err) =
                enforce_topology_policy(&backend, &store, &state_store, HookPolicy::Enabled)
            else {
                panic!("repeated topology changes should fail");
            };

            assert!(err
                .to_string()
                .contains("giving up after repeated topology changes during daemon apply"));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn apply_profile_retries_when_backend_reports_blank_applied_topology(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let current = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1", false))]),
            };
            let blank_after_apply = current.clone();
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: Some(blank_after_apply),
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };
            let profile = profile("default", "eDP-1", true);

            let outcome = apply_profile(
                &backend,
                &state_store,
                &profile,
                &current,
                Some(&profile.name),
                HookPolicy::Enabled,
            )?;
            let state = state_store.load_state()?.unwrap_or_default();

            assert!(matches!(outcome, DaemonOutcome::TopologyChanged));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(state.last_profile, None);
            assert!(state.remembered_setups.is_empty());
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn apply_profile_skips_backend_calls_when_plan_already_matches() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                enumerated_topology: topology.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };
            let profile = profile("desk", "DP-1", true);

            let outcome = apply_profile(
                &backend,
                &state_store,
                &profile,
                &topology,
                Some(&profile.name),
                HookPolicy::Enabled,
            )?;
            let state = state_store
                .load_state()?
                .ok_or_else(|| std::io::Error::other("state should exist"))?;

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(state.last_profile.as_deref(), Some("desk"));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn remembered_setup_is_applied_without_setting_last_profile() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", false))]),
            };
            let remembered = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: remembered.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };

            let mut state = state_store.load_state()?.unwrap_or_default();
            state
                .remembered_setups
                .insert(current.setup_fingerprint(), remembered.clone());
            state.last_profile = Some("old".to_string());
            state_store.save_state(&state)?;

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store
                .load_state()?
                .ok_or_else(|| std::io::Error::other("state should exist"))?;

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                1
            );
            assert_eq!(state.last_profile, None);
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn matching_saved_profile_is_preferred_over_remembered_layout() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let remembered = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", false))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: current.clone(),
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };
            let profile = profile("desk", "DP-1", true);

            store.save(&profile, &state_store)?;

            let mut state = state_store.load_state()?.unwrap_or_default();
            state
                .remembered_setups
                .insert(current.setup_fingerprint(), remembered);
            state_store.save_state(&state)?;

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store
                .load_state()?
                .ok_or_else(|| std::io::Error::other("state should exist"))?;

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(state.last_profile.as_deref(), Some("desk"));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn unsafe_remembered_layout_is_not_applied() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", false))]),
            };
            let remembered = current.clone();
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                enumerated_topology: Topology {
                    outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
                },
                applied_topology: None,
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };

            let mut state = state_store.load_state()?.unwrap_or_default();
            state
                .remembered_setups
                .insert(current.setup_fingerprint(), remembered);
            state_store.save_state(&state)?;

            let outcome = maybe_apply_matching_profile(
                &backend,
                &store,
                &state_store,
                &current,
                HookPolicy::Enabled,
            )?;
            let state = state_store
                .load_state()?
                .ok_or_else(|| std::io::Error::other("state should exist"))?;

            assert!(matches!(outcome, DaemonOutcome::NoMatch));
            assert_eq!(
                *test_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert_eq!(
                *apply_calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                0
            );
            assert!(!state
                .remembered_setups
                .contains_key(&current.setup_fingerprint()));
            Ok(())
        })?;
        Ok(())
    }
}
