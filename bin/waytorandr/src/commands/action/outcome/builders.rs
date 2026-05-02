use anyhow::{bail, Result};
use waytorandr_core::workflow;
use waytorandr_core::{BackendKind, ConfigFailureKind};

use super::{ActionOutcome, ActionTargetType, DefaultAssignment, DefaultScope};

pub(in crate::commands) fn build_dry_run_outcome(
    target: String,
    target_type: ActionTargetType,
    default_scope: Option<DefaultScope>,
    execution: workflow::ValidationExecution,
) -> Result<ActionOutcome> {
    match execution {
        workflow::ValidationExecution::Accepted { plan, validation }
        | workflow::ValidationExecution::Unsupported { plan, validation } => {
            let default_assignment =
                default_scope.map(|scope| DefaultAssignment::new(target.clone(), scope));
            Ok(ActionOutcome {
                target,
                target_type,
                dry_run: true,
                plan,
                validation: Some(validation),
                default_assignment,
                saved_profile: None,
                backend_kind: None,
                applied_topology: None,
            })
        }
        workflow::ValidationExecution::Rejected { validation, .. } => bail!(
            "{}",
            validation_failure_context(
                &target,
                target_type,
                "dry-run validation rejected",
                validation.failure(),
                validation.message.as_deref(),
                "backend rejected configuration",
            )
        ),
    }
}

fn validation_failure_context(
    target: &str,
    target_type: ActionTargetType,
    outcome: &str,
    failure_kind: Option<ConfigFailureKind>,
    failure_message: Option<&str>,
    fallback_message: &str,
) -> String {
    let failure_kind = failure_kind.map_or("unknown", ConfigFailureKind::as_label);
    let message = failure_message.unwrap_or(fallback_message);
    format!(
        "{} for {} '{}' (failure_kind={failure_kind}): {message}",
        outcome,
        target_type.as_human(),
        target,
    )
}

pub(in crate::commands) fn build_apply_outcome(
    target: String,
    target_type: ActionTargetType,
    default_scope: Option<DefaultScope>,
    backend_kind: BackendKind,
    execution: workflow::ApplyExecution,
) -> Result<ActionOutcome> {
    match execution {
        workflow::ApplyExecution::Applied {
            plan,
            applied_topology,
            ..
        } => {
            let default_assignment =
                default_scope.map(|scope| DefaultAssignment::new(target.clone(), scope));
            Ok(ActionOutcome {
                target,
                target_type,
                dry_run: false,
                plan,
                validation: None,
                default_assignment,
                saved_profile: None,
                backend_kind: Some(backend_kind),
                applied_topology: Some(applied_topology),
            })
        }
        workflow::ApplyExecution::ApplyFailed { apply_result, .. } => bail!(
            "{}",
            apply_failure_context(
                &target,
                target_type,
                backend_kind,
                "apply failed",
                apply_result.failure(),
                apply_result.message(),
                "backend failed to apply configuration",
            )
        ),
        workflow::ApplyExecution::Rejected { validation, .. } => bail!(
            "{}",
            apply_failure_context(
                &target,
                target_type,
                backend_kind,
                "rejected",
                validation.failure(),
                validation.message.as_deref(),
                "backend rejected configuration",
            )
        ),
        workflow::ApplyExecution::Unsupported { validation, .. } => bail!(
            "{}",
            apply_failure_context(
                &target,
                target_type,
                backend_kind,
                "unsupported",
                validation.failure(),
                validation.message.as_deref(),
                "backend validation is unsupported",
            )
        ),
    }
}

