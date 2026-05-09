use crate::engine::{
    ApplyResult, Backend, ConfigFailureKind, Engine, HookPolicy, TestResult, ValidationStatus,
};
use crate::error::{CoreError, CoreResult};
use crate::matcher::Matcher;
use crate::model::{BackendKind, Topology, VirtualPreset};
use crate::normalize::normalize_topology_with_known_outputs;
use crate::planner::{LayoutPlan, Planner};
use crate::profile::{Hooks, OutputMatcher, Profile};
use crate::state::{State, StateStore};
use crate::store::ProfilesSettings;

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

#[non_exhaustive]
struct PlanSnapshot {
    pub topology: Topology,
    pub plan: LayoutPlan,
}

impl PlanSnapshot {
    #[must_use]
    pub fn new(topology: Topology, plan: LayoutPlan) -> Self {
        Self { topology, plan }
    }
}

pub enum ValidationExecution {
    Accepted {
        plan: LayoutPlan,
        validation: TestResult,
    },
    Unsupported {
        plan: LayoutPlan,
        validation: TestResult,
    },
    Rejected {
        plan: LayoutPlan,
        validation: TestResult,
    },
}

impl ValidationExecution {
    #[must_use]
    pub fn failure_kind(&self) -> Option<ConfigFailureKind> {
        match self {
            Self::Unsupported { validation, .. } | Self::Rejected { validation, .. } => {
                validation.failure
            }
            Self::Accepted { .. } => None,
        }
    }

    #[must_use]
    pub fn failure_message(&self) -> Option<&str> {
        match self {
            Self::Unsupported { validation, .. } | Self::Rejected { validation, .. } => {
                validation.message.as_deref()
            }
            Self::Accepted { .. } => None,
        }
    }
}

pub enum ApplyExecution {
    Unsupported {
        plan: LayoutPlan,
        validation: TestResult,
    },
    Rejected {
        plan: LayoutPlan,
        validation: TestResult,
    },
    ApplyFailed {
        plan: LayoutPlan,
        validation: TestResult,
        apply_result: ApplyResult,
    },
    Applied {
        plan: LayoutPlan,
        validation: TestResult,
        apply_result: ApplyResult,
        applied_topology: Topology,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ApplyPolicy {
    pub allow_unsupported_validation: bool,
    pub hook_policy: HookPolicy,
}

impl Default for ApplyPolicy {
    fn default() -> Self {
        Self {
            allow_unsupported_validation: false,
            hook_policy: HookPolicy::Enabled,
        }
    }
}

impl ApplyExecution {
    #[must_use]
    pub fn failure_kind(&self) -> Option<ConfigFailureKind> {
        match self {
            Self::Unsupported { validation, .. } | Self::Rejected { validation, .. } => {
                validation.failure
            }
            Self::ApplyFailed { apply_result, .. } => apply_result.failure,
            Self::Applied { .. } => None,
        }
    }

    #[must_use]
    pub fn failure_message(&self) -> Option<&str> {
        match self {
            Self::Unsupported { validation, .. } | Self::Rejected { validation, .. } => {
                validation.message.as_deref()
            }
            Self::ApplyFailed { apply_result, .. } => apply_result.message.as_deref(),
            Self::Applied { .. } => None,
        }
    }
}

#[must_use]
pub fn select_target_for_topology(
    topology: &Topology,
    profiles: &[Profile],
    settings: &ProfilesSettings,
) -> Option<Profile> {
    let setup_fingerprint = topology.setup_fingerprint();
    if let Some(profile) =
        settings
            .setup_default_profile(&setup_fingerprint)
            .and_then(|default_name| {
                profiles.iter().find(|profile| {
                    profile.name == default_name && profile.setup_fingerprint() == setup_fingerprint
                })
            })
    {
        return Some(profile.clone());
    }

    Matcher::match_profile_exact(topology, profiles).map(|matched| matched.profile)
}

#[must_use]
pub fn select_trusted_target_for_topology(
    topology: &Topology,
    profiles: &[Profile],
    settings: &ProfilesSettings,
) -> Option<Profile> {
    if !topology.has_strong_setup_identity() {
        return None;
    }

    let setup_fingerprint = topology.setup_fingerprint();
    let trusted_profiles: Vec<Profile> = profiles
        .iter()
        .filter(|profile| profile.setup_fingerprint() == setup_fingerprint)
        .cloned()
        .collect();

    select_target_for_topology(topology, &trusted_profiles, settings)
}

#[must_use]
pub fn select_profile_for_topology(
    topology: &Topology,
    profiles: &[Profile],
    settings: &ProfilesSettings,
) -> Option<Profile> {
    select_target_for_topology(topology, profiles, settings)
}

#[must_use]
pub fn current_profile_name(
    topology: &Topology,
    profiles: &[Profile],
    state: &State,
) -> Option<String> {
    state
        .last_profile
        .as_deref()
        .and_then(|last_profile| profiles.iter().find(|profile| profile.name == last_profile))
        .and_then(|profile| {
            if Matcher::match_profile_exact(topology, std::slice::from_ref(profile)).is_some() {
                Some(profile.name.clone())
            } else {
                None
            }
        })
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
            .map(|(output_name, output)| {
                (
                    output_name.clone(),
                    output.clone().with_refreshed_scaled_resolution().into(),
                )
            })
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
    let matched = Matcher::match_profile_exact(topology, std::slice::from_ref(profile))
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
    let topology = bounded_topology_from_backend(backend)?;
    let state = state_store.load_state()?.unwrap_or_default();
    let normalized = normalize_topology_with_known_outputs(&topology, &state.known_outputs);
    normalized
        .validate_limits()
        .map_err(CoreError::InvalidTopology)?;
    Ok(normalized)
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
    let topology = bounded_topology_from_backend(backend)?;
    state_store.observe_topology_and_persist_known_outputs(&topology)
}

/// Build a topology by enumerating outputs from a backend and validate topology limits.
///
/// # Errors
/// Returns `CoreError::InvalidTopology` when `validate_limits` fails, or forwards
/// errors from `backend.enumerate_outputs()`.
pub fn bounded_topology_from_backend<B: Backend + ?Sized>(backend: &B) -> CoreResult<Topology> {
    let topology = backend.enumerate_outputs()?;
    topology
        .validate_limits()
        .map_err(CoreError::InvalidTopology)?;
    Ok(topology)
}

/// Load a normalized topology from the backend and plan `profile` for it.
///
/// # Errors
/// Returns an error if topology loading or profile planning fails.
fn plan_profile_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<PlanSnapshot> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = plan_profile_for_topology(profile, &topology)?;
    Ok(PlanSnapshot::new(topology, plan))
}

/// Load a normalized topology from the backend and plan `preset` for it.
///
/// # Errors
/// Returns an error if topology loading or preset planning fails.
fn plan_preset_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&crate::model::OutputIdentity>,
) -> CoreResult<PlanSnapshot> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = Planner::plan_from_preset(preset, &topology, builtin_output, None)?;
    Ok(PlanSnapshot::new(topology, plan))
}

