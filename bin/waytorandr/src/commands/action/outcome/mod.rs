use anyhow::{bail, Result};

use super::super::OutputMode;

mod builders;
mod json;
mod model;
mod text;

pub(in crate::commands) use builders::{build_apply_outcome, build_dry_run_outcome};
pub(in crate::commands) use model::{
    ActionOutcome, ActionTargetType, DefaultAssignment, DefaultScope, RemoveOutcome, SaveOutcome,
};

pub(in crate::commands) fn emit_remove_outcome(
    outcome: &RemoveOutcome,
    output_mode: OutputMode,
) -> Result<()> {
    if output_mode.is_json() {
        json::emit_remove_json(outcome)
    } else {
        text::emit_remove_text(outcome);
        Ok(())
    }
}

pub(in crate::commands) fn emit_save_outcome(
    outcome: &SaveOutcome,
    output_mode: OutputMode,
) -> Result<()> {
    if output_mode.is_json() {
        json::emit_save_json(outcome)
    } else {
        text::emit_save_text(outcome);
        Ok(())
    }
}

pub(in crate::commands) fn emit_action_outcome(
    command: &'static str,
    selection: Option<&'static str>,
    outcome: &ActionOutcome,
    output_mode: OutputMode,
) -> Result<()> {
    let validation_failure = outcome
        .validation
        .as_ref()
        .filter(|validation| validation.status == waytorandr_core::ValidationStatus::Rejected)
        .map(|validation| {
            validation
                .message
                .clone()
                .unwrap_or_else(|| "backend rejected configuration".to_string())
        });

    if output_mode.is_json() {
        json::emit_action_json(command, selection, outcome)?;
    } else if outcome.dry_run {
        text::emit_dry_run_text(outcome);
    } else {
        text::emit_apply_text(outcome);
    }

    if let Some(message) = validation_failure {
        bail!(message);
    }

    Ok(())
}

fn default_assignment_description(outcome: &ActionOutcome) -> Option<String> {
    outcome
        .default_assignment
        .as_ref()
        .map(DefaultAssignment::description)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use waytorandr_core::LayoutPlan;

    use super::{
        default_assignment_description, ActionOutcome, ActionTargetType, DefaultAssignment,
        DefaultScope,
    };

    #[test]
    fn default_assignment_description_formats_present_assignment() {
        let outcome = ActionOutcome {
            target: "docked".to_string(),
            target_type: ActionTargetType::Profile,
            dry_run: true,
            plan: LayoutPlan::new(HashMap::new()),
            validation: None,
            default_assignment: Some(DefaultAssignment::new("docked", DefaultScope::Setup)),
            saved_profile: None,
            backend_kind: None,
            applied_topology: None,
        };

        assert_eq!(
            default_assignment_description(&outcome).as_deref(),
            Some("'docked' as the default profile for this setup")
        );
    }

    #[test]
    fn default_assignment_description_returns_none_without_assignment() {
        let outcome = ActionOutcome {
            target: "external".to_string(),
            target_type: ActionTargetType::Virtual,
            dry_run: true,
            plan: LayoutPlan::new(HashMap::new()),
            validation: None,
            default_assignment: None,
            saved_profile: None,
            backend_kind: None,
            applied_topology: None,
        };

        assert_eq!(default_assignment_description(&outcome), None);
    }
}