fn apply_failure_context(
    target: &str,
    target_type: ActionTargetType,
    backend_kind: BackendKind,
    outcome: &str,
    failure_kind: Option<ConfigFailureKind>,
    failure_message: Option<&str>,
    fallback_message: &str,
) -> String {
    let failure_kind = failure_kind.map_or("unknown", ConfigFailureKind::as_label);
    let message = failure_message.unwrap_or(fallback_message);
    format!(
        "failed to apply {} '{}' with {} backend ({outcome}, failure_kind={failure_kind}): {message}",
        target_type.as_human(),
        target,
        backend_kind,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use waytorandr_core::{LayoutPlan, ValidationResult};

    #[test]
    fn dry_run_outcome_includes_plan_when_validation_is_unsupported() -> Result<()> {
        let outcome = build_dry_run_outcome(
            "desk".to_string(),
            ActionTargetType::Profile,
            None,
            workflow::ValidationExecution::Unsupported {
                plan: LayoutPlan::new(HashMap::new()),
                validation: ValidationResult::unsupported(Some("no dry run".to_string())),
            },
        )?;

        assert!(outcome.dry_run);
        let Some(validation) = outcome.validation else {
            anyhow::bail!("validation should be present");
        };
        assert_eq!(
            validation.status,
            waytorandr_core::ValidationStatus::Unsupported
        );
        Ok(())
    }

    #[test]
    fn default_scope_creates_one_consistent_assignment() -> Result<()> {
        let outcome = build_dry_run_outcome(
            "desk".to_string(),
            ActionTargetType::Profile,
            Some(DefaultScope::Setup),
            workflow::ValidationExecution::Accepted {
                plan: LayoutPlan::new(HashMap::new()),
                validation: ValidationResult::supported(None),
            },
        )?;

        let Some(assignment) = outcome.default_assignment.as_ref() else {
            anyhow::bail!("default assignment should be present");
        };
        assert!(matches!(assignment.scope, DefaultScope::Setup));
        assert_eq!(assignment.target, "desk");
        assert_eq!(
            super::super::default_assignment_description(&outcome).as_deref(),
            Some("'desk' as the default profile for this setup")
        );
        Ok(())
    }

    #[test]
    fn dry_run_rejection_error_includes_target_and_failure_context() {
        let execution = workflow::ValidationExecution::Rejected {
            plan: LayoutPlan::new(HashMap::new()),
            validation: ValidationResult::rejected(
                Some(ConfigFailureKind::Rejected),
                Some("not supported here".to_string()),
            ),
        };

        let Err(err) = build_dry_run_outcome(
            "desk".to_string(),
            ActionTargetType::Profile,
            None,
            execution,
        ) else {
            panic!("validation rejection should return contextual error");
        };
        let message = err.to_string();

        assert!(message.contains("dry-run validation rejected"));
        assert!(message.contains("profile 'desk'"));
        assert!(message.contains("failure_kind=rejected"));
        assert!(message.contains("not supported here"));
    }

    #[test]
    fn apply_failed_error_includes_target_backend_and_failure_context() {
        let execution = workflow::ApplyExecution::ApplyFailed {
            plan: LayoutPlan::new(HashMap::new()),
            validation: ValidationResult::supported(None),
            apply_result: waytorandr_core::ApplyResult::failed(
                Some(ConfigFailureKind::Rejected),
                Some("backend said no".to_string()),
            ),
        };

        let Err(err) = build_apply_outcome(
            "desk".to_string(),
            ActionTargetType::Profile,
            None,
            BackendKind::Test,
            execution,
        ) else {
            panic!("apply failure should return contextual error");
        };
        let message = err.to_string();

        assert!(message.contains("profile 'desk'"));
        assert!(message.contains("test backend"));
        assert!(message.contains("apply failed"));
        assert!(message.contains("failure_kind=rejected"));
        assert!(message.contains("backend said no"));
    }

    #[test]
    fn apply_rejected_error_includes_target_backend_and_failure_context() {
        let execution = workflow::ApplyExecution::Rejected {
            plan: LayoutPlan::new(HashMap::new()),
            validation: ValidationResult::rejected(
                Some(ConfigFailureKind::TopologyChanged),
                Some("topology changed".to_string()),
            ),
        };

        let Err(err) = build_apply_outcome(
            "external".to_string(),
            ActionTargetType::Virtual,
            None,
            BackendKind::Wlroots,
            execution,
        ) else {
            panic!("rejection should return contextual error");
        };
        let message = err.to_string();

        assert!(message.contains("virtual configuration 'external'"));
        assert!(message.contains("wlroots backend"));
        assert!(message.contains("rejected"));
        assert!(message.contains("failure_kind=topology_changed"));
        assert!(message.contains("topology changed"));
    }
}
