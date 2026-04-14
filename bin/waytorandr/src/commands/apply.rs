use anyhow::{bail, Result};
use serde::Serialize;

use super::output::{print_plan_summary, print_validation_result, write_json};
use super::shared::{load_current_topology, plan_outputs, JsonOutputEntry};
use super::OutputMode;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::engine::{ConfigFailureKind, TestResult};
use waytorandr_core::model::{BackendKind, VirtualPreset};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::{Hooks, Profile};
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
        success: test.is_supported(),
        failure: test.failure.map(failure_kind_label),
        message: test.message.clone(),
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
    let hooks = Hooks::default();
    let state_store = StateStore::bootstrap()?;
    let backend_kind = capabilities.backend;
    let validation_snapshot =
        workflow::plan_preset_with_backend(backend.as_ref(), &state_store, preset)
            .map_err(anyhow::Error::from)?;
    if dry_run {
        match workflow::validate_plan_cycle(backend.as_ref(), validation_snapshot)
            .map_err(anyhow::Error::from)?
        {
            workflow::ExecutionCycle::DryRun {
                validation_plan,
                validation,
            } => Ok(ActionOutcome {
                target: preset.to_string(),
                target_type: ActionTargetType::Virtual,
                dry_run: true,
                plan: validation_plan,
                validation: Some(validation),
                default_set: false,
            }),
            workflow::ExecutionCycle::Unsupported { validation, .. }
            | workflow::ExecutionCycle::Rejected { validation, .. } => bail!(validation
                .message
                .unwrap_or_else(|| "backend rejected configuration".to_string())),
            workflow::ExecutionCycle::Applied { .. } => unreachable!(),
        }
    } else {
        let apply_snapshot =
            workflow::plan_preset_with_backend(backend.as_ref(), &state_store, preset)
                .map_err(anyhow::Error::from)?;
        match workflow::apply_plan_cycle(
            backend.as_ref(),
            &hooks,
            validation_snapshot,
            apply_snapshot,
        )
        .map_err(anyhow::Error::from)?
        {
            workflow::ExecutionCycle::Applied {
                apply_plan,
                apply_result,
                applied_topology,
                ..
            } => {
                if !apply_result.success {
                    bail!(apply_result
                        .message
                        .unwrap_or_else(|| "backend failed to apply configuration".to_string()));
                }

                save_runtime_state(preset.as_str(), Some(backend_kind), &applied_topology)?;

                Ok(ActionOutcome {
                    target: preset.to_string(),
                    target_type: ActionTargetType::Virtual,
                    dry_run: false,
                    plan: apply_plan,
                    validation: None,
                    default_set: false,
                })
            }
            workflow::ExecutionCycle::Unsupported { validation, .. }
            | workflow::ExecutionCycle::Rejected { validation, .. } => bail!(validation
                .message
                .unwrap_or_else(|| "backend rejected configuration".to_string())),
            workflow::ExecutionCycle::DryRun { .. } => unreachable!(),
        }
    }
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
    let validation_snapshot =
        workflow::plan_profile_with_backend(backend.as_ref(), &state_store, profile)
            .map_err(anyhow::Error::from)?;
    if dry_run {
        match workflow::validate_plan_cycle(backend.as_ref(), validation_snapshot)
            .map_err(anyhow::Error::from)?
        {
            workflow::ExecutionCycle::DryRun {
                validation_plan,
                validation,
            } => Ok(ActionOutcome {
                target: profile.name.clone(),
                target_type: ActionTargetType::Profile,
                dry_run: true,
                plan: validation_plan,
                validation: Some(validation),
                default_set: make_default,
            }),
            workflow::ExecutionCycle::Unsupported { validation, .. }
            | workflow::ExecutionCycle::Rejected { validation, .. } => bail!(validation
                .message
                .unwrap_or_else(|| "backend rejected configuration".to_string())),
            workflow::ExecutionCycle::Applied { .. } => unreachable!(),
        }
    } else {
        let apply_snapshot =
            workflow::plan_profile_with_backend(backend.as_ref(), &state_store, profile)
                .map_err(anyhow::Error::from)?;
        match workflow::apply_plan_cycle(
            backend.as_ref(),
            &profile.hooks,
            validation_snapshot,
            apply_snapshot,
        )
        .map_err(anyhow::Error::from)?
        {
            workflow::ExecutionCycle::Applied {
                apply_plan,
                apply_result,
                applied_topology,
                ..
            } => {
                if !apply_result.success {
                    bail!(apply_result
                        .message
                        .unwrap_or_else(|| "backend failed to apply configuration".to_string()));
                }

                save_runtime_state(&profile.name, Some(backend_kind), &applied_topology)?;
                if make_default {
                    set_default_profile_for_fingerprint(
                        &profile.name,
                        &applied_topology.setup_fingerprint(),
                    )?;
                }

                Ok(ActionOutcome {
                    target: profile.name.clone(),
                    target_type: ActionTargetType::Profile,
                    dry_run: false,
                    plan: apply_plan,
                    validation: None,
                    default_set: make_default,
                })
            }
            workflow::ExecutionCycle::Unsupported { validation, .. }
            | workflow::ExecutionCycle::Rejected { validation, .. } => bail!(validation
                .message
                .unwrap_or_else(|| "backend rejected configuration".to_string())),
            workflow::ExecutionCycle::DryRun { .. } => unreachable!(),
        }
    }
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
        .filter(|test| !test.is_supported())
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
            "Dry run for {target_type} '{target}':",
            target_type = outcome.target_type.as_human(),
            target = outcome.target
        );
        print_plan_summary(&outcome.plan);
        if let Some(test) = &outcome.validation {
            print_validation_result(&Ok(test.clone()));
        }
        if outcome.default_set {
            println!(
                "Would also set '{}' as the default profile for this fingerprint",
                outcome.target
            );
        }
        if let Some(message) = validation_failure {
            bail!(message);
        }
        return Ok(());
    }

    println!(
        "Set {target_type} '{target}'",
        target_type = outcome.target_type.as_human(),
        target = outcome.target
    );
    print_plan_summary(&outcome.plan);
    if outcome.default_set {
        println!(
            "Set '{}' as the default profile for this fingerprint",
            outcome.target
        );
    }
    Ok(())
}

pub(super) fn current_setup_fingerprint() -> Result<Option<String>> {
    let state_store = StateStore::bootstrap()?;
    load_current_topology(&state_store).map(|topology| Some(topology.setup_fingerprint()))
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