fn validate_with_planner<B, F>(backend: &B, mut plan_snapshot: F) -> CoreResult<ValidationExecution>
where
    B: Backend + ?Sized,
    F: FnMut() -> CoreResult<PlanSnapshot>,
{
    match validate_plan_cycle(backend, plan_snapshot()?)? {
        ExecutionCycle::DryRun {
            validation_plan,
            validation,
        } => Ok(ValidationExecution::Accepted {
            plan: validation_plan,
            validation,
        }),
        ExecutionCycle::Unsupported {
            validation_plan,
            validation,
        } => Ok(ValidationExecution::Unsupported {
            plan: validation_plan,
            validation,
        }),
        ExecutionCycle::Rejected {
            validation_plan,
            validation,
        } => Ok(ValidationExecution::Rejected {
            plan: validation_plan,
            validation,
        }),
        ExecutionCycle::Applied { .. } => {
            unreachable!("validate_plan_cycle never applies a plan")
        }
    }
}

fn apply_with_planner<B, F>(
    backend: &B,
    hooks: &Hooks,
    policy: ApplyPolicy,
    mut plan_snapshot: F,
) -> CoreResult<ApplyExecution>
where
    B: Backend + ?Sized,
    F: FnMut() -> CoreResult<PlanSnapshot>,
{
    let plan_snapshot = plan_snapshot()?;

    if !plan_snapshot.plan.has_enabled_real_outputs() {
        return Ok(ApplyExecution::Rejected {
            plan: plan_snapshot.plan,
            validation: TestResult::rejected(
                Some(ConfigFailureKind::Rejected),
                Some("refusing to apply a layout with no enabled real outputs".to_string()),
            ),
        });
    }

    match apply_plan_cycle(backend, hooks, policy, plan_snapshot)? {
        ExecutionCycle::Applied {
            validation,
            apply_plan,
            apply_result,
            applied_topology,
            ..
        } => {
            if apply_result.success {
                Ok(ApplyExecution::Applied {
                    plan: apply_plan,
                    validation,
                    apply_result,
                    applied_topology,
                })
            } else {
                Ok(ApplyExecution::ApplyFailed {
                    plan: apply_plan,
                    validation,
                    apply_result,
                })
            }
        }
        ExecutionCycle::Unsupported {
            validation_plan,
            validation,
        } => Ok(ApplyExecution::Unsupported {
            plan: validation_plan,
            validation,
        }),
        ExecutionCycle::Rejected {
            validation_plan,
            validation,
        } => Ok(ApplyExecution::Rejected {
            plan: validation_plan,
            validation,
        }),
        ExecutionCycle::DryRun { .. } => unreachable!("apply_plan_cycle never returns DryRun"),
    }
}

