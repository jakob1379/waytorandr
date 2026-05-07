use anyhow::{bail, Result};
use serde::Serialize;

use super::output::{
    print_plan_summary, print_validation_result, success, value, warning, write_json,
};
use super::shared::{load_current_topology, plan_outputs, JsonOutputEntry};
use super::OutputMode;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::engine::{
    Backend, ConfigFailureKind, HookPolicy, TestResult, ValidationStatus,
};
use waytorandr_core::error::CoreError;
use waytorandr_core::model::{BackendKind, OutputIdentity, Topology, VirtualPreset};
use waytorandr_core::planner::{topology_is_blank_internal_only, LayoutPlan, Planner};
use waytorandr_core::profile::{Hooks, Profile};
use waytorandr_core::state::StateStore;
use waytorandr_core::store::ProfileStore;
use waytorandr_core::workflow;

fn apply_policy(force: bool) -> workflow::ApplyPolicy {
    workflow::ApplyPolicy {
        allow_unsupported_validation: force,
        ..workflow::ApplyPolicy::default()
    }
}

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
    default_scope: Option<DefaultScope>,
    default_target: Option<String>,
    saved_profile: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum DefaultScope {
    Setup,
}

impl DefaultScope {
    pub(super) const fn as_json(self) -> &'static str {
        match self {
            Self::Setup => "setup",
        }
    }

    fn description(self, target: &str) -> String {
        match self {
            Self::Setup => format!("'{target}' as the default profile for this setup"),
        }
    }
}

impl ActionOutcome {
    pub(super) fn record_saved_profile(&mut self, profile_name: impl Into<String>) {
        self.saved_profile = Some(profile_name.into());
    }

    fn default_description_target(&self) -> &str {
        self.default_target
            .as_deref()
            .unwrap_or(self.target.as_str())
    }

    pub(super) fn set_default_assignment(
        &mut self,
        target: impl Into<String>,
        scope: DefaultScope,
    ) {
        self.default_set = true;
        self.default_scope = Some(scope);
        self.default_target = Some(target.into());
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    default_scope: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_profile: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) default_scope: Option<&'static str>,
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
    default_scope: Option<DefaultScope>,
    execution: workflow::ValidationExecution,
) -> Result<ActionOutcome> {
    match execution {
        workflow::ValidationExecution::Accepted { plan, validation }
        | workflow::ValidationExecution::Unsupported { plan, validation } => {
            let default_target = default_set.then(|| target.clone());
            Ok(ActionOutcome {
                target,
                target_type,
                dry_run: true,
                plan,
                validation: Some(validation),
                default_set,
                default_scope,
                default_target,
                saved_profile: None,
            })
        }
        workflow::ValidationExecution::Rejected { validation, .. } => bail!(
            "{}",
            validation
                .message
                .as_deref()
                .unwrap_or("backend rejected configuration")
        ),
    }
}

fn build_apply_outcome(
    target: String,
    target_type: ActionTargetType,
    default_set: bool,
    default_scope: Option<DefaultScope>,
    execution: workflow::ApplyExecution,
) -> Result<ActionOutcome> {
    match execution {
        workflow::ApplyExecution::Applied {
            plan, validation, ..
        } => {
            let default_target = default_set.then(|| target.clone());
            Ok(ActionOutcome {
                target,
                target_type,
                dry_run: false,
                plan,
                validation: Some(validation),
                default_set,
                default_scope,
                default_target,
                saved_profile: None,
            })
        }
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
    builtin_output: Option<&OutputIdentity>,
    force: bool,
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
            None,
            workflow::validate_preset_workflow(
                backend.as_ref(),
                &state_store,
                preset,
                builtin_output,
            )
            .map_err(anyhow::Error::from)?,
        );
    }

    let execution = workflow::apply_preset_workflow_with_policy(
        backend.as_ref(),
        &state_store,
        preset,
        builtin_output,
        apply_policy(force),
    )
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
        None,
        execution,
    )
}

