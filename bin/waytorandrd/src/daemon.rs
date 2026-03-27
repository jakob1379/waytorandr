use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use waytorandr_core::engine::{Backend, ConfigFailureKind};
use waytorandr_core::error::CoreError;
use waytorandr_core::model::{OutputState, Topology};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::Profile;
use waytorandr_core::runtime;
use waytorandr_core::store::{ProfileStore, StateStore};

const STABLE_SAMPLES: usize = 2;
const STABLE_INTERVAL: Duration = Duration::from_millis(250);
const STABLE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: usize = 5;

enum DaemonOutcome {
    Applied,
    NoMatch,
    TopologyChanged,
}

pub(crate) fn handle_topology_change(
    backend: &impl Backend,
    store: &ProfileStore,
    state_store: &StateStore,
) -> Result<()> {
    for attempt in 0..MAX_RETRIES {
        let topology = wait_for_stable_topology(backend, state_store)?;
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

    tracing::error!("giving up after repeated topology changes during daemon apply");
    Ok(())
}

fn wait_for_stable_topology(backend: &impl Backend, state_store: &StateStore) -> Result<Topology> {
    let deadline = Instant::now() + STABLE_TIMEOUT;
    let mut last_fingerprint = None;
    let mut stable_samples = 0usize;

    loop {
        let topology = runtime::normalized_topology_from_backend(backend, state_store)?;
        let fingerprint = topology.state_fingerprint();

        if last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            stable_samples += 1;
            if stable_samples >= STABLE_SAMPLES {
                return Ok(topology);
            }
        } else {
            last_fingerprint = Some(fingerprint);
            stable_samples = 1;
        }

        if Instant::now() >= deadline {
            return Ok(topology);
        }

        std::thread::sleep(STABLE_INTERVAL);
    }
}

fn maybe_apply_matching_profile(
    backend: &impl Backend,
    store: &ProfileStore,
    state_store: &StateStore,
    topology: &Topology,
) -> Result<DaemonOutcome> {
    let profiles = store.profiles()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let selected = match runtime::select_profile_for_topology(topology, &profiles, &state) {
        Some(selected) => selected,
        None => {
            tracing::info!("no matching profile and no default configured");
            return Ok(DaemonOutcome::NoMatch);
        }
    };

    tracing::info!(profile = %selected.name, "selected profile for current topology");

    apply_profile(backend, state_store, &selected, topology)
}

fn apply_profile(
    backend: &impl Backend,
    state_store: &StateStore,
    profile: &Profile,
    topology: &Topology,
) -> Result<DaemonOutcome> {
    let plan =
        runtime::plan_profile_for_topology(profile, topology).map_err(anyhow::Error::from)?;
    if plan_matches_topology(&plan, topology) {
        persist_applied_profile(state_store, profile, topology)?;
        tracing::info!(profile = %profile.name, "profile already matches current topology");
        return Ok(DaemonOutcome::Applied);
    }

    let mut first_plan = Some((topology.clone(), plan));
    let cycle = runtime::execute_plan_cycle_with_backend(backend, &profile.hooks, false, || {
        if let Some(first) = first_plan.take() {
            return Ok(first);
        }

        runtime::plan_profile_with_backend(backend, state_store, profile)
    })
    .map_err(anyhow::Error::from)?;

    let test = cycle.validation;
    if !test.success {
        if test.failure == Some(ConfigFailureKind::TopologyChanged) {
            return Ok(DaemonOutcome::TopologyChanged);
        }
        bail!(test
            .message
            .unwrap_or_else(|| "backend rejected configuration".to_string()));
    }

    let refreshed_topology = cycle
        .apply_topology
        .ok_or_else(|| anyhow::anyhow!("missing apply topology"))?;
    let result = cycle
        .apply_result
        .ok_or_else(|| anyhow::anyhow!("missing apply result"))?;
    if !result.success {
        if result.failure == Some(ConfigFailureKind::TopologyChanged) {
            return Ok(DaemonOutcome::TopologyChanged);
        }
        bail!(result
            .message
            .unwrap_or_else(|| "backend failed to apply configuration".to_string()));
    }

    let applied = result.applied_state.unwrap_or(refreshed_topology);
    persist_applied_profile(state_store, profile, &applied)?;

    tracing::info!(profile = %profile.name, "applied profile");
    Ok(DaemonOutcome::Applied)
}

