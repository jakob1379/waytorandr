use super::*;
use crate::engine::OutputWatcher;
use crate::error::{CoreError, CoreResult};
use crate::model::{BackendKind, Capabilities, OutputIdentity, OutputState, Position};
use crate::profile::{OutputConfig, OutputMatcher};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn execution_output_state(connector: &str) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = true;
    state
}

fn execution_profile(name: &str, connector: &str) -> Profile {
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
            OutputConfig::from(execution_output_state(connector)),
        )]),
    )
}

struct CycleBackend {
    validation_result: ValidationResult,
    apply_success: bool,
    apply_failure: Option<ConfigFailureKind>,
    apply_message: Option<String>,
    applied_state: Option<Topology>,
    test_calls: Arc<Mutex<u32>>,
    apply_calls: Arc<Mutex<u32>>,
}

impl Backend for CycleBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Test);
        capabilities.can_validate = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        Ok(Topology::default())
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used"),
        })
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        *lock_calls(self.test_calls.as_ref()) += 1;
        Ok(self.validation_result.clone())
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        *lock_calls(self.apply_calls.as_ref()) += 1;
        Ok(if self.apply_success {
            ApplyResult::applied(self.apply_message.clone(), self.applied_state.clone())
        } else {
            ApplyResult::failed(self.apply_failure, self.apply_message.clone())
        })
    }
}

fn lock_calls(counter: &Mutex<u32>) -> std::sync::MutexGuard<'_, u32> {
    counter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn prepared_plan(connector: &str) -> PreparedPlanApplication {
    let plan = LayoutPlan::new(HashMap::from([(
        connector.to_string(),
        execution_output_state(connector),
    )]));
    PreparedPlanApplication::new(plan)
}

#[test]
fn prepare_profile_application_builds_plan_for_topology() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([("DP-1".to_string(), execution_output_state("DP-1"))]),
    };

    let prepared = prepare_profile_application(&execution_profile("desk", "DP-1"), &topology)?;

    assert_eq!(prepared.plan().outputs, topology.outputs);
    Ok(())
}

#[test]
fn validate_plan_cycle_returns_unsupported_without_apply() -> anyhow::Result<()> {
    let backend = CycleBackend {
        validation_result: ValidationResult::unsupported(Some(
            "backend cannot validate".to_string(),
        )),
        apply_success: false,
        apply_failure: None,
        apply_message: None,
        applied_state: None,
        test_calls: Arc::new(Mutex::new(0)),
        apply_calls: Arc::new(Mutex::new(0)),
    };

    let execution = validate_plan_cycle(&backend, prepared_plan("DP-1"))?;

    let ValidationExecution::Unsupported { validation, .. } = execution else {
        anyhow::bail!("expected unsupported validation execution");
    };
    assert_eq!(validation.status, ValidationStatus::Unsupported);
    assert_eq!(
        validation.message.as_deref(),
        Some("backend cannot validate")
    );
    assert_eq!(*lock_calls(backend.test_calls.as_ref()), 1);
    assert_eq!(*lock_calls(backend.apply_calls.as_ref()), 0);
    Ok(())
}

#[test]
fn apply_plan_cycle_rejects_without_apply() -> anyhow::Result<()> {
    let backend = CycleBackend {
        validation_result: ValidationResult::rejected(
            Some(ConfigFailureKind::Rejected),
            Some("layout rejected".to_string()),
        ),
        apply_success: false,
        apply_failure: None,
        apply_message: None,
        applied_state: None,
        test_calls: Arc::new(Mutex::new(0)),
        apply_calls: Arc::new(Mutex::new(0)),
    };

    let execution = apply_plan_cycle(&backend, &Hooks::default(), prepared_plan("DP-1"))?;

    let ApplyExecution::Rejected { validation, .. } = execution else {
        anyhow::bail!("expected rejected apply execution");
    };
    assert_eq!(validation.failure(), Some(ConfigFailureKind::Rejected));
    assert_eq!(validation.message.as_deref(), Some("layout rejected"));
    assert_eq!(*lock_calls(backend.test_calls.as_ref()), 1);
    assert_eq!(*lock_calls(backend.apply_calls.as_ref()), 0);
    Ok(())
}

#[test]
fn apply_prepared_profile_workflow_reports_apply_failure() -> anyhow::Result<()> {
    let backend = CycleBackend {
        validation_result: ValidationResult::supported(None),
        apply_success: false,
        apply_failure: Some(ConfigFailureKind::TopologyChanged),
        apply_message: Some("outputs changed".to_string()),
        applied_state: None,
        test_calls: Arc::new(Mutex::new(0)),
        apply_calls: Arc::new(Mutex::new(0)),
    };

    let execution =
        apply_prepared_profile_workflow(&backend, &Hooks::default(), prepared_plan("DP-1"))?;

    let ApplyExecution::ApplyFailed {
        validation,
        apply_result,
        ..
    } = execution
    else {
        anyhow::bail!("expected apply failure");
    };
    assert!(validation.is_accepted());
    assert_eq!(
        apply_result.failure(),
        Some(ConfigFailureKind::TopologyChanged)
    );
    assert_eq!(apply_result.message(), Some("outputs changed"));
    assert_eq!(*lock_calls(backend.test_calls.as_ref()), 1);
    assert_eq!(*lock_calls(backend.apply_calls.as_ref()), 1);
    Ok(())
}

#[test]
fn apply_prepared_profile_workflow_uses_applied_state_from_backend() -> anyhow::Result<()> {
    let applied_topology = Topology {
        outputs: HashMap::from([("HDMI-A-1".to_string(), execution_output_state("HDMI-A-1"))]),
    };
    let backend = CycleBackend {
        validation_result: ValidationResult::supported(None),
        apply_success: true,
        apply_failure: None,
        apply_message: None,
        applied_state: Some(applied_topology.clone()),
        test_calls: Arc::new(Mutex::new(0)),
        apply_calls: Arc::new(Mutex::new(0)),
    };

    let execution =
        apply_prepared_profile_workflow(&backend, &Hooks::default(), prepared_plan("DP-1"))?;

    let ApplyExecution::Applied {
        applied_topology: actual,
        ..
    } = execution
    else {
        anyhow::bail!("expected applied execution");
    };
    assert_eq!(actual.outputs, applied_topology.outputs);
    Ok(())
}
