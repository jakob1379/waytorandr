use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use waytorandr_core::engine::{Backend, ConfigFailureKind};
use waytorandr_core::matcher::Matcher;
use waytorandr_core::model::{BackendKind, Topology, VirtualPreset};
use waytorandr_core::planner::{LayoutPlan, Planner};
use waytorandr_core::profile::Profile;
use waytorandr_core::state::StateStore;
use waytorandr_core::store::{DefaultTarget, ProfileStore};
use waytorandr_core::workflow;

const STABLE_SAMPLES: usize = 2;
const STABLE_INTERVAL: Duration = Duration::from_millis(250);
const STABLE_TIMEOUT: Duration = Duration::from_secs(3);
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
        match maybe_apply_matching_profile(backend, store, state_store, &topology)? {
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
) -> Result<DaemonOutcome> {
    let setup_fingerprint = topology.setup_fingerprint();
    let state = state_store.load_state()?.unwrap_or_default();
    let settings = store.settings()?;
    let all_profiles = store.profiles(state_store)?;
    let setup_profiles = store.profiles_for_setup(&setup_fingerprint, state_store)?;

    if let Some(default_name) = settings.setup_default_profile(&setup_fingerprint) {
        if let Some(profile) = setup_profiles
            .iter()
            .find(|profile| profile.name == default_name)
        {
            tracing::info!(profile = %profile.name, "selected explicit default profile for current topology");
            return apply_profile(backend, state_store, profile, topology, Some(&profile.name));
        }
        tracing::warn!(
            profile = %default_name,
            fingerprint = %setup_fingerprint,
            "configured setup default profile was not found for current topology"
        );
    }

    if let Some(DefaultTarget::Profile { .. }) = settings.new_setup_default.as_ref() {
        if let Some(target) = settings.new_setup_default.as_ref().and_then(|target| {
            workflow::resolve_default_target_for_topology(topology, &all_profiles, target)
        }) {
            tracing::info!(
                fingerprint = %setup_fingerprint,
                "using configured global default profile for current topology"
            );
            if let workflow::SelectedTarget::Profile(profile) = target {
                return apply_profile(
                    backend,
                    state_store,
                    &profile,
                    topology,
                    Some(&profile.name),
                );
            }
        }
    }

    if let Some(matched) = Matcher::match_profile(topology, &setup_profiles) {
        tracing::info!(profile = %matched.profile.name, "selected matching profile for current topology");
        return apply_profile(
            backend,
            state_store,
            &matched.profile,
            topology,
            Some(&matched.profile.name),
        );
    }

    if let Some(default_target @ DefaultTarget::Virtual { .. }) =
        settings.new_setup_default.as_ref()
    {
        if let Some(target) =
            workflow::resolve_default_target_for_topology(topology, &all_profiles, default_target)
        {
            tracing::info!(fingerprint = %setup_fingerprint, "using configured default for new setups");
            let outcome = match target {
                workflow::SelectedTarget::Profile(profile) => apply_profile(
                    backend,
                    state_store,
                    &profile,
                    topology,
                    Some(&profile.name),
                ),
                workflow::SelectedTarget::Virtual(preset) => {
                    apply_preset(backend, state_store, preset, topology)
                }
            };

            return match outcome {
                Ok(result) => Ok(result),
                Err(err) => {
                    tracing::warn!(
                        fingerprint = %setup_fingerprint,
                        error = %err,
                        "configured default for new setups could not be applied; remembering current topology"
                    );
                    remember_current_topology(
                        state_store,
                        backend.capabilities().backend,
                        topology,
                    )?;
                    Ok(DaemonOutcome::NoMatch)
                }
            };
        }

        tracing::warn!(
            fingerprint = %setup_fingerprint,
            "configured default for new setups did not resolve for current topology"
        );
    }

    if let Some(remembered) = state.remembered_topology_for_setup(&setup_fingerprint) {
        tracing::info!(fingerprint = %setup_fingerprint, "using remembered layout for current topology");
        let remembered_profile = workflow::profile_from_topology("__remembered__", remembered);
        return apply_profile(backend, state_store, &remembered_profile, topology, None);
    }

    remember_current_topology(state_store, backend.capabilities().backend, topology)?;
    tracing::info!(
        fingerprint = %setup_fingerprint,
        "no explicit default or remembered layout for current topology; remembered current setup"
    );

    Ok(DaemonOutcome::NoMatch)
}

