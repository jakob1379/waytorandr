use anyhow::{bail, Result};
use serde::Serialize;

use super::output::{
    print_plan_summary, print_validation_result, success, value, warning, write_json,
};
use super::shared::{load_current_topology, plan_outputs, JsonOutputEntry};
use super::OutputMode;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::engine::{ConfigFailureKind, TestResult};
use waytorandr_core::model::{BackendKind, VirtualPreset};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::Profile;
use waytorandr_core::state::StateStore;
use waytorandr_core::workflow;

#[derive(Clone, Copy)]
pub(super) enum ActionTargetType {
    Profile,
    Virtual,
}

impl ActionTargetType {
    const fn as_json(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Virtual => "virtual",
        }
    }

    const fn as_human(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Virtual => "virtual configuration",
        }
    }
}

pub(super) struct ActionOutcome {
    target: String,
    target_type: ActionTargetType,
    dry_run: bool,
    plan: LayoutPlan,
    validation: Option<TestResult>,
    default_set: bool,
}

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
}

#[derive(Serialize)]
pub(super) struct JsonSaveResponse {
    pub(super) command: &'static str,
    pub(super) profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) setup_name: Option<String>,
    pub(super) dry_run: bool,
    pub(super) saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) plan: Option<Vec<JsonOutputEntry>>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) default_set: bool,
}

#[derive(Serialize)]
pub(super) struct JsonRemoveResponse {
    pub(super) command: &'static str,
    pub(super) profile: String,
    pub(super) dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) removed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) would_remove: Option<bool>,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

const fn failure_kind_label(kind: ConfigFailureKind) -> &'static str {
    match kind {
        ConfigFailureKind::Rejected => "rejected",
        ConfigFailureKind::TopologyChanged => "topology_changed",
    }
}

fn json_validation(test: &TestResult) -> JsonValidation {
    JsonValidation {
        status: match test.status {
            waytorandr_core::engine::ValidationStatus::Supported => "ok",
            waytorandr_core::engine::ValidationStatus::Rejected => "failed",
            waytorandr_core::engine::ValidationStatus::Unsupported => "unsupported",
        },
        success: test.is_accepted(),
        failure: test.failure.map(failure_kind_label),
        message: test.message.clone(),
    }
}

fn build_dry_run_outcome(
    target: String,
    target_type: ActionTargetType,
    default_set: bool,
    execution: workflow::ValidationExecution,
) -> Result<ActionOutcome> {
    match execution {
        workflow::ValidationExecution::Accepted { plan, validation } => Ok(ActionOutcome {
            target,
            target_type,
            dry_run: true,
            plan,
            validation: Some(validation),
            default_set,
        }),
        other => bail!(
            "{}",
            other
                .failure_message()
                .unwrap_or("backend rejected configuration")
        ),
    }
}

fn build_apply_outcome(
    target: String,
    target_type: ActionTargetType,
    default_set: bool,
    execution: workflow::ApplyExecution,
) -> Result<ActionOutcome> {
    match execution {
        workflow::ApplyExecution::Applied { plan, .. } => Ok(ActionOutcome {
            target,
            target_type,
            dry_run: false,
            plan,
            validation: None,
            default_set,
        }),
        workflow::ApplyExecution::ApplyFailed { .. } => bail!(
            "{}",
            execution
                .failure_message()
                .unwrap_or("backend failed to apply configuration")
        ),
        other => bail!(
            "{}",
            other
                .failure_message()
                .unwrap_or("backend rejected configuration")
        ),
    }
}

pub(super) fn execute_virtual_action(
    preset: VirtualPreset,
    dry_run: bool,
) -> Result<ActionOutcome> {
    let backend = connect_backend()?;
    let capabilities = backend.capabilities();
    if let Some(message) = capabilities.virtual_preset_unavailable_message(preset) {
        bail!(message);
    }
    let state_store = StateStore::bootstrap()?;
    let backend_kind = capabilities.backend;

    if dry_run {
        return build_dry_run_outcome(
            preset.to_string(),
            ActionTargetType::Virtual,
            false,
            workflow::validate_preset_workflow(backend.as_ref(), &state_store, preset)
                .map_err(anyhow::Error::from)?,
        );
    }

    let execution = workflow::apply_preset_workflow(backend.as_ref(), &state_store, preset)
        .map_err(anyhow::Error::from)?;

    if let workflow::ApplyExecution::Applied {
        applied_topology, ..
    } = &execution
    {
        save_runtime_state(preset.as_str(), Some(backend_kind), applied_topology)?;
    }

    build_apply_outcome(
        preset.to_string(),
        ActionTargetType::Virtual,
        false,
        execution,
    )
}

