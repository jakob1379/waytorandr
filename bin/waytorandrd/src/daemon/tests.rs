use super::*;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use waytorandr_core::{ApplyResult, OutputWatcher, ValidationResult};
use waytorandr_core::{Capabilities, OutputIdentity, OutputState, Position};
use waytorandr_core::{CoreError, CoreResult};
use waytorandr_core::{OutputMatcher, Profile};

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

fn with_test_state_dir<T>(
    f: impl FnOnce() -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    let _guard = super::xdg_test_guard();
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

fn daemon_output_state(connector: &str, enabled: bool) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = enabled;
    state
}

fn daemon_profile(name: &str, connector: &str, enabled: bool) -> Profile {
    Profile::new(
        name,
        0,
        vec![OutputMatcher::new(
            OutputIdentity::new(connector),
            true,
            Some(Position::default()),
        )],
        HashMap::from([(
            connector.to_string(),
            daemon_output_state(connector, enabled).into(),
        )]),
    )
}

struct StubBackend {
    enumerated_topology: Topology,
    applied_topology: Option<Topology>,
    test_success: bool,
    validation_failure: Option<ConfigFailureKind>,
    validation_message: Option<String>,
    apply_calls: Arc<Mutex<usize>>,
    test_calls: Arc<Mutex<usize>>,
}

impl Backend for StubBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Test);
        capabilities.can_validate = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        Ok(self.enumerated_topology.clone())
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        *self
            .test_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Ok(if self.test_success {
            ValidationResult::supported(self.validation_message.clone())
        } else {
            ValidationResult::rejected(self.validation_failure, self.validation_message.clone())
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        *self
            .apply_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        let applied_topology = self
            .applied_topology
            .clone()
            .unwrap_or_else(|| self.enumerated_topology.clone());
        Ok(ApplyResult::applied(
            Some("applied".to_string()),
            Some(applied_topology),
        ))
    }
}

struct PreparedApplyBackend {
    enumerated_topology: Topology,
    applied_topology: Topology,
    enumerate_calls: Arc<Mutex<usize>>,
    apply_calls: Arc<Mutex<usize>>,
    test_calls: Arc<Mutex<usize>>,
}

impl Backend for PreparedApplyBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Test);
        capabilities.can_validate = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        *self
            .enumerate_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Ok(self.enumerated_topology.clone())
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        *self
            .test_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Ok(ValidationResult::supported(None))
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        *self
            .apply_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Ok(ApplyResult::applied(
            None,
            Some(self.applied_topology.clone()),
        ))
    }
}

