use crate::engine::{ApplyResult, Backend, Engine, TestResult, ValidationStatus};
use crate::error::{CoreError, CoreResult};
use crate::matcher::Matcher;
use crate::model::{BackendKind, Topology, VirtualPreset};
use crate::normalize::normalize_topology_with_known_outputs;
use crate::planner::{LayoutPlan, Planner};
use crate::profile::{Hooks, OutputMatcher, Profile};
use crate::state::{State, StateStore};

pub enum ExecutionCycle {
    DryRun {
        validation_plan: LayoutPlan,
        validation: TestResult,
    },
    Unsupported {
        validation_plan: LayoutPlan,
        validation: TestResult,
    },
    Rejected {
        validation_plan: LayoutPlan,
        validation: TestResult,
    },
    Applied {
        validation_plan: LayoutPlan,
        validation: TestResult,
        apply_plan: LayoutPlan,
        apply_result: ApplyResult,
        applied_topology: Topology,
    },
}

pub struct PlanSnapshot {
    pub topology: Topology,
    pub plan: LayoutPlan,
}

#[must_use]
pub fn default_profile_for_setup<'a>(state: &'a State, setup_fingerprint: &str) -> Option<&'a str> {
    state
        .default_profiles
        .get(setup_fingerprint)
        .map(String::as_str)
        .or_else(|| state.global_default_profile())
}

#[must_use]
pub fn remembered_topology_for_setup<'a>(
    state: &'a State,
    setup_fingerprint: &str,
) -> Option<&'a Topology> {
    state.remembered_setups.get(setup_fingerprint)
}

#[must_use]
pub fn select_profile_for_topology(
    topology: &Topology,
    profiles: &[Profile],
    state: &State,
) -> Option<Profile> {
    let setup_fingerprint = topology.setup_fingerprint();
    if let Some(profile) = state
        .default_profiles
        .get(&setup_fingerprint)
        .and_then(|default_name| {
            profiles
                .iter()
                .find(|profile| profile.name == *default_name)
        })
    {
        return Some(profile.clone());
    }

    if let Some(matched) = Matcher::match_profile(topology, profiles) {
        return Some(matched.profile);
    }

    let default_name = state.global_default_profile()?;
    profiles
        .iter()
        .find(|profile| profile.name == default_name)
        .cloned()
}

#[must_use]
pub fn current_profile_name(
    topology: &Topology,
    profiles: &[Profile],
    state: &State,
) -> Option<String> {
    if let Some(profile) = state
        .last_profile
        .as_deref()
        .and_then(|last_profile| profiles.iter().find(|profile| profile.name == last_profile))
    {
        if Matcher::match_profile(topology, std::slice::from_ref(profile)).is_some() {
            return Some(profile.name.clone());
        }
    }

    Matcher::match_profile(topology, profiles).map(|matched| matched.profile.name)
}

#[must_use]
pub fn profile_from_topology(name: &str, topology: &Topology) -> Profile {
    Profile {
        name: name.to_string(),
        priority: 0,
        match_rules: topology
            .outputs
            .values()
            .filter(|output| !output.identity.is_ignored && !output.identity.is_virtual)
            .map(|output| OutputMatcher {
                identity: output.identity.clone(),
                required: output.enabled,
                position_hint: Some(output.position),
            })
            .collect(),
        layout: topology
            .outputs
            .iter()
            .map(|(output_name, output)| (output_name.clone(), output.clone().into()))
            .collect(),
        hooks: Hooks::default(),
    }
}

/// Build a layout plan for `profile` against `topology`.
///
/// # Errors
/// Returns `CoreError::ProfileMismatch` if the profile does not match the topology,
/// or any planning error reported by the planner.
pub fn plan_profile_for_topology(profile: &Profile, topology: &Topology) -> CoreResult<LayoutPlan> {
    let matched = Matcher::match_profile(topology, std::slice::from_ref(profile))
        .ok_or(CoreError::ProfileMismatch)?;
    Planner::plan_from_profile(&matched.profile, &matched.matched_outputs, topology)
        .map_err(Into::into)
}

/// Load the backend topology and normalize it using stored known outputs.
///
/// # Errors
/// Returns an error if the backend cannot enumerate outputs, or if the state store
/// cannot be read.
pub fn normalized_topology_from_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
) -> CoreResult<Topology> {
    let topology = backend.enumerate_outputs()?;
    let state = state_store.load_state()?.unwrap_or_default();
    Ok(normalize_topology_with_known_outputs(
        &topology,
        &state.known_outputs,
    ))
}