/// Plan and validate `profile` against the current backend topology.
///
/// # Errors
/// Returns an error if topology loading, planning, or validation transport fails.
pub fn validate_profile_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<ValidationExecution> {
    validate_with_planner(backend, || {
        plan_profile_with_backend(backend, state_store, profile)
    })
}

/// Plan and validate `preset` against the current backend topology.
///
/// # Errors
/// Returns an error if topology loading, planning, or validation transport fails.
pub fn validate_preset_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&crate::model::OutputIdentity>,
) -> CoreResult<ValidationExecution> {
    validate_with_planner(backend, || {
        plan_preset_with_backend(backend, state_store, preset, builtin_output)
    })
}

/// Plan, validate, and apply `profile` against the current backend topology.
///
/// # Errors
/// Returns an error if topology loading, planning, validation transport, or apply transport fails.
pub fn apply_profile_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<ApplyExecution> {
    apply_profile_workflow_with_policy(backend, state_store, profile, ApplyPolicy::default())
}

pub fn apply_profile_workflow_with_policy<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
    policy: ApplyPolicy,
) -> CoreResult<ApplyExecution> {
    apply_with_planner(backend, &profile.hooks, policy, || {
        plan_profile_with_backend(backend, state_store, profile)
    })
}

/// Plan, validate, and apply `preset` against the current backend topology.
///
/// # Errors
/// Returns an error if topology loading, planning, validation transport, or apply transport fails.
pub fn apply_preset_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&crate::model::OutputIdentity>,
) -> CoreResult<ApplyExecution> {
    apply_preset_workflow_with_policy(
        backend,
        state_store,
        preset,
        builtin_output,
        ApplyPolicy::default(),
    )
}