pub(super) fn execute_profile_action(
    profile: &Profile,
    dry_run: bool,
    make_default: bool,
) -> Result<ActionOutcome> {
    validate_profile(profile)?;
    let backend = connect_backend()?;
    let state_store = StateStore::bootstrap()?;
    let backend_kind = backend.capabilities().backend;

    if dry_run {
        return build_dry_run_outcome(
            profile.name.clone(),
            ActionTargetType::Profile,
            make_default,
            workflow::validate_profile_workflow(backend.as_ref(), &state_store, profile)
                .map_err(anyhow::Error::from)?,
        );
    }

    let execution = workflow::apply_profile_workflow(backend.as_ref(), &state_store, profile)
        .map_err(anyhow::Error::from)?;

    if let workflow::ApplyExecution::Applied {
        applied_topology, ..
    } = &execution
    {
        save_runtime_state(&profile.name, Some(backend_kind), applied_topology)?;
        if make_default {
            set_default_profile_for_fingerprint(
                &profile.name,
                &applied_topology.setup_fingerprint(),
            )?;
        }
    }

    build_apply_outcome(
        profile.name.clone(),
        ActionTargetType::Profile,
        make_default,
        execution,
    )
}

pub(super) fn emit_action_outcome(
    command: &'static str,
    selection: Option<&'static str>,
    outcome: &ActionOutcome,
    output_mode: OutputMode,
) -> Result<()> {
    let validation_failure = outcome
        .validation
        .as_ref()
        .filter(|test| !test.is_accepted())
        .map(|test| {
            test.message
                .clone()
                .unwrap_or_else(|| "backend rejected configuration".to_string())
        });

    if output_mode.is_json() {
        write_json(&JsonActionResponse {
            command,
            selection,
            target: outcome.target.clone(),
            target_type: outcome.target_type.as_json(),
            dry_run: outcome.dry_run,
            plan: plan_outputs(&outcome.plan),
            validation: outcome.validation.as_ref().map(json_validation),
            default_set: outcome.default_set,
        })?;
        if let Some(message) = validation_failure {
            bail!(message);
        }
        return Ok(());
    }

    if outcome.dry_run {
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
        if let Some(test) = &outcome.validation {
            print_validation_result(&Ok(test.clone()));
        }
        if outcome.default_set {
            println!(
                "{} {}",
                warning("Would also set"),
                value(format!(
                    "'{}' as the default profile for this setup",
                    outcome.target
                ))
            );
        }
        if let Some(message) = validation_failure {
            bail!(message);
        }
        return Ok(());
    }

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
    if outcome.default_set {
        println!(
            "{} {}",
            success("Set"),
            value(format!(
                "'{}' as the default profile for this setup",
                outcome.target
            ))
        );
    }
    Ok(())
}

pub(super) fn current_setup_fingerprint() -> Result<String> {
    let state_store = StateStore::bootstrap()?;
    load_current_topology(&state_store).map(|topology| topology.setup_fingerprint())
}

pub(super) fn set_default_profile_for_fingerprint(
    profile_name: &str,
    setup_fingerprint: &str,
) -> Result<()> {
    let state_store = StateStore::bootstrap()?;
    workflow::set_default_profile_for_setup_in_store(&state_store, setup_fingerprint, profile_name)
        .map_err(anyhow::Error::from)
}

pub(super) fn save_runtime_state(
    profile_name: &str,
    backend: Option<BackendKind>,
    topology: &waytorandr_core::model::Topology,
) -> Result<()> {
    let state_store = StateStore::bootstrap()?;
    workflow::persist_applied_runtime_state(&state_store, profile_name, backend, topology)
        .map_err(anyhow::Error::from)
}

fn validate_profile(profile: &Profile) -> Result<()> {
    if profile.layout.is_empty() {
        bail!(
            "profile '{name}' contains no saved layout; re-save it with 'waytorandr save {name}' before setting it",
            name = profile.name.as_str()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use waytorandr_core::engine::{ConfigFailureKind, TestResult};
    use waytorandr_core::profile::Profile;

    #[test]
    fn validate_profile_rejects_empty_layout() {
        let profile = Profile::new("desk", 0, Vec::new(), HashMap::new());

        assert!(validate_profile(&profile).is_err());
    }

    #[test]
    fn json_validation_maps_failure_kind_label() {
        let test = TestResult::rejected(
            Some(ConfigFailureKind::TopologyChanged),
            Some("changed".to_string()),
        );

        let validation = json_validation(&test);

        assert!(!validation.success);
        assert_eq!(validation.status, "failed");
        assert_eq!(validation.failure, Some("topology_changed"));
        assert_eq!(validation.message.as_deref(), Some("changed"));
    }

    #[test]
    fn json_validation_marks_unsupported() {
        let test = TestResult::unsupported(Some("no test mode".to_string()));

        let validation = json_validation(&test);

        assert!(!validation.success);
        assert_eq!(validation.status, "unsupported");
    }
}