/// Load the backend topology and persist it as the latest observed state.
///
/// # Errors
/// Returns an error if the backend cannot enumerate outputs, or if the observed
/// topology cannot be persisted.
pub fn observed_topology_from_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
) -> CoreResult<Topology> {
    let topology = backend.enumerate_outputs()?;
    state_store.observe_topology_and_persist_known_outputs(&topology)
}

/// Load a normalized topology from the backend and plan `profile` for it.
///
/// # Errors
/// Returns an error if topology loading or profile planning fails.
pub fn plan_profile_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<PlanSnapshot> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = plan_profile_for_topology(profile, &topology)?;
    Ok(PlanSnapshot { topology, plan })
}

/// Load a normalized topology from the backend and plan `preset` for it.
///
/// # Errors
/// Returns an error if topology loading or preset planning fails.
pub fn plan_preset_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
) -> CoreResult<PlanSnapshot> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = Planner::plan_from_preset(preset, &topology, None)?;
    Ok(PlanSnapshot { topology, plan })
}

/// Validate a layout plan against the backend.
///
/// # Errors
/// Returns an error if the backend cannot run validation.
pub fn validate_plan<B: Backend + ?Sized>(
    backend: &B,
    plan: &LayoutPlan,
) -> CoreResult<TestResult> {
    let engine = Engine::new(backend);
    engine.test_plan(plan)
}

/// Apply a layout plan through the backend.
///
/// # Errors
/// Returns an error if the backend cannot apply the plan.
pub fn apply_plan<B: Backend + ?Sized>(
    backend: &B,
    hooks: &Hooks,
    plan: &LayoutPlan,
) -> CoreResult<ApplyResult> {
    let engine = Engine::new(backend);
    engine.apply_plan(plan, hooks)
}

fn invalid_validation_cycle(validation_plan: LayoutPlan, validation: TestResult) -> ExecutionCycle {
    if validation.status == ValidationStatus::Unsupported {
        ExecutionCycle::Unsupported {
            validation_plan,
            validation,
        }
    } else {
        ExecutionCycle::Rejected {
            validation_plan,
            validation,
        }
    }
}

/// Validate a plan without applying it.
///
/// # Errors
/// Returns an error if validation fails.
pub fn validate_plan_cycle<B: Backend + ?Sized>(
    backend: &B,
    validation_snapshot: PlanSnapshot,
) -> CoreResult<ExecutionCycle> {
    let validation = validate_plan(backend, &validation_snapshot.plan)?;

    Ok(match validation.status {
        ValidationStatus::Supported => ExecutionCycle::DryRun {
            validation_plan: validation_snapshot.plan,
            validation,
        },
        _ => invalid_validation_cycle(validation_snapshot.plan, validation),
    })
}

/// Validate a plan and apply it if validation succeeds.
///
/// # Errors
/// Returns an error if validation or applying the plan fails.
pub fn apply_plan_cycle<B: Backend + ?Sized>(
    backend: &B,
    hooks: &Hooks,
    validation_snapshot: PlanSnapshot,
    apply_snapshot: PlanSnapshot,
) -> CoreResult<ExecutionCycle> {
    let validation = validate_plan(backend, &validation_snapshot.plan)?;

    if !validation.is_supported() {
        return Ok(invalid_validation_cycle(
            validation_snapshot.plan,
            validation,
        ));
    }

    let apply_result = apply_plan(backend, hooks, &apply_snapshot.plan)?;
    let applied_topology = apply_result
        .applied_state
        .clone()
        .unwrap_or(apply_snapshot.topology);

    Ok(ExecutionCycle::Applied {
        validation_plan: validation_snapshot.plan,
        validation,
        apply_plan: apply_snapshot.plan,
        apply_result,
        applied_topology,
    })
}

/// Persist runtime state after a successful apply.
///
/// # Errors
/// Returns an error if the topology or state cannot be loaded or saved.
pub fn persist_applied_runtime_state(
    state_store: &StateStore,
    profile_name: &str,
    backend: Option<BackendKind>,
    topology: &Topology,
) -> CoreResult<()> {
    let topology = state_store.observe_topology_and_persist_known_outputs(topology)?;
    let mut state = state_store.load_state()?.unwrap_or_default();
    state.record_applied_profile(profile_name, backend, &topology);
    state_store.save_state(&state)?;
    Ok(())
}

/// Persist runtime state after observing a topology.
///
/// # Errors
/// Returns an error if the topology or state cannot be loaded or saved.
pub fn persist_observed_runtime_state(
    state_store: &StateStore,
    backend: Option<BackendKind>,
    topology: &Topology,
) -> CoreResult<()> {
    let topology = state_store.observe_topology_and_persist_known_outputs(topology)?;
    let mut state = state_store.load_state()?.unwrap_or_default();
    state.record_observed_topology(backend, &topology);
    state_store.save_state(&state)?;
    Ok(())
}