pub fn apply_preset_workflow_with_policy<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&crate::model::OutputIdentity>,
    policy: ApplyPolicy,
) -> CoreResult<ApplyExecution> {
    let hooks = Hooks::default();
    apply_with_planner(backend, &hooks, policy, || {
        plan_preset_with_backend(backend, state_store, preset, builtin_output)
    })
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
    hook_policy: HookPolicy,
    plan: &LayoutPlan,
) -> CoreResult<ApplyResult> {
    let engine = Engine::new(backend);
    engine.apply_plan(plan, hooks, hook_policy)
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

/// Check if the applied topology matches the intended plan.
///
/// Returns true if all real outputs in the topology match their desired layout
/// in the plan, false otherwise.
#[must_use]
pub fn topology_matches_plan(topology: &Topology, plan: &LayoutPlan) -> bool {
    topology
        .outputs
        .iter()
        .filter(|(_, output)| !output.identity.is_ignored && !output.identity.is_virtual)
        .all(|(name, current)| match plan.outputs.get(name) {
            Some(desired) => desired.same_layout_as(current),
            None => !current.enabled,
        })
}

/// Validate a plan without applying it.
///
/// # Errors
/// Returns an error only if validation transport fails.
fn validate_plan_cycle<B: Backend + ?Sized>(
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
/// Returns an error only if validation transport or apply transport fails.
fn apply_plan_cycle<B: Backend + ?Sized>(
    backend: &B,
    hooks: &Hooks,
    policy: ApplyPolicy,
    plan_snapshot: PlanSnapshot,
) -> CoreResult<ExecutionCycle> {
    let validation = validate_plan(backend, &plan_snapshot.plan)?;

    if validation.status == ValidationStatus::Rejected {
        return Ok(ExecutionCycle::Rejected {
            validation_plan: plan_snapshot.plan,
            validation,
        });
    }

    if validation.status == ValidationStatus::Unsupported && !policy.allow_unsupported_validation {
        return Ok(ExecutionCycle::Unsupported {
            validation_plan: plan_snapshot.plan,
            validation,
        });
    }

    let mut apply_result = apply_plan(backend, hooks, policy.hook_policy, &plan_snapshot.plan)?;
    let applied_topology = match apply_result.applied_state.clone() {
        Some(topology) => topology,
        None => {
            // Re-enumerate to get post-apply topology if backend didn't provide it
            match bounded_topology_from_backend(backend) {
                Ok(topology) => topology,
                Err(err) => {
                    // Propagate enumeration error into apply_result message
                    apply_result.success = false;
                    apply_result.failure = Some(ConfigFailureKind::Rejected);
                    apply_result.message =
                        Some(format!("failed to enumerate topology after apply: {}", err));
                    plan_snapshot.topology.clone()
                }
            }
        }
    };
    let apply_result = match applied_topology.validate_limits() {
        Ok(()) => {
            // Check if the applied topology matches the intended plan
            if apply_result.success
                && !topology_matches_plan(&applied_topology, &plan_snapshot.plan)
            {
                let mut failed = apply_result;
                failed.success = false;
                failed.failure = Some(ConfigFailureKind::Rejected);
                failed.message = Some(
                    "backend reported success but applied topology does not match the intended plan".to_string()
                );
                failed
            } else {
                apply_result
            }
        }
        Err(message) => {
            let mut failed = apply_result;
            failed.success = false;
            failed.failure = Some(ConfigFailureKind::Rejected);
            failed.message = Some(format!(
                "backend returned invalid topology after apply: {message}"
            ));
            failed
        }
    };

    Ok(ExecutionCycle::Applied {
        validation_plan: plan_snapshot.plan.clone(),
        validation,
        apply_plan: plan_snapshot.plan,
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
    topology
        .validate_limits()
        .map_err(CoreError::InvalidTopology)?;
    state_store.update_observed_topology(topology, |state, topology| {
        state.record_applied_profile(profile_name, backend, topology);
        Ok(())
    })
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
    topology
        .validate_limits()
        .map_err(CoreError::InvalidTopology)?;
    state_store.update_observed_topology(topology, |state, topology| {
        state.record_observed_topology(backend, topology);
        Ok(())
    })
}

/// Persist daemon runtime state after observing or applying a topology.
///
/// # Errors
/// Returns an error if topology or state cannot be loaded or saved.
pub fn persist_daemon_runtime_state(
    state_store: &StateStore,
    profile_name: Option<&str>,
    backend: BackendKind,
    topology: &Topology,
) -> CoreResult<()> {
    topology
        .validate_limits()
        .map_err(CoreError::InvalidTopology)?;
    state_store.update_observed_topology(topology, |state, topology| {
        if let Some(profile_name) = profile_name {
            state.record_applied_profile(profile_name, Some(backend), topology);
        } else {
            state.record_observed_topology(Some(backend), topology);
        }
        state.record_daemon_started(backend);
        Ok(())
    })
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
    state_store.update_state(|state| {
        state.set_default_profile_for_setup(setup_fingerprint, profile_name);
        Ok(())
    })
}

/// Persist the display name for a setup fingerprint.
///
/// # Errors
/// Returns an error if the state cannot be loaded or saved.
pub fn set_setup_name_for_setup_in_store(
    state_store: &StateStore,
    setup_fingerprint: &str,
    setup_name: &str,
) -> CoreResult<()> {
    state_store.update_state(|state| {
        state.set_setup_name_for_setup(setup_fingerprint, setup_name);
        Ok(())
    })
}

/// Persist the backend that started the daemon.
///
/// # Errors
/// Returns an error if the state cannot be loaded or saved.
pub fn record_daemon_started_in_store(
    state_store: &StateStore,
    backend: BackendKind,
) -> CoreResult<()> {
    state_store.update_state(|state| {
        state.record_daemon_started(backend);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::OutputWatcher;
    use crate::model::{Mode, OutputIdentity, OutputState, Resolution};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn output(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.identity.make = Some("Test".to_string());
        state.identity.model = Some(connector.to_string());
        state.enabled = true;
        state
    }

    fn profile(name: &str, connector: &str) -> Profile {
        let output = output(connector);
        Profile {
            name: name.to_string(),
            priority: 0,
            match_rules: vec![crate::profile::OutputMatcher {
                identity: output.identity.clone(),
                required: true,
                position_hint: Some(crate::model::Position::default()),
            }],
            layout: HashMap::from([(
                connector.to_string(),
                crate::profile::OutputConfig {
                    state: output,
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
        let mut settings = ProfilesSettings::default();
        settings.set_setup_default_profile(&topology.setup_fingerprint(), "external-only");
        let profiles = vec![profile("both", "DP-1"), profile("external-only", "DP-1")];

        let selected = select_profile_for_topology(&topology, &profiles, &settings).unwrap();

        assert_eq!(selected.name, "external-only");
    }

    #[test]
    fn select_profile_uses_matching_profile_without_fallback() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let settings = ProfilesSettings::default();
        let profiles = vec![profile("desk", "DP-1"), profile("fallback", "HDMI-A-1")];

        let selected = select_profile_for_topology(&topology, &profiles, &settings).unwrap();

        assert_eq!(selected.name, "desk");
    }

    #[test]
    fn select_profile_refuses_connector_only_topology() {
        let mut weak_output = OutputState::new("DP-1");
        weak_output.enabled = true;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), weak_output.clone())]),
        };
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: vec![crate::profile::OutputMatcher {
                identity: weak_output.identity.clone(),
                required: true,
                position_hint: Some(crate::model::Position::default()),
            }],
            layout: HashMap::from([(
                "DP-1".to_string(),
                crate::profile::OutputConfig {
                    state: weak_output,
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
        };

        let selected =
            select_trusted_target_for_topology(&topology, &[profile], &ProfilesSettings::default());

        assert!(selected.is_none());
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
            state
                .remembered_topology_for_setup(&topology.setup_fingerprint())
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
    fn current_profile_name_returns_none_when_recorded_profile_is_stale() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1"), profile("manual", "HDMI-A-1")];
        let mut state = State::default();
        state.last_profile = Some("manual".to_string());

        let selected = current_profile_name(&topology, &profiles, &state);

        assert_eq!(selected, None);
    }

    #[test]
    fn current_profile_name_returns_none_without_recorded_profile() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1")];

        let selected = current_profile_name(&topology, &profiles, &State::default());

        assert_eq!(selected, None);
    }

    #[test]
    fn current_profile_name_rejects_profiles_when_topology_has_extra_real_outputs() {
        let topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), output("DP-1")),
                ("HDMI-A-1".to_string(), output("HDMI-A-1")),
            ]),
        };
        let profiles = vec![profile("desk", "DP-1")];
        let mut state = State::default();
        state.last_profile = Some("desk".to_string());

        assert_eq!(current_profile_name(&topology, &profiles, &state), None);
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
    fn profile_from_topology_persists_derived_scaled_resolution() {
        let mut output = output("DP-1");
        output.mode = Some(Mode::new(2560, 1440, 60));
        output.scale = 1.25;
        output.scaled_resolution = None;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output)]),
        };

        let profile = profile_from_topology("desk", &topology);

        assert_eq!(
            profile.layout["DP-1"].state.scaled_resolution,
            Some(Resolution::new(2048, 1152))
        );
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

    #[test]
    fn plan_profile_for_topology_rejects_extra_real_outputs() {
        let topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), output("DP-1")),
                ("HDMI-A-1".to_string(), output("HDMI-A-1")),
            ]),
        };

        assert!(matches!(
            plan_profile_for_topology(&profile("desk", "DP-1"), &topology),
            Err(CoreError::ProfileMismatch)
        ));
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
            PlanSnapshot::new(
                topology.clone(),
                LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            ),
        )
        .unwrap();

        match cycle {
            ExecutionCycle::DryRun { validation, .. } => {
                assert!(validation.success);
                assert!(validation.is_accepted());
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
            ApplyPolicy::default(),
            PlanSnapshot::new(
                topology.clone(),
                LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            ),
        )
        .unwrap();

        match cycle {
            ExecutionCycle::Applied {
                validation,
                apply_result,
                ..
            } => {
                assert!(validation.success);
                assert!(validation.is_accepted());
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
    fn apply_with_planner_rejects_blank_plan_before_backend_calls() {
        let backend = CycleBackend {
            test_result: TestResult::supported(None),
            test_calls: Arc::new(Mutex::new(0)),
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), {
                let mut state = output("DP-1");
                state.enabled = false;
                state
            })]),
        };
        let hooks = Hooks::default();

        let execution = apply_with_planner(&backend, &hooks, ApplyPolicy::default(), || {
            Ok(PlanSnapshot::new(
                topology.clone(),
                LayoutPlan::new(topology.outputs.clone()),
            ))
        })
        .unwrap();

        match execution {
            ApplyExecution::Rejected { validation, .. } => {
                assert_eq!(validation.status, ValidationStatus::Rejected);
            }
            ApplyExecution::Unsupported { .. }
            | ApplyExecution::ApplyFailed { .. }
            | ApplyExecution::Applied { .. } => panic!("expected blank plan rejection"),
        }
        assert_eq!(*backend.test_calls.lock().unwrap(), 0);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 0);
    }

    #[test]
    fn execute_plan_cycle_blocks_apply_when_validation_is_unsupported() {
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
            ApplyPolicy::default(),
            PlanSnapshot::new(
                topology.clone(),
                LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            ),
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