fn apply_preset(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    preset: VirtualPreset,
    topology: &Topology,
) -> Result<DaemonOutcome> {
    let backend_kind = backend.capabilities().backend;
    if let Some(message) = backend
        .capabilities()
        .virtual_preset_unavailable_message(preset)
    {
        bail!(message);
    }

    let plan = Planner::plan_from_preset(preset, topology, None).map_err(anyhow::Error::from)?;
    if plan_matches_topology(&plan, topology) {
        persist_runtime_state(state_store, None, backend_kind, topology)?;
        tracing::info!(preset = %preset, "virtual preset already matches current topology");
        return Ok(DaemonOutcome::Applied);
    }

    let execution = workflow::apply_preset_workflow(backend, state_store, preset)
        .map_err(anyhow::Error::from)?;

    if execution.failure_kind() == Some(ConfigFailureKind::TopologyChanged) {
        return Ok(DaemonOutcome::TopologyChanged);
    }

    match execution {
        workflow::ApplyExecution::Applied {
            applied_topology, ..
        } => {
            persist_runtime_state(state_store, None, backend_kind, &applied_topology)?;
            tracing::info!(preset = %preset, "applied virtual preset");
            Ok(DaemonOutcome::Applied)
        }
        workflow::ApplyExecution::ApplyFailed { .. } => {
            bail!(
                "{}",
                execution
                    .failure_message()
                    .unwrap_or("backend failed to apply configuration")
            );
        }
        workflow::ApplyExecution::Unsupported { .. }
        | workflow::ApplyExecution::Rejected { .. } => {
            bail!(
                "{}",
                execution
                    .failure_message()
                    .unwrap_or("backend rejected configuration")
            );
        }
    }
}

fn apply_profile(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    profile: &Profile,
    topology: &Topology,
    recorded_profile_name: Option<&str>,
) -> Result<DaemonOutcome> {
    let backend_kind = backend.capabilities().backend;
    let plan =
        workflow::plan_profile_for_topology(profile, topology).map_err(anyhow::Error::from)?;
    if plan_matches_topology(&plan, topology) {
        persist_runtime_state(state_store, recorded_profile_name, backend_kind, topology)?;
        if let Some(profile_name) = recorded_profile_name {
            tracing::info!(profile = %profile_name, "profile already matches current topology");
        } else {
            tracing::info!("remembered layout already matches current topology");
        }
        return Ok(DaemonOutcome::Applied);
    }

    let execution = workflow::apply_profile_workflow(backend, state_store, profile)
        .map_err(anyhow::Error::from)?;

    if execution.failure_kind() == Some(ConfigFailureKind::TopologyChanged) {
        return Ok(DaemonOutcome::TopologyChanged);
    }

    match execution {
        workflow::ApplyExecution::Applied {
            applied_topology, ..
        } => {
            persist_runtime_state(
                state_store,
                recorded_profile_name,
                backend_kind,
                &applied_topology,
            )?;

            if let Some(profile_name) = recorded_profile_name {
                tracing::info!(profile = %profile_name, "applied profile");
            } else {
                tracing::info!("applied remembered layout");
            }
            Ok(DaemonOutcome::Applied)
        }
        workflow::ApplyExecution::ApplyFailed { .. } => {
            bail!(
                "{}",
                execution
                    .failure_message()
                    .unwrap_or("backend failed to apply configuration")
            );
        }
        workflow::ApplyExecution::Unsupported { .. }
        | workflow::ApplyExecution::Rejected { .. } => {
            bail!(
                "{}",
                execution
                    .failure_message()
                    .unwrap_or("backend rejected configuration")
            );
        }
    }
}

fn persist_runtime_state(
    state_store: &StateStore,
    profile_name: Option<&str>,
    backend: BackendKind,
    topology: &Topology,
) -> Result<()> {
    if let Some(profile_name) = profile_name {
        workflow::persist_applied_runtime_state(
            state_store,
            profile_name,
            Some(backend),
            topology,
        )?;
    } else {
        workflow::persist_observed_runtime_state(state_store, Some(backend), topology)?;
    }
    workflow::record_daemon_started_in_store(state_store, backend)?;

    Ok(())
}

