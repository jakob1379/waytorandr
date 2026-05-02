use super::{default_assignment_description, ActionOutcome, RemoveOutcome, SaveOutcome};
use crate::commands::output::{
    failure, print_plan_summary, print_validation_result, success, value, warning,
};

pub(super) fn emit_dry_run_text(outcome: &ActionOutcome) {
    println!(
        "{} {}:",
        warning("Dry run for"),
        value(format!(
            "{} '{}'",
            outcome.target_type.as_human(),
            outcome.target
        ))
    );
    print_plan_summary(&outcome.plan);
    if let Some(validation) = &outcome.validation {
        print_validation_result(&Ok(validation.clone()));
    }
    if let Some(saved_profile) = &outcome.saved_profile {
        println!(
            "{} {}",
            warning("Would also save"),
            value(format!("profile '{saved_profile}'"))
        );
    }
    if let Some(description) = default_assignment_description(outcome) {
        println!("{} {}", warning("Would also set"), value(description));
    }
}

pub(super) fn emit_apply_text(outcome: &ActionOutcome) {
    println!(
        "{} {}",
        success("Set"),
        value(format!(
            "{} '{}'",
            outcome.target_type.as_human(),
            outcome.target
        ))
    );
    print_plan_summary(&outcome.plan);
    if let Some(saved_profile) = &outcome.saved_profile {
        println!(
            "{} {}",
            success("Saved"),
            value(format!("profile '{saved_profile}'"))
        );
    }
    if let Some(description) = default_assignment_description(outcome) {
        println!("{} {}", success("Set"), value(description));
    }
}

pub(super) fn emit_save_text(outcome: &SaveOutcome) {
    if outcome.dry_run {
        println!(
            "{} {}:",
            warning("Would save"),
            value(format!("profile '{}'", outcome.profile))
        );
        if let Some(plan) = &outcome.plan {
            print_plan_summary(plan);
        }
        if let Some(setup_name) = &outcome.setup_name {
            println!(
                "{} {}",
                warning("Would also name this setup"),
                value(format!("'{setup_name}'"))
            );
        }
        if outcome.default_scope.is_some() {
            println!(
                "{} {}",
                warning("Would also set"),
                value(format!(
                    "'{}' as the default profile for this setup",
                    outcome.profile
                ))
            );
        }
        return;
    }

    println!(
        "{} {}",
        success("Saved"),
        value(format!("profile '{}'", outcome.profile))
    );
    if let Some(setup_name) = &outcome.setup_name {
        println!(
            "{} {}",
            success("Named this setup"),
            value(format!("'{setup_name}'"))
        );
    }
    if outcome.default_scope.is_some() {
        println!(
            "{} {}",
            success("Set"),
            value(format!(
                "'{}' as the default profile for this setup",
                outcome.profile
            ))
        );
    }
}

pub(super) fn emit_remove_text(outcome: &RemoveOutcome) {
    if outcome.dry_run {
        if outcome.would_remove == Some(true) {
            println!(
                "{} {}",
                warning("Would remove"),
                value(format!("profile '{}'", outcome.profile))
            );
        } else {
            println!(
                "{} {}",
                failure("Profile not found"),
                value(format!("'{}'", outcome.profile))
            );
        }
        return;
    }

    if outcome.removed == Some(true) {
        println!(
            "{} {}",
            success("Removed"),
            value(format!("profile '{}'", outcome.profile))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::action::outcome::{ActionTargetType, DefaultAssignment, DefaultScope};
    use std::collections::HashMap;
    use waytorandr_core::{LayoutPlan, ValidationResult};

    fn outcome(dry_run: bool) -> ActionOutcome {
        ActionOutcome {
            target: "desk".to_string(),
            target_type: ActionTargetType::Profile,
            dry_run,
            plan: LayoutPlan::new(HashMap::new()),
            validation: dry_run.then(|| ValidationResult::supported(None)),
            default_assignment: Some(DefaultAssignment::new("desk", DefaultScope::Setup)),
            saved_profile: Some("desk".to_string()),
            backend_kind: None,
            applied_topology: None,
        }
    }

    #[test]
    fn dry_run_text_emitter_accepts_complete_outcome() {
        emit_dry_run_text(&outcome(true));
    }

    #[test]
    fn apply_text_emitter_accepts_complete_outcome() {
        emit_apply_text(&outcome(false));
    }
}