#[test]
fn plan_match_ignores_virtual_outputs() {
    let plan = LayoutPlan::new(HashMap::from([(
        "DP-1".to_string(),
        daemon_output_state("DP-1", true),
    )]));
    let topology = Topology {
        outputs: HashMap::from([
            ("DP-1".to_string(), daemon_output_state("DP-1", true)),
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
        outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
    };

    assert!(!plan_matches_topology(&plan, &topology));
}

#[test]
fn plan_match_ignores_mode_inventory_changes() {
    let mut planned = daemon_output_state("DP-1", true);
    planned.available_modes = vec![waytorandr_core::Mode::new(1920, 1080, 60)];
    planned.mode = Some(waytorandr_core::Mode::new(1920, 1080, 60));

    let mut current = planned.clone();
    current.available_modes = vec![
        waytorandr_core::Mode::new(1280, 720, 60),
        waytorandr_core::Mode::new(1920, 1080, 60),
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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let backend = StubBackend {
            enumerated_topology: topology.clone(),
            applied_topology: None,
            test_success: false,
            validation_failure: Some(ConfigFailureKind::TopologyChanged),
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };
        let profile = daemon_profile("desk", "DP-1", false);

        let outcome = apply_profile(
            &backend,
            &state_store,
            &profile,
            &topology,
            Some(&profile.name),
            false,
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
fn daemon_apply_failed_error_includes_profile_backend_and_output_context(
) -> Result<(), Box<dyn Error>> {
    with_test_state_dir(|| {
        let state_store = StateStore::bootstrap()?;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", false))]),
        };
        let plan = LayoutPlan::new(HashMap::from([(
            "DP-1".to_string(),
            daemon_output_state("DP-1", true),
        )]));
        let profile = daemon_profile("desk", "DP-1", true);
        let execution = workflow::ApplyExecution::ApplyFailed {
            plan,
            validation: ValidationResult::supported(None),
            apply_result: ApplyResult::failed(
                Some(ConfigFailureKind::Rejected),
                Some("backend said no".to_string()),
            ),
        };

        let daemon_apply = DaemonApplyWorkflow::new(
            &state_store,
            &profile,
            Some(&profile.name),
            BackendKind::Test,
            &topology,
            "DP-1:on:unknown@0,0".to_string(),
        );

        let Err(err) = daemon_apply.finish_execution(execution) else {
            return Err(
                std::io::Error::other("apply failure should include daemon context").into(),
            );
        };
        let message = err.to_string();

        assert!(message.contains("profile 'desk'"));
        assert!(message.contains("test backend"));
        assert!(message.contains("apply failed"));
        assert!(message.contains("failure_kind=rejected"));
        assert!(message.contains("backend said no"));
        assert!(message.contains("current_outputs=DP-1:off:unknown@0,0"));
        assert!(message.contains("planned_outputs=DP-1:on:unknown@0,0"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn daemon_rejected_error_includes_profile_backend_and_output_context() -> Result<(), Box<dyn Error>>
{
    with_test_state_dir(|| {
        let state_store = StateStore::bootstrap()?;
        let topology = Topology {
            outputs: HashMap::from([(
                "HDMI-A-1".to_string(),
                daemon_output_state("HDMI-A-1", true),
            )]),
        };
        let plan = LayoutPlan::new(HashMap::from([(
            "HDMI-A-1".to_string(),
            daemon_output_state("HDMI-A-1", false),
        )]));
        let profile = daemon_profile("presentation", "HDMI-A-1", false);
        let execution = workflow::ApplyExecution::Rejected {
            plan,
            validation: ValidationResult::rejected(
                Some(ConfigFailureKind::Rejected),
                Some("configuration rejected".to_string()),
            ),
        };

        let daemon_apply = DaemonApplyWorkflow::new(
            &state_store,
            &profile,
            Some(&profile.name),
            BackendKind::Test,
            &topology,
            "HDMI-A-1:off:unknown@0,0".to_string(),
        );

        let Err(err) = daemon_apply.finish_execution(execution) else {
            return Err(std::io::Error::other("rejection should include daemon context").into());
        };
        let message = err.to_string();

        assert!(message.contains("profile 'presentation'"));
        assert!(message.contains("test backend"));
        assert!(message.contains("rejected"));
        assert!(message.contains("failure_kind=rejected"));
        assert!(message.contains("configuration rejected"));
        assert!(message.contains("current_outputs=HDMI-A-1:on:unknown@0,0"));
        assert!(message.contains("planned_outputs=HDMI-A-1:off:unknown@0,0"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn wait_for_stable_topology_reports_stable_when_samples_stop_changing() -> Result<(), Box<dyn Error>>
{
    with_test_state_dir(|| {
        let state_store = StateStore::bootstrap()?;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let backend = StubBackend {
            enumerated_topology: topology.clone(),
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let backend = StubBackend {
            enumerated_topology: topology.clone(),
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", false))]),
        };
        let apply_calls = Arc::new(Mutex::new(0));
        let test_calls = Arc::new(Mutex::new(0));
        let backend = StubBackend {
            enumerated_topology: topology,
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };

        enforce_topology_policy(&backend, &store, &state_store, false)?;

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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let backend = StubBackend {
            enumerated_topology: topology.clone(),
            applied_topology: None,
            test_success: false,
            validation_failure: Some(ConfigFailureKind::TopologyChanged),
            validation_message: None,
            apply_calls: Arc::new(Mutex::new(0)),
            test_calls: Arc::new(Mutex::new(0)),
        };
        let profile = daemon_profile("desk", "DP-1", false);
        store.save(&profile, &state_store)?;

        store.set_setup_default_profile(&topology.setup_fingerprint(), &profile.name)?;

        let Err(err) = enforce_topology_policy(&backend, &store, &state_store, false) else {
            return Err(std::io::Error::other("repeated topology changes should fail").into());
        };

        assert!(err
            .to_string()
            .contains("giving up after repeated topology changes during daemon apply"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn apply_profile_retries_when_backend_reports_blank_applied_topology() -> Result<(), Box<dyn Error>>
{
    with_test_state_dir(|| {
        let state_store = StateStore::bootstrap()?;
        let apply_calls = Arc::new(Mutex::new(0));
        let test_calls = Arc::new(Mutex::new(0));
        let current = Topology {
            outputs: HashMap::from([("eDP-1".to_string(), daemon_output_state("eDP-1", false))]),
        };
        let blank_after_apply = current.clone();
        let backend = StubBackend {
            enumerated_topology: current.clone(),
            applied_topology: Some(blank_after_apply),
            test_success: true,
            validation_failure: None,
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };
        let profile = daemon_profile("default", "eDP-1", true);

        let outcome = apply_profile(
            &backend,
            &state_store,
            &profile,
            &current,
            Some(&profile.name),
            false,
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
fn apply_profile_uses_prepared_topology_without_reenumerating() -> Result<(), Box<dyn Error>> {
    with_test_state_dir(|| {
        let state_store = StateStore::bootstrap()?;
        let enumerate_calls = Arc::new(Mutex::new(0));
        let apply_calls = Arc::new(Mutex::new(0));
        let test_calls = Arc::new(Mutex::new(0));
        let current = Topology {
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", false))]),
        };
        let backend = PreparedApplyBackend {
            enumerated_topology: Topology {
                outputs: HashMap::from([(
                    "HDMI-A-1".to_string(),
                    daemon_output_state("HDMI-A-1", true),
                )]),
            },
            applied_topology: Topology {
                outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
            },
            enumerate_calls: enumerate_calls.clone(),
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };
        let profile = daemon_profile("desk", "DP-1", true);

        let outcome = apply_profile(
            &backend,
            &state_store,
            &profile,
            &current,
            Some(&profile.name),
            false,
        )?;

        assert!(matches!(outcome, DaemonOutcome::Applied));
        assert_eq!(
            *enumerate_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            0
        );
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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let backend = StubBackend {
            enumerated_topology: topology.clone(),
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };
        let profile = daemon_profile("desk", "DP-1", true);

        let outcome = apply_profile(
            &backend,
            &state_store,
            &profile,
            &topology,
            Some(&profile.name),
            false,
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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", false))]),
        };
        let remembered = Topology {
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let apply_calls = Arc::new(Mutex::new(0));
        let test_calls = Arc::new(Mutex::new(0));
        let backend = StubBackend {
            enumerated_topology: remembered.clone(),
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };

        let mut state = state_store.load_state()?.unwrap_or_default();
        state
            .remembered_setups
            .insert(current.setup_fingerprint(), remembered.clone());
        state.last_profile = Some("old".to_string());
        state_store.save_state(&state)?;

        let outcome =
            maybe_apply_matching_profile(&backend, &store, &state_store, &current, false)?;
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
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", false))]),
        };
        let remembered = Topology {
            outputs: HashMap::from([("DP-1".to_string(), daemon_output_state("DP-1", true))]),
        };
        let apply_calls = Arc::new(Mutex::new(0));
        let test_calls = Arc::new(Mutex::new(0));
        let backend = StubBackend {
            enumerated_topology: current.clone(),
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };
        let profile = daemon_profile("desk", "DP-1", false);

        store.save(&profile, &state_store)?;

        let mut state = state_store.load_state()?.unwrap_or_default();
        state
            .remembered_setups
            .insert(current.setup_fingerprint(), remembered);
        state_store.save_state(&state)?;

        let outcome =
            maybe_apply_matching_profile(&backend, &store, &state_store, &current, false)?;
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
            outputs: HashMap::from([("eDP-1".to_string(), daemon_output_state("eDP-1", false))]),
        };
        let remembered = current.clone();
        let apply_calls = Arc::new(Mutex::new(0));
        let test_calls = Arc::new(Mutex::new(0));
        let backend = StubBackend {
            enumerated_topology: Topology {
                outputs: HashMap::from([("eDP-1".to_string(), daemon_output_state("eDP-1", true))]),
            },
            applied_topology: None,
            test_success: true,
            validation_failure: None,
            validation_message: None,
            apply_calls: apply_calls.clone(),
            test_calls: test_calls.clone(),
        };

        let mut state = state_store.load_state()?.unwrap_or_default();
        state
            .remembered_setups
            .insert(current.setup_fingerprint(), remembered);
        state_store.save_state(&state)?;

        let outcome =
            maybe_apply_matching_profile(&backend, &store, &state_store, &current, false)?;
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
                .get(&current.setup_fingerprint())
                .map(Topology::fingerprint),
            Some("eDP-1:off".to_string())
        );
        Ok(())
    })?;
    Ok(())
}
