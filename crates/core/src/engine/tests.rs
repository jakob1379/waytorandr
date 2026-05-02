use super::*;
use crate::error::CoreError;
use crate::model::{BackendKind, OutputState};
use crate::profile::{Hook, Hooks};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[test]
fn config_failure_kind_labels_are_stable() {
    assert_eq!(ConfigFailureKind::Rejected.as_label(), "rejected");
    assert_eq!(
        ConfigFailureKind::TopologyChanged.as_label(),
        "topology_changed"
    );
}

#[test]
fn apply_result_constructors_keep_status_and_applied_state_consistent() {
    let topology = Topology {
        outputs: HashMap::from([("DP-1".to_string(), OutputState::new("DP-1"))]),
    };

    let applied = ApplyResult::applied(Some("ok".to_string()), Some(topology));
    assert_eq!(applied.status(), ApplyStatus::Applied);
    assert_eq!(applied.message(), Some("ok"));
    assert!(applied.applied_state().is_some());
    assert_eq!(applied.failure(), None);

    let failed = ApplyResult::failed(Some(ConfigFailureKind::Rejected), Some("no".to_string()));
    assert_eq!(
        failed.status(),
        ApplyStatus::Failed {
            failure: Some(ConfigFailureKind::Rejected)
        }
    );
    assert_eq!(failed.message(), Some("no"));
    assert_eq!(failed.failure(), Some(ConfigFailureKind::Rejected));
    assert!(failed.applied_state().is_none());
}

#[derive(Clone)]
struct TestBackend {
    apply_calls: Arc<Mutex<usize>>,
}

impl Backend for TestBackend {
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
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        Ok(ValidationResult::supported(None))
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        *lock_apply_calls(self.apply_calls.as_ref()) += 1;
        Ok(ApplyResult::applied(
            None,
            Some(Topology {
                outputs: plan.outputs.clone(),
            }),
        ))
    }
}

fn lock_apply_calls(counter: &Mutex<usize>) -> std::sync::MutexGuard<'_, usize> {
    counter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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

fn failing_hook(label: &str) -> Hook {
    let mut hook = Hook::new("sh");
    hook.args = vec![
        "-c".to_string(),
        format!("printf '{label} diagnostic' >&2; exit 7"),
    ];
    hook.timeout_secs = 5;
    hook
}

#[test]
fn apply_plan_applies_once_and_runs_phase_specific_hooks() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
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

    let result = engine.apply_plan(&plan, &hooks)?;

    assert!(result.is_applied());
    assert_eq!(*lock_apply_calls(backend.apply_calls.as_ref()), 1);

    let log = std::fs::read_to_string(log_path)?;
    assert!(log.contains("pre"));
    assert!(log.contains("post"));
    assert!(!log.contains("failure"));
    Ok(())
}

#[test]
fn apply_plan_rejects_when_pre_hook_cannot_start() -> anyhow::Result<()> {
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

    let result = engine.apply_plan(&plan, &hooks)?;

    assert!(!result.is_applied());
    assert_eq!(result.failure(), Some(ConfigFailureKind::Rejected));
    assert!(result
        .message()
        .is_some_and(|message| message.contains("pre-apply hook")));
    assert_eq!(*lock_apply_calls(backend.apply_calls.as_ref()), 0);
    Ok(())
}

#[test]
fn apply_plan_reports_failed_hook_stderr() -> anyhow::Result<()> {
    let backend = TestBackend {
        apply_calls: Arc::new(Mutex::new(0)),
    };
    let engine = Engine::new(backend.clone());
    let mut hook = Hook::new("sh");
    hook.args = vec![
        "-c".to_string(),
        "printf diagnostic >&2; exit 7".to_string(),
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

    let result = engine.apply_plan(&plan, &hooks)?;

    assert!(!result.is_applied());
    assert!(result
        .message()
        .is_some_and(|message| message.contains("diagnostic")));
    assert_eq!(*lock_apply_calls(backend.apply_calls.as_ref()), 0);
    Ok(())
}

#[test]
fn apply_plan_surfaces_post_apply_hook_failure_without_overriding_apply_success(
) -> anyhow::Result<()> {
    let backend = TestBackend {
        apply_calls: Arc::new(Mutex::new(0)),
    };
    let engine = Engine::new(backend.clone());
    let hooks = Hooks {
        post_apply: vec![failing_hook("post")],
        ..Hooks::default()
    };
    let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
        let mut state = OutputState::new("DP-1");
        state.enabled = true;
        state
    })]));

    let result = engine.apply_plan(&plan, &hooks)?;

    assert!(result.is_applied());
    assert!(result.applied_state().is_some());
    assert!(result
        .message()
        .is_some_and(|message| message.contains("post diagnostic")));
    Ok(())
}

#[test]
fn apply_plan_surfaces_failure_hook_failure_without_overriding_backend_failure(
) -> anyhow::Result<()> {
    let backend = FailingApplyBackend;
    let engine = Engine::new(backend);
    let hooks = Hooks {
        on_failure: vec![failing_hook("failure")],
        ..Hooks::default()
    };
    let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
        let mut state = OutputState::new("DP-1");
        state.enabled = true;
        state
    })]));

    let result = engine.apply_plan(&plan, &hooks)?;

    assert!(!result.is_applied());
    assert_eq!(result.failure(), Some(ConfigFailureKind::Rejected));
    assert!(result.message().is_some_and(|message| {
        message.contains("backend rejected") && message.contains("failure diagnostic")
    }));
    Ok(())
}

#[test]
fn validate_plan_short_circuits_when_backend_cannot_validate() -> anyhow::Result<()> {
    let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), {
        let mut state = OutputState::new("DP-1");
        state.enabled = true;
        state
    })]));

    let no_validation_capabilities = Capabilities::new(BackendKind::Test);
    let backend = NoValidationBackend {
        capabilities: no_validation_capabilities,
    };
    let engine = Engine::new(backend);
    let result = engine.validate_plan(&plan)?;

    assert!(!result.is_accepted());
    assert_eq!(result.status, ValidationStatus::Unsupported);
    assert_eq!(
        result.message.as_deref(),
        Some("Backend does not support validation")
    );
    Ok(())
}

struct NoValidationBackend {
    capabilities: Capabilities,
}

impl Backend for NoValidationBackend {
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

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        Err(CoreError::Backend {
            source: anyhow::anyhow!("should not be called"),
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }
}

struct FailingApplyBackend;

impl Backend for FailingApplyBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new(BackendKind::Test)
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        Ok(Topology::default())
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
        Ok(ApplyResult::failed(
            Some(ConfigFailureKind::Rejected),
            Some("backend rejected".to_string()),
        ))
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
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let _ = plan;
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let _ = plan;
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
fn polling_output_watcher_ignores_non_physical_layout_changes() -> anyhow::Result<()> {
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

    let changed = watcher.poll_changed()?;

    assert!(changed.is_none());
    Ok(())
}

#[test]
fn polling_output_watcher_reports_physical_setup_changes() -> anyhow::Result<()> {
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

    let changed = watcher.poll_changed()?;

    assert_eq!(changed, Some(changed_topology));
    Ok(())
}

#[test]
fn polling_output_watcher_reports_blank_setup_changes() -> anyhow::Result<()> {
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

    assert_eq!(watcher.poll_changed()?, Some(blank));
    assert!(watcher.poll_changed()?.is_none());
    Ok(())
}