fn persist_applied_profile(
    state_store: &StateStore,
    profile: &Profile,
    topology: &Topology,
) -> Result<()> {
    let mut state = state_store.load_state()?.unwrap_or_default();
    runtime::record_applied_profile(&mut state, &profile.name, Some("wlroots"), topology);
    state.daemon_enabled = true;
    state_store.save_state(&state)?;

    Ok(())
}

fn plan_matches_topology(plan: &LayoutPlan, topology: &Topology) -> bool {
    topology
        .outputs
        .iter()
        .filter(|(_, output)| !output.identity.is_ignored && !output.identity.is_virtual)
        .all(|(name, current)| match plan.outputs.get(name) {
            Some(desired) => desired == current,
            None => !current.enabled,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Arc, Mutex, OnceLock};
    use waytorandr_core::engine::{ApplyResult, OutputWatcher, TestResult};
    use waytorandr_core::model::{Capabilities, OutputIdentity, Position};
    use waytorandr_core::profile::{Hooks, OutputConfig, OutputMatcher, Profile, ProfileOptions};

    fn xdg_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_test_state_dir<T>(f: impl FnOnce() -> T) -> T {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let unique = format!(
            "waytorandrd-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let state_home = root.join("state");
        std::fs::create_dir_all(&state_home).unwrap();

        let previous_state = std::env::var_os("XDG_STATE_HOME");
        std::env::set_var("XDG_STATE_HOME", &state_home);

        let result = f();

        restore_env("XDG_STATE_HOME", previous_state);
        let _ = std::fs::remove_dir_all(root);
        result
    }

    fn restore_env(key: &str, value: Option<OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    fn output(connector: &str, enabled: bool) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = enabled;
        state
    }

    fn profile(name: &str, connector: &str, enabled: bool) -> Profile {
        Profile {
            name: name.to_string(),
            priority: 0,
            match_rules: vec![OutputMatcher {
                identity: OutputIdentity::new(connector),
                required: true,
                position_hint: Some(Position::default()),
            }],
            layout: HashMap::from([(
                connector.to_string(),
                OutputConfig {
                    state: output(connector, enabled),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
            options: ProfileOptions::default(),
        }
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
            let mut capabilities = Capabilities::named("stub");
            capabilities.can_enumerate = true;
            capabilities.can_test = true;
            capabilities.can_apply = true;
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

        fn current_state(&self) -> waytorandr_core::error::CoreResult<Topology> {
            Ok(self.topology.clone())
        }

        fn test(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<TestResult> {
            *self.test_calls.lock().unwrap() += 1;
            let mut result = TestResult::default();
            result.success = self.test_success;
            result.failure = self.test_failure;
            result.message = self.test_message.clone();
            Ok(result)
        }

        fn apply(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<ApplyResult> {
            *self.apply_calls.lock().unwrap() += 1;
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
    fn apply_profile_returns_topology_changed_when_backend_rejects_test_due_to_change() {
        with_test_state_dir(|| {
            let state_store = StateStore::new().unwrap();
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

            let outcome = apply_profile(&backend, &state_store, &profile, &topology).unwrap();

            assert!(matches!(outcome, DaemonOutcome::TopologyChanged));
            assert_eq!(*test_calls.lock().unwrap(), 1);
            assert_eq!(*apply_calls.lock().unwrap(), 0);
        });
    }

    #[test]
    fn apply_profile_skips_backend_calls_when_plan_already_matches() {
        with_test_state_dir(|| {
            let state_store = StateStore::new().unwrap();
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

            let outcome = apply_profile(&backend, &state_store, &profile, &topology).unwrap();
            let state = state_store.load_state().unwrap().unwrap();

            assert!(matches!(outcome, DaemonOutcome::Applied));
            assert_eq!(*test_calls.lock().unwrap(), 0);
            assert_eq!(*apply_calls.lock().unwrap(), 0);
            assert_eq!(state.last_profile.as_deref(), Some("desk"));
        });
    }
}