pub(super) fn execute_builtin_fallback_action(
    dry_run: bool,
    builtin_output: Option<&OutputIdentity>,
    force: bool,
) -> Result<Option<ActionOutcome>> {
    let backend = connect_backend()?;
    let capabilities = backend.capabilities();
    let Some((_topology, plan)) =
        plan_builtin_fallback_from_current(backend.as_ref(), builtin_output)?
    else {
        return Ok(None);
    };
    let validation = workflow::validate_plan(backend.as_ref(), &plan)?;
    if validation.status == ValidationStatus::Rejected {
        bail!(
            "{}",
            validation
                .message
                .as_deref()
                .unwrap_or("backend rejected built-in fallback")
        );
    }

    if dry_run {
        return Ok(Some(ActionOutcome {
            target: VirtualPreset::Builtin.to_string(),
            target_type: ActionTargetType::Virtual,
            dry_run: true,
            plan,
            validation: Some(validation),
            default_set: false,
            default_scope: None,
            default_target: None,
            saved_profile: None,
        }));
    }

    if validation.status == ValidationStatus::Unsupported && !force {
        bail!(
            "{}",
            validation
                .message
                .as_deref()
                .unwrap_or("backend validation is unsupported")
        );
    }

    let Some((latest_topology, latest_plan)) =
        plan_builtin_fallback_from_current(backend.as_ref(), builtin_output)?
    else {
        return Ok(None);
    };
    let latest_validation = workflow::validate_plan(backend.as_ref(), &latest_plan)?;
    if latest_validation.status == ValidationStatus::Rejected {
        bail!(
            "{}",
            latest_validation
                .message
                .as_deref()
                .unwrap_or("backend rejected built-in fallback")
        );
    }
    if latest_validation.status == ValidationStatus::Unsupported && !force {
        bail!(
            "{}",
            latest_validation
                .message
                .as_deref()
                .unwrap_or("backend validation is unsupported")
        );
    }

    let hooks = Hooks::default();
    let apply_result =
        workflow::apply_plan(backend.as_ref(), &hooks, HookPolicy::Disabled, &latest_plan)?;
    if !apply_result.success {
        bail!(
            "{}",
            apply_result
                .message
                .as_deref()
                .unwrap_or("backend failed to apply built-in fallback")
        );
    }
    let applied_topology = workflow::bounded_topology_from_backend(backend.as_ref())?;
    if applied_topology.setup_fingerprint() != latest_topology.setup_fingerprint()
        || !applied_topology.has_enabled_real_outputs()
    {
        bail!("topology changed during built-in fallback apply");
    }
    save_runtime_state(
        VirtualPreset::Builtin.as_str(),
        Some(capabilities.backend),
        &applied_topology,
    )?;

    Ok(Some(ActionOutcome {
        target: VirtualPreset::Builtin.to_string(),
        target_type: ActionTargetType::Virtual,
        dry_run: false,
        plan: latest_plan,
        validation: Some(latest_validation),
        default_set: false,
        default_scope: None,
        default_target: None,
        saved_profile: None,
    }))
}

fn plan_builtin_fallback_from_current(
    backend: &(impl waytorandr_core::engine::Backend + ?Sized),
    builtin_output: Option<&OutputIdentity>,
) -> Result<Option<(waytorandr_core::model::Topology, LayoutPlan)>> {
    let topology = workflow::bounded_topology_from_backend(backend)?;
    if !topology_is_blank_internal_only(&topology, builtin_output) {
        return Ok(None);
    }

    let plan = Planner::plan_from_preset(VirtualPreset::Builtin, &topology, builtin_output, None)?;
    Ok(Some((topology, plan)))
}

pub(super) fn execute_profile_action(
    profile: &Profile,
    dry_run: bool,
    make_default: bool,
    force: bool,
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
            make_default.then_some(DefaultScope::Setup),
            workflow::validate_profile_workflow(backend.as_ref(), &state_store, profile)
                .map_err(anyhow::Error::from)?,
        );
    }

    warn_profile_hooks(profile);
    let execution = workflow::apply_profile_workflow_with_policy(
        backend.as_ref(),
        &state_store,
        profile,
        apply_policy(force),
    )
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
        make_default.then_some(DefaultScope::Setup),
        execution,
    )
}

