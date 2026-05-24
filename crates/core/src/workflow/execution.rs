use super::selection::plan_profile_for_topology;
use super::topology::normalized_topology_from_backend;
use crate::engine::{
    ApplyResult, Backend, ConfigFailureKind, Engine, ValidationResult, ValidationStatus,
};
use crate::error::CoreResult;
use crate::model::{OutputIdentity, Topology, VirtualPreset};
use crate::planning::{LayoutPlan, Planner};
use crate::profile::{Hooks, Profile};
use crate::state::StateStore;
use std::time::Instant;

#[non_exhaustive]
pub struct PreparedPlanApplication {
    plan: LayoutPlan,
}

impl PreparedPlanApplication {
    #[must_use]
    pub fn new(plan: LayoutPlan) -> Self {
        Self { plan }
    }

    #[must_use]
    pub fn plan(&self) -> &LayoutPlan {
        &self.plan
    }
}

pub enum ValidationExecution {
    Accepted {
        plan: LayoutPlan,
        validation: ValidationResult,
    },
    Unsupported {
        plan: LayoutPlan,
        validation: ValidationResult,
    },
    Rejected {
        plan: LayoutPlan,
        validation: ValidationResult,
    },
}

impl ValidationExecution {
    #[must_use]
    pub fn failure_kind(&self) -> Option<ConfigFailureKind> {
        match self {
            Self::Unsupported { validation, .. } | Self::Rejected { validation, .. } => {
                validation.failure()
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
        validation: ValidationResult,
    },
    Rejected {
        plan: LayoutPlan,
        validation: ValidationResult,
    },
    ApplyFailed {
        plan: LayoutPlan,
        validation: ValidationResult,
        apply_result: ApplyResult,
    },
    Applied {
        plan: LayoutPlan,
        validation: ValidationResult,
        apply_result: ApplyResult,
        applied_topology: Topology,
    },
}

impl ApplyExecution {
    #[must_use]
    pub fn failure_kind(&self) -> Option<ConfigFailureKind> {
        match self {
            Self::Rejected { validation, .. } => validation.failure(),
            Self::Unsupported { validation, .. } => validation.failure(),
            Self::ApplyFailed { apply_result, .. } => apply_result.failure(),
            Self::Applied { .. } => None,
        }
    }

    #[must_use]
    pub fn failure_message(&self) -> Option<&str> {
        match self {
            Self::Rejected { validation, .. } => validation.message.as_deref(),
            Self::Unsupported { validation, .. } => validation.message.as_deref(),
            Self::ApplyFailed { apply_result, .. } => apply_result.message(),
            Self::Applied { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApplyPolicy {
    pub allow_unsupported_validation: bool,
}

fn plan_profile_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<PreparedPlanApplication> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    prepare_profile_application(profile, &topology)
}

fn plan_preset_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&OutputIdentity>,
) -> CoreResult<PreparedPlanApplication> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = Planner::plan_from_preset(preset, &topology, builtin_output, None)?;
    Ok(PreparedPlanApplication::new(plan))
}

/// Plan `profile` for a pre-loaded topology.
///
/// # Errors
/// Returns planning errors when the profile cannot be converted into a layout
/// plan for `topology`.
pub fn prepare_profile_application(
    profile: &Profile,
    topology: &Topology,
) -> CoreResult<PreparedPlanApplication> {
    let plan = plan_profile_for_topology(profile, topology)?;
    Ok(PreparedPlanApplication::new(plan))
}

fn validate_with_planner<B, F>(backend: &B, mut plan_snapshot: F) -> CoreResult<ValidationExecution>
where
    B: Backend + ?Sized,
    F: FnMut() -> CoreResult<PreparedPlanApplication>,
{
    validate_plan_cycle(backend, plan_snapshot()?)
}

fn apply_with_planner<B, F>(
    backend: &B,
    hooks: &Hooks,
    policy: ApplyPolicy,
    mut plan_snapshot: F,
) -> CoreResult<ApplyExecution>
where
    B: Backend + ?Sized,
    F: FnMut() -> CoreResult<PreparedPlanApplication>,
{
    apply_plan_cycle(backend, hooks, policy, plan_snapshot()?)
}

/// Plan and validate `profile` against the current backend topology.
///
/// # Errors
/// Returns errors while reading backend topology/state or while planning.
/// Backend validation rejections are returned in `ValidationExecution`.
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
/// Returns errors while reading backend topology/state or while planning.
/// Backend validation rejections are returned in `ValidationExecution`.
pub fn validate_preset_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&OutputIdentity>,
) -> CoreResult<ValidationExecution> {
    validate_with_planner(backend, || {
        plan_preset_with_backend(backend, state_store, preset, builtin_output)
    })
}

/// Plan, validate, and apply `profile` against the current backend topology.
///
/// # Errors
/// Returns errors while reading backend topology/state, planning, or when
/// backend validation/apply transport fails. Validation rejections, backend
/// apply rejections, and hook failures are returned in `ApplyExecution`.
pub fn apply_profile_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<ApplyExecution> {
    apply_profile_workflow_with_policy(backend, state_store, profile, ApplyPolicy::default())
}

/// Plan, validate, and apply `profile` with explicit apply policy.
///
/// # Errors
/// Returns errors while reading backend topology/state, planning, or when
/// backend validation/apply transport fails.
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

/// Validate and apply a precomputed plan.
///
/// # Errors
/// Returns errors when backend validation or apply transport fails. Validation
/// rejections, backend apply rejections, and hook failures are returned in
/// `ApplyExecution`.
pub fn apply_prepared_profile_workflow<B: Backend + ?Sized>(
    backend: &B,
    hooks: &Hooks,
    prepared: PreparedPlanApplication,
) -> CoreResult<ApplyExecution> {
    apply_plan_cycle(backend, hooks, ApplyPolicy::default(), prepared)
}

/// Plan, validate, and apply `preset` against the current backend topology.
///
/// # Errors
/// Returns errors while reading backend topology/state, planning, or when
/// backend validation/apply transport fails. Validation rejections and backend
/// apply rejections are returned in `ApplyExecution`.
pub fn apply_preset_workflow<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&OutputIdentity>,
) -> CoreResult<ApplyExecution> {
    apply_preset_workflow_with_policy(
        backend,
        state_store,
        preset,
        builtin_output,
        ApplyPolicy::default(),
    )
}

/// Plan, validate, and apply `preset` with explicit apply policy.
///
/// # Errors
/// Returns errors while reading backend topology/state, planning, or when
/// backend validation/apply transport fails.
pub fn apply_preset_workflow_with_policy<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: VirtualPreset,
    builtin_output: Option<&OutputIdentity>,
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
/// Returns an error only if backend validation transport fails. Unsupported
/// validation is returned as `ValidationResult::unsupported`.
pub fn validate_plan<B: Backend + ?Sized>(
    backend: &B,
    plan: &LayoutPlan,
) -> CoreResult<ValidationResult> {
    let engine = Engine::new(backend);
    engine.validate_plan(plan)
}

/// Apply a layout plan through the backend.
///
/// # Errors
/// Returns an error only if backend apply transport fails. Backend rejections
/// and hook failures are returned in the `ApplyResult`.
pub fn apply_plan<B: Backend + ?Sized>(
    backend: &B,
    hooks: &Hooks,
    plan: &LayoutPlan,
) -> CoreResult<ApplyResult> {
    let engine = Engine::new(backend);
    engine.apply_plan(plan, hooks)
}

fn invalid_validation_execution(
    validation_plan: LayoutPlan,
    validation: ValidationResult,
) -> ValidationExecution {
    if validation.status == ValidationStatus::Unsupported {
        ValidationExecution::Unsupported {
            plan: validation_plan,
            validation,
        }
    } else {
        ValidationExecution::Rejected {
            plan: validation_plan,
            validation,
        }
    }
}

pub(crate) fn validate_plan_cycle<B: Backend + ?Sized>(
    backend: &B,
    validation_snapshot: PreparedPlanApplication,
) -> CoreResult<ValidationExecution> {
    let validation = validate_plan(backend, &validation_snapshot.plan)?;

    Ok(match validation.status {
        ValidationStatus::Supported => ValidationExecution::Accepted {
            plan: validation_snapshot.plan,
            validation,
        },
        _ => invalid_validation_execution(validation_snapshot.plan, validation),
    })
}

pub(crate) fn apply_plan_cycle<B: Backend + ?Sized>(
    backend: &B,
    hooks: &Hooks,
    policy: ApplyPolicy,
    plan_snapshot: PreparedPlanApplication,
) -> CoreResult<ApplyExecution> {
    let plan_outputs = plan_snapshot.plan.outputs.len();
    let validation_start = Instant::now();
    let validation = validate_plan(backend, &plan_snapshot.plan)?;
    tracing::debug!(
        elapsed_ms = validation_start.elapsed().as_millis(),
        plan_outputs,
        validation_status = ?validation.status,
        failure_kind = validation.failure().map(ConfigFailureKind::as_label).unwrap_or("none"),
        "display plan validation completed"
    );

    if validation.status == ValidationStatus::Rejected {
        return Ok(ApplyExecution::Rejected {
            plan: plan_snapshot.plan,
            validation,
        });
    }
    if validation.status == ValidationStatus::Unsupported && !policy.allow_unsupported_validation {
        return Ok(ApplyExecution::Unsupported {
            plan: plan_snapshot.plan,
            validation,
        });
    }

    let apply_start = Instant::now();
    let apply_result = apply_plan(backend, hooks, &plan_snapshot.plan)?;
    tracing::debug!(
        elapsed_ms = apply_start.elapsed().as_millis(),
        plan_outputs,
        applied = apply_result.is_applied(),
        failure_kind = apply_result
            .failure()
            .map(ConfigFailureKind::as_label)
            .unwrap_or("none"),
        "display plan apply completed"
    );

    let applied_state_start = Instant::now();
    let applied_topology = apply_result
        .applied_state()
        .cloned()
        .unwrap_or_else(|| Topology {
            outputs: plan_snapshot.plan.outputs.clone(),
        });
    tracing::debug!(
        elapsed_ms = applied_state_start.elapsed().as_millis(),
        plan_outputs,
        applied_state_from_backend = apply_result.applied_state().is_some(),
        setup_fingerprint = %applied_topology.setup_fingerprint(),
        state_fingerprint = %applied_topology.state_fingerprint(),
        "display plan applied topology resolved"
    );

    if apply_result.is_applied() {
        Ok(ApplyExecution::Applied {
            plan: plan_snapshot.plan,
            validation,
            apply_result,
            applied_topology,
        })
    } else {
        Ok(ApplyExecution::ApplyFailed {
            plan: plan_snapshot.plan,
            validation,
            apply_result,
        })
    }
}

#[cfg(test)]
mod tests;