/// Persist the default profile for a setup fingerprint.
///
/// # Errors
/// Returns an error if the state cannot be loaded or saved.
pub fn set_default_profile_for_setup_in_store(
    state_store: &StateStore,
    setup_fingerprint: &str,
    profile_name: &str,
) -> CoreResult<()> {
    let mut state = state_store.load_state()?.unwrap_or_default();
    state.set_default_profile_for_setup(setup_fingerprint, profile_name);
    state_store.save_state(&state)?;
    Ok(())
}

/// Persist the backend that started the daemon.
///
/// # Errors
/// Returns an error if the state cannot be loaded or saved.
pub fn record_daemon_started_in_store(
    state_store: &StateStore,
    backend: BackendKind,
) -> CoreResult<()> {
    let mut state = state_store.load_state()?.unwrap_or_default();
    state.record_daemon_started(backend);
    state_store.save_state(&state)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::OutputWatcher;
    use crate::model::{OutputIdentity, OutputState};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn output(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state
    }

    fn profile(name: &str, connector: &str) -> Profile {
        Profile {
            name: name.to_string(),
            priority: 0,
            match_rules: vec![crate::profile::OutputMatcher {
                identity: OutputIdentity::new(connector),
                required: true,
                position_hint: Some(crate::model::Position::default()),
            }],
            layout: HashMap::from([(
                connector.to_string(),
                crate::profile::OutputConfig {
                    state: output(connector),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        }
    }

    #[test]
    fn select_profile_prefers_setup_default_before_match() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let mut state = State::default();
        state
            .default_profiles
            .insert(topology.setup_fingerprint(), "external-only".to_string());
        let profiles = vec![profile("both", "DP-1"), profile("external-only", "DP-1")];

        let selected = select_profile_for_topology(&topology, &profiles, &state).unwrap();

        assert_eq!(selected.name, "external-only");
    }

    #[test]
    fn select_profile_prefers_match_before_global_default() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let mut state = State::default();
        state.default_profiles.insert(
            State::GLOBAL_DEFAULT_PROFILE_KEY.to_string(),
            "fallback".to_string(),
        );
        let profiles = vec![profile("desk", "DP-1"), profile("fallback", "HDMI-A-1")];

        let selected = select_profile_for_topology(&topology, &profiles, &state).unwrap();

        assert_eq!(selected.name, "desk");
    }

    #[test]
    fn record_applied_profile_updates_runtime_state() {
        let mut state = State::default();
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };

        state.record_applied_profile("desk", Some(BackendKind::Wlroots), &topology);

        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert!(state.last_topology_fingerprint.is_some());
        assert_eq!(
            state
                .remembered_setups
                .get(&topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
    }

    #[test]
    fn record_observed_topology_clears_last_profile_and_remembers_setup() {
        let mut state = State::default();
        state.last_profile = Some("desk".to_string());
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };

        state.record_observed_topology(Some(BackendKind::Wlroots), &topology);

        assert_eq!(state.last_profile, None);
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert_eq!(
            remembered_topology_for_setup(&state, &topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
    }

    #[test]
    fn current_profile_name_uses_recorded_profile_when_it_still_matches() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1")];
        let mut state = State::default();
        state.last_profile = Some("desk".to_string());

        let selected = current_profile_name(&topology, &profiles, &state);

        assert_eq!(selected.as_deref(), Some("desk"));
    }

    #[test]
    fn current_profile_name_falls_back_when_recorded_profile_is_stale() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1"), profile("manual", "HDMI-A-1")];
        let mut state = State::default();
        state.last_profile = Some("manual".to_string());

        let selected = current_profile_name(&topology, &profiles, &state);

        assert_eq!(selected.as_deref(), Some("desk"));
    }

    #[test]
    fn profile_from_topology_builds_matchers_from_real_outputs() {
        let mut virtual_output = output("HEADLESS-1");
        virtual_output.identity.is_virtual = true;
        let mut topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), output("DP-1")),
                ("HEADLESS-1".to_string(), virtual_output),
            ]),
        };
        topology.outputs.get_mut("DP-1").unwrap().enabled = false;

        let profile = profile_from_topology("desk", &topology);

        assert_eq!(profile.name, "desk");
        assert_eq!(profile.layout.len(), 2);
        assert_eq!(profile.match_rules.len(), 1);
        assert_eq!(
            profile.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
        assert!(!profile.match_rules[0].required);
    }

    #[test]
    fn plan_profile_for_topology_returns_mismatch_error_for_nonmatching_profile() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profile = profile("desk", "HDMI-A-1");

        let result = plan_profile_for_topology(&profile, &topology);

        assert!(matches!(result, Err(CoreError::ProfileMismatch)));
    }

    #[test]
    fn plan_profile_for_topology_uses_matched_layout_binding() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: vec![crate::profile::OutputMatcher {
                identity: OutputIdentity::new("DP-1"),
                required: true,
                position_hint: Some(crate::model::Position::default()),
            }],
            layout: HashMap::from([(
                "left-panel".to_string(),
                crate::profile::OutputConfig {
                    state: output("DP-1"),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        };

        let plan = plan_profile_for_topology(&profile, &topology).unwrap();

        assert!(plan.outputs.contains_key("DP-1"));
        assert_eq!(
            plan.outputs["DP-1"].identity.connector.as_deref(),
            Some("DP-1")
        );
    }

    #[derive(Clone)]
    struct CycleBackend {
        test_result: TestResult,
        test_calls: Arc<Mutex<usize>>,
        apply_calls: Arc<Mutex<usize>>,
    }

    impl Backend for CycleBackend {
        fn capabilities(&self) -> crate::model::Capabilities {
            let mut capabilities = crate::model::Capabilities::new(BackendKind::Test);
            capabilities.can_test = true;
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

        fn test(&self, _plan: &LayoutPlan) -> CoreResult<TestResult> {
            *self.test_calls.lock().unwrap() += 1;
            Ok(self.test_result.clone())
        }

        fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            *self.apply_calls.lock().unwrap() += 1;
            Ok(ApplyResult {
                success: true,
                applied_state: Some(Topology {
                    outputs: plan.outputs.clone(),
                }),
                failure: None,
                message: None,
            })
        }
    }

    #[test]
    fn execute_plan_cycle_skips_apply_on_dry_run() {
        let backend = CycleBackend {
            test_result: TestResult::supported(None),
            test_calls: Arc::new(Mutex::new(0)),
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let cycle = validate_plan_cycle(
            &backend,
            PlanSnapshot {
                topology: topology.clone(),
                plan: LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            },
        )
        .unwrap();

        match cycle {
            ExecutionCycle::DryRun { validation, .. } => {
                assert!(validation.success);
                assert!(validation.is_supported());
            }
            ExecutionCycle::Unsupported { .. }
            | ExecutionCycle::Rejected { .. }
            | ExecutionCycle::Applied { .. } => panic!("expected dry-run cycle"),
        }
        assert_eq!(*backend.test_calls.lock().unwrap(), 1);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 0);
    }

    #[test]
    fn execute_plan_cycle_applies_after_successful_validation() {
        let backend = CycleBackend {
            test_result: TestResult::supported(None),
            test_calls: Arc::new(Mutex::new(0)),
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let hooks = Hooks::default();

        let cycle = apply_plan_cycle(
            &backend,
            &hooks,
            PlanSnapshot {
                topology: topology.clone(),
                plan: LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            },
            PlanSnapshot {
                topology: topology.clone(),
                plan: LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            },
        )
        .unwrap();

        match cycle {
            ExecutionCycle::Applied {
                validation,
                apply_result,
                ..
            } => {
                assert!(validation.success);
                assert!(validation.is_supported());
                assert!(apply_result.success);
            }
            ExecutionCycle::DryRun { .. }
            | ExecutionCycle::Unsupported { .. }
            | ExecutionCycle::Rejected { .. } => panic!("expected applied cycle"),
        }
        assert_eq!(*backend.test_calls.lock().unwrap(), 1);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);
    }

    #[test]
    fn execute_plan_cycle_stops_when_validation_is_unsupported() {
        let backend = CycleBackend {
            test_result: TestResult::unsupported(None),
            test_calls: Arc::new(Mutex::new(0)),
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let hooks = Hooks::default();

        let cycle = apply_plan_cycle(
            &backend,
            &hooks,
            PlanSnapshot {
                topology: topology.clone(),
                plan: LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            },
            PlanSnapshot {
                topology: topology.clone(),
                plan: LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            },
        )
        .unwrap();

        match cycle {
            ExecutionCycle::Unsupported { validation, .. } => {
                assert_eq!(validation.status, ValidationStatus::Unsupported);
                assert!(!validation.success);
            }
            ExecutionCycle::DryRun { .. }
            | ExecutionCycle::Rejected { .. }
            | ExecutionCycle::Applied { .. } => panic!("expected unsupported cycle"),
        }
        assert_eq!(*backend.test_calls.lock().unwrap(), 1);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 0);
    }
}
