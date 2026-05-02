use anyhow::Result;
use serde::Serialize;
use waytorandr_core::{ConfigFailureKind, ValidationResult};

use super::{ActionOutcome, RemoveOutcome, SaveOutcome};
use crate::commands::output::write_json;
use crate::commands::shared::{plan_outputs, JsonOutputEntry};

#[derive(Serialize)]
struct JsonValidation {
    status: &'static str,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Serialize)]
struct JsonActionResponse {
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<&'static str>,
    target: String,
    target_type: &'static str,
    dry_run: bool,
    plan: Vec<JsonOutputEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<JsonValidation>,
    #[serde(skip_serializing_if = "is_false")]
    default_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_scope: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_profile: Option<String>,
}

#[derive(Serialize)]
struct JsonSaveResponse {
    command: &'static str,
    profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_name: Option<String>,
    dry_run: bool,
    saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<Vec<JsonOutputEntry>>,
    #[serde(skip_serializing_if = "is_false")]
    default_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_scope: Option<&'static str>,
}

#[derive(Serialize)]
struct JsonRemoveResponse {
    command: &'static str,
    profile: String,
    dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    would_remove: Option<bool>,
}

// Serde skip_serializing_if callbacks are called with field references.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}

fn json_validation(validation: &ValidationResult) -> JsonValidation {
    JsonValidation {
        status: match validation.status {
            waytorandr_core::ValidationStatus::Supported => "ok",
            waytorandr_core::ValidationStatus::Rejected => "failed",
            waytorandr_core::ValidationStatus::Unsupported => "unsupported",
        },
        success: validation.is_accepted(),
        failure: validation.failure().map(ConfigFailureKind::as_label),
        message: validation.message.clone(),
    }
}

pub(super) fn emit_action_json(
    command: &'static str,
    selection: Option<&'static str>,
    outcome: &ActionOutcome,
) -> Result<()> {
    write_json(&JsonActionResponse {
        command,
        selection,
        target: outcome.target.clone(),
        target_type: outcome.target_type.as_json(),
        dry_run: outcome.dry_run,
        plan: plan_outputs(&outcome.plan),
        validation: outcome.validation.as_ref().map(json_validation),
        default_set: outcome.default_assignment.is_some(),
        default_scope: outcome
            .default_assignment
            .as_ref()
            .map(|assignment| assignment.scope.as_json()),
        default_target: outcome
            .default_assignment
            .as_ref()
            .map(|assignment| assignment.target.clone()),
        saved_profile: outcome.saved_profile.clone(),
    })
}

pub(super) fn emit_save_json(outcome: &SaveOutcome) -> Result<()> {
    write_json(&JsonSaveResponse {
        command: "save",
        profile: outcome.profile.clone(),
        setup_name: outcome.setup_name.clone(),
        dry_run: outcome.dry_run,
        saved: outcome.saved,
        plan: outcome.plan.as_ref().map(plan_outputs),
        default_set: outcome.default_scope.is_some(),
        default_scope: outcome.default_scope.map(super::DefaultScope::as_json),
    })
}

pub(super) fn emit_remove_json(outcome: &RemoveOutcome) -> Result<()> {
    write_json(&JsonRemoveResponse {
        command: "remove",
        profile: outcome.profile.clone(),
        dry_run: outcome.dry_run,
        removed: outcome.removed,
        would_remove: outcome.would_remove,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_validation_maps_failure_kind_label() {
        let validation_result = ValidationResult::rejected(
            Some(ConfigFailureKind::TopologyChanged),
            Some("changed".to_string()),
        );

        let validation = json_validation(&validation_result);

        assert!(!validation.success);
        assert_eq!(validation.status, "failed");
        assert_eq!(validation.failure, Some("topology_changed"));
        assert_eq!(validation.message.as_deref(), Some("changed"));
    }

    #[test]
    fn json_validation_marks_unsupported() {
        let validation_result = ValidationResult::unsupported(Some("no validation".to_string()));

        let validation = json_validation(&validation_result);

        assert!(!validation.success);
        assert_eq!(validation.status, "unsupported");
    }
}
