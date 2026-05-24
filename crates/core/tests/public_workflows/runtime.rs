use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::support::{
    public_workflow_output_state, public_workflow_profile, with_test_dirs, TestBackend, TestResult,
};
use waytorandr_core::workflow;
use waytorandr_core::{
    BackendKind, ConfigFailureKind, Hook, Hooks, ProfilesSettings, State, StateStore, Topology,
    ValidationResult,
};

#[test]
fn runtime_selects_applies_and_records_matching_profile() -> TestResult {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology: topology.clone(),
            enumerate_calls: Arc::new(Mutex::new(0)),
            can_validate: true,
            validation_result: ValidationResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };
        let profiles = vec![
            public_workflow_profile("desk", "DP-1"),
            public_workflow_profile("fallback", "HDMI-A-1"),
        ];
        let settings = ProfilesSettings::default();
        let mut state = State::default();

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &settings)
            .ok_or_else(|| std::io::Error::other("matching profile should be selected"))?;
        let cycle = workflow::apply_profile_workflow(&backend, &state_store, &selected)?;
        assert!(
            matches!(cycle, workflow::ApplyExecution::Applied { apply_result, .. } if apply_result.is_applied())
        );
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );

        state.record_applied_profile(&selected.name, Some(BackendKind::Test), &topology);
        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_prefers_setup_default_over_matching_profile() -> TestResult {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let profiles = vec![
            public_workflow_profile("both", "DP-1"),
            public_workflow_profile("external-only", "DP-1"),
        ];
        let mut settings = ProfilesSettings::default();
        settings.set_setup_default_profile(&topology.setup_fingerprint(), "external-only");

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &settings)
            .ok_or_else(|| std::io::Error::other("setup default should be selected"))?;

        assert_eq!(selected.name, "external-only");
        Ok(())
    })?;
    Ok(())
}

#[test]
fn setup_names_persist_per_setup_fingerprint() -> TestResult {
    with_test_dirs(|_| {
        let state_store = StateStore::bootstrap()?;
        workflow::set_setup_name_for_setup_in_store(&state_store, "conn:DP-1", "office")?;

        let state = state_store
            .load_state()?
            .ok_or_else(|| std::io::Error::other("state should exist"))?;

        assert_eq!(state.setup_name_for_setup("conn:DP-1"), Some("office"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_cycle_applies_plan_once_through_public_api() -> TestResult {
    with_test_dirs(|temp| {
        let log_path = temp.path().join("hooks.log");
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology,
            enumerate_calls: Arc::new(Mutex::new(0)),
            can_validate: true,
            validation_result: ValidationResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };
        let mut pre_hook = Hook::new("sh");
        pre_hook.args = vec![
            "-c".to_string(),
            format!("printf '%s\\n' pre >> {}", log_path.display()),
        ];
        pre_hook.timeout_secs = 5;
        let mut post_hook = Hook::new("sh");
        post_hook.args = vec![
            "-c".to_string(),
            format!("printf '%s\\n' post >> {}", log_path.display()),
        ];
        post_hook.timeout_secs = 5;
        let mut hooks = Hooks::default();
        hooks.pre_apply = vec![pre_hook];
        hooks.post_apply = vec![post_hook];
        let mut profile = public_workflow_profile("desk", "DP-1");
        profile.hooks = hooks;

        let cycle = workflow::apply_profile_workflow(&backend, &state_store, &profile)?;
        assert!(
            matches!(cycle, workflow::ApplyExecution::Applied { apply_result, .. } if apply_result.is_applied())
        );
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );
        assert_eq!(
            *backend
                .enumerate_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );

        let log = std::fs::read_to_string(log_path)?;
        assert!(log.contains("pre"));
        assert!(log.contains("post"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn validate_profile_workflow_returns_accepted_plan() -> TestResult {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology: topology.clone(),
            enumerate_calls: Arc::new(Mutex::new(0)),
            can_validate: true,
            validation_result: ValidationResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };

        let execution = workflow::validate_profile_workflow(
            &backend,
            &state_store,
            &public_workflow_profile("desk", "DP-1"),
        )?;

        assert!(matches!(
            execution,
            workflow::ValidationExecution::Accepted {
                ref plan,
                ref validation,
            } if validation.is_accepted() && plan.outputs.contains_key("DP-1")
        ));
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            0
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn apply_profile_workflow_returns_structured_apply_failures() -> TestResult {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology,
            enumerate_calls: Arc::new(Mutex::new(0)),
            can_validate: true,
            validation_result: ValidationResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: false,
            apply_failure: Some(ConfigFailureKind::TopologyChanged),
            apply_message: Some("changed".to_string()),
        };

        let execution = workflow::apply_profile_workflow(
            &backend,
            &state_store,
            &public_workflow_profile("desk", "DP-1"),
        )?;

        assert!(matches!(
            execution,
            workflow::ApplyExecution::ApplyFailed { ref apply_result, .. }
                if apply_result.failure() == Some(ConfigFailureKind::TopologyChanged)
                    && apply_result.message() == Some("changed")
        ));
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn apply_profile_workflow_applies_when_unsupported_validation_is_allowed() -> TestResult {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology,
            enumerate_calls: Arc::new(Mutex::new(0)),
            can_validate: false,
            validation_result: ValidationResult::unsupported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };

        let execution = workflow::apply_profile_workflow_with_policy(
            &backend,
            &state_store,
            &public_workflow_profile("desk", "DP-1"),
            workflow::ApplyPolicy {
                allow_unsupported_validation: true,
            },
        )?;

        assert!(matches!(
            execution,
            workflow::ApplyExecution::Applied { .. }
        ));
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn apply_prepared_profile_workflow_uses_existing_topology_snapshot() -> TestResult {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
        };
        let backend = TestBackend {
            topology: Topology {
                outputs: HashMap::from([(
                    "HDMI-A-1".to_string(),
                    public_workflow_output_state("HDMI-A-1"),
                )]),
            },
            enumerate_calls: Arc::new(Mutex::new(0)),
            can_validate: true,
            validation_result: ValidationResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };
        let profile = public_workflow_profile("desk", "DP-1");
        let prepared = workflow::prepare_profile_application(&profile, &topology)?;

        let execution =
            workflow::apply_prepared_profile_workflow(&backend, &profile.hooks, prepared)?;

        assert!(matches!(
            execution,
            workflow::ApplyExecution::Applied { ref plan, .. } if plan.outputs.contains_key("DP-1")
        ));
        assert_eq!(
            *backend
                .enumerate_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            0
        );
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );
        Ok(())
    })?;
    Ok(())
}