pub(super) fn execute_trusted_profile_action(
    profile: &Profile,
    dry_run: bool,
    force: bool,
) -> Result<ActionOutcome> {
    validate_profile(profile)?;
    let backend = connect_backend()?;
    let backend_kind = backend.capabilities().backend;
    let Some((_topology, plan)) = plan_trusted_profile_from_current(backend.as_ref(), profile)?
    else {
        bail!("no matching profile configured");
    };
    if !plan.has_enabled_real_outputs() {
        bail!("refusing to apply a layout with no enabled real outputs");
    }

    let validation = workflow::validate_plan(backend.as_ref(), &plan)?;
    if validation.status == ValidationStatus::Rejected {
        bail!(
            "{}",
            validation
                .message
                .as_deref()
                .unwrap_or("backend rejected profile")
        );
    }

    if dry_run {
        return Ok(ActionOutcome {
            target: profile.name.clone(),
            target_type: ActionTargetType::Profile,
            dry_run: true,
            plan,
            validation: Some(validation),
            default_set: false,
            default_scope: None,
            default_target: None,
            saved_profile: None,
        });
    }

    if validation.status == ValidationStatus::Unsupported && !force {
        bail!(
            "{}",
            validation
                .message
                .as_deref()
                .unwrap_or("backend validation is unsupported")
        );
    }

    let Some((latest_topology, latest_plan)) =
        plan_trusted_profile_from_current(backend.as_ref(), profile)?
    else {
        bail!("no matching profile configured");
    };
    if !latest_plan.has_enabled_real_outputs() {
        bail!("refusing to apply a layout with no enabled real outputs");
    }
    let latest_validation = workflow::validate_plan(backend.as_ref(), &latest_plan)?;
    if latest_validation.status == ValidationStatus::Rejected {
        bail!(
            "{}",
            latest_validation
                .message
                .as_deref()
                .unwrap_or("backend rejected profile")
        );
    }
    if latest_validation.status == ValidationStatus::Unsupported && !force {
        bail!(
            "{}",
            latest_validation
                .message
                .as_deref()
                .unwrap_or("backend validation is unsupported")
        );
    }

    warn_profile_hooks(profile);
    let apply_result = workflow::apply_plan(
        backend.as_ref(),
        &profile.hooks,
        HookPolicy::Enabled,
        &latest_plan,
    )?;
    if !apply_result.success {
        bail!(
            "{}",
            apply_result
                .message
                .as_deref()
                .unwrap_or("backend failed to apply profile")
        );
    }

    let applied_topology = workflow::bounded_topology_from_backend(backend.as_ref())?;
    if applied_topology.setup_fingerprint() != latest_topology.setup_fingerprint()
        || !applied_topology.has_enabled_real_outputs()
    {
        bail!("topology changed during profile apply");
    }
    save_runtime_state(&profile.name, Some(backend_kind), &applied_topology)?;

    Ok(ActionOutcome {
        target: profile.name.clone(),
        target_type: ActionTargetType::Profile,
        dry_run: false,
        plan: latest_plan,
        validation: Some(latest_validation),
        default_set: false,
        default_scope: None,
        default_target: None,
        saved_profile: None,
    })
}

fn warn_profile_hooks(profile: &Profile) {
    if profile.has_hooks() {
        eprintln!(
            "{} {}",
            warning("Warning:"),
            value(format!(
                "profile '{}' contains hooks and will execute commands as the current user",
                profile.name
            ))
        );
    }
}

fn plan_trusted_profile_from_current(
    backend: &(impl Backend + ?Sized),
    profile: &Profile,
) -> Result<Option<(Topology, LayoutPlan)>> {
    let topology = workflow::bounded_topology_from_backend(backend)?;
    if !topology.has_strong_setup_identity() {
        return Ok(None);
    }

    match workflow::plan_profile_for_topology(profile, &topology) {
        Ok(plan) => Ok(Some((topology, plan))),
        Err(CoreError::ProfileMismatch) => Ok(None),
        Err(err) => Err(anyhow::Error::from(err)),
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
        .filter(|test| test.status == waytorandr_core::engine::ValidationStatus::Rejected)
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
            default_scope: outcome.default_scope.map(DefaultScope::as_json),
            default_target: outcome.default_target.clone(),
            saved_profile: outcome.saved_profile.clone(),
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
        if let Some(saved_profile) = &outcome.saved_profile {
            println!(
                "{} {}",
                warning("Would also save"),
                value(format!("profile '{saved_profile}'"))
            );
        }
        if outcome.default_set {
            println!(
                "{} {}",
                warning("Would also set"),
                value(
                    outcome
                        .default_scope
                        .expect("default scope should be set when default_set is true")
                        .description(outcome.default_description_target())
                )
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
    if let Some(test) = &outcome.validation {
        print_validation_result(&Ok(test.clone()));
    }
    if let Some(saved_profile) = &outcome.saved_profile {
        println!(
            "{} {}",
            success("Saved"),
            value(format!("profile '{saved_profile}'"))
        );
    }
    if outcome.default_set {
        println!(
            "{} {}",
            success("Set"),
            value(
                outcome
                    .default_scope
                    .expect("default scope should be set when default_set is true")
                    .description(outcome.default_description_target())
            )
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
    let store = ProfileStore::bootstrap()?;
    store
        .set_setup_default_profile(setup_fingerprint, profile_name)
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
    use waytorandr_core::planner::LayoutPlan;
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

    #[test]
    fn dry_run_outcome_includes_plan_when_validation_is_unsupported() {
        let outcome = build_dry_run_outcome(
            "desk".to_string(),
            ActionTargetType::Profile,
            false,
            None,
            workflow::ValidationExecution::Unsupported {
                plan: LayoutPlan::new(HashMap::new()),
                validation: TestResult::unsupported(Some("no dry run".to_string())),
            },
        )
        .expect("unsupported dry-run validation should still show the plan");

        assert!(outcome.dry_run);
        assert_eq!(
            outcome.validation.expect("validation").status,
            waytorandr_core::engine::ValidationStatus::Unsupported
        );
    }
}