fn remember_current_topology(
    state_store: &StateStore,
    backend: BackendKind,
    topology: &Topology,
) -> Result<()> {
    persist_runtime_state(state_store, None, backend, topology)
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
        state.enabled = enabled;
        state
    }

    fn profile(name: &str, connector: &str, enabled: bool) -> Profile {
        Profile::new(
            name,
            0,
            vec![OutputMatcher::new(
                OutputIdentity::new(connector),
                true,
                Some(Position::default()),
            )],
            HashMap::from([(connector.to_string(), output(connector, enabled).into())]),
        )
    }

    struct StubBackend {
        topology: Topology,
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
            Ok(self.topology.clone())
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

        fn apply(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<ApplyResult> {
            *self
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            let mut result = ApplyResult::default();
            result.success = true;
            result.message = Some("applied".to_string());
            result.applied_state = Some(self.topology.clone());
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

        assert!(plan_matches_topology(&plan, &topology));
    }

    #[test]
    fn plan_match_requires_missing_enabled_outputs_to_be_disabled() {
        let plan = LayoutPlan::new(HashMap::new());
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
        };

        assert!(!plan_matches_topology(&plan, &topology));
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

        assert!(plan_matches_topology(&plan, &topology));
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
                topology: topology.clone(),
                test_success: false,
                test_failure: Some(ConfigFailureKind::TopologyChanged),
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };
            let profile = profile("desk", "DP-1", false);

            let outcome = apply_profile(
                &backend,
                &state_store,
                &profile,
                &topology,
                Some(&profile.name),
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
                topology: topology.clone(),
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
                topology: topology.clone(),
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
    fn enforce_topology_policy_returns_error_after_repeated_topology_changes(
    ) -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                topology: topology.clone(),
                test_success: false,
                test_failure: Some(ConfigFailureKind::TopologyChanged),
                test_message: None,
                apply_calls: Arc::new(Mutex::new(0)),
                test_calls: Arc::new(Mutex::new(0)),
            };
            let profile = profile("desk", "DP-1", false);
            store.save(&profile, &state_store)?;

            store.set_setup_default_profile(&topology.setup_fingerprint(), &profile.name)?;

            let Err(err) = enforce_topology_policy(&backend, &store, &state_store) else {
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
    fn apply_profile_skips_backend_calls_when_plan_already_matches() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let backend = StubBackend {
                topology: topology.clone(),
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
    fn new_setup_uses_configured_virtual_default() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1", true))]),
            };
            let backend = StubBackend {
                topology: topology.clone(),
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: Arc::new(Mutex::new(0)),
                test_calls: Arc::new(Mutex::new(0)),
            };

            store.set_new_setup_default(waytorandr_core::store::DefaultTarget::Virtual {
                preset: VirtualPreset::Vertical,
            })?;

            let outcome = maybe_apply_matching_profile(&backend, &store, &state_store, &topology)?;
            let state = state_store
                .load_state()?
                .ok_or_else(|| std::io::Error::other("state should exist"))?;

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(state.last_profile, None);
            assert_eq!(
                state
                    .remembered_setups
                    .get(&topology.setup_fingerprint())
                    .map(Topology::fingerprint),
                Some(topology.fingerprint())
            );
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn builtin_default_applies_on_internal_only_setups() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1", false))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                topology: Topology {
                    outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1", true))]),
                },
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };

            store.set_new_setup_default(waytorandr_core::store::DefaultTarget::Virtual {
                preset: VirtualPreset::Builtin,
            })?;

            let outcome = maybe_apply_matching_profile(&backend, &store, &state_store, &topology)?;
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
    fn builtin_default_is_skipped_when_no_internal_output_exists() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let topology = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                topology: topology.clone(),
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };

            store.set_new_setup_default(waytorandr_core::store::DefaultTarget::Virtual {
                preset: VirtualPreset::Builtin,
            })?;

            let outcome = maybe_apply_matching_profile(&backend, &store, &state_store, &topology)?;
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
            assert_eq!(
                state
                    .remembered_setups
                    .get(&topology.setup_fingerprint())
                    .map(Topology::fingerprint),
                Some(topology.fingerprint())
            );
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
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", true))]),
            };
            let remembered = Topology {
                outputs: HashMap::from([("DP-1".to_string(), output("DP-1", false))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                topology: remembered.clone(),
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

            let outcome = maybe_apply_matching_profile(&backend, &store, &state_store, &current)?;
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
    fn global_default_profile_beats_remembered_layout() -> Result<(), Box<dyn Error>> {
        with_test_state_dir(|| {
            let state_store = StateStore::bootstrap()?;
            let store = ProfileStore::bootstrap()?;
            let current = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1", false))]),
            };
            let desired = Topology {
                outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1", true))]),
            };
            let apply_calls = Arc::new(Mutex::new(0));
            let test_calls = Arc::new(Mutex::new(0));
            let backend = StubBackend {
                topology: desired.clone(),
                test_success: true,
                test_failure: None,
                test_message: None,
                apply_calls: apply_calls.clone(),
                test_calls: test_calls.clone(),
            };
            let profile = profile("laptop", "eDP-1", true);

            store.save(&profile, &state_store)?;
            store.set_new_setup_default(waytorandr_core::store::DefaultTarget::Profile {
                name: profile.name.clone(),
            })?;

            let mut state = state_store.load_state()?.unwrap_or_default();
            state
                .remembered_setups
                .insert(current.setup_fingerprint(), current.clone());
            state_store.save_state(&state)?;

            let outcome = maybe_apply_matching_profile(&backend, &store, &state_store, &current)?;
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
            assert_eq!(state.last_profile.as_deref(), Some("laptop"));
            Ok(())
        })?;
        Ok(())
    }
}
