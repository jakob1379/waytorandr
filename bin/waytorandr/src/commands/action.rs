use anyhow::{anyhow, bail, Result};

mod outcome;

use outcome::{build_apply_outcome, build_dry_run_outcome, ActionTargetType};
pub(super) use outcome::{
    emit_action_outcome, emit_remove_outcome, emit_save_outcome, ActionOutcome, DefaultScope,
    RemoveOutcome, SaveOutcome,
};
use waytorandr_core::workflow;
use waytorandr_core::Backend;
use waytorandr_core::Profile;
use waytorandr_core::ProfileStore;
use waytorandr_core::StateStore;
use waytorandr_core::{BackendKind, OutputIdentity, VirtualPreset};

pub(super) fn execute_virtual_action(
    backend: &(impl Backend + ?Sized),
    state_store: &StateStore,
    preset: VirtualPreset,
    dry_run: bool,
    builtin_output: Option<&OutputIdentity>,
    force: bool,
) -> Result<ActionOutcome> {
    let capabilities = backend.capabilities();
    if let Some(message) = capabilities.virtual_preset_unavailable_message(preset) {
        bail!(message);
    }
    let backend_kind = capabilities.backend;

    if dry_run {
        return build_dry_run_outcome(
            preset.to_string(),
            ActionTargetType::Virtual,
            None,
            workflow::validate_preset_workflow(backend, state_store, preset, builtin_output)
                .map_err(anyhow::Error::from)?,
        );
    }

    let execution = workflow::apply_preset_workflow_with_policy(
        backend,
        state_store,
        preset,
        builtin_output,
        workflow::ApplyPolicy {
            allow_unsupported_validation: force,
        },
    )
    .map_err(anyhow::Error::from)?;

    build_apply_outcome(
        preset.to_string(),
        ActionTargetType::Virtual,
        None,
        backend_kind,
        execution,
    )
}

pub(super) fn persist_virtual_action_outcome(
    state_store: &StateStore,
    profile_store: Option<&ProfileStore>,
    outcome: &mut ActionOutcome,
    recorded_profile_name: &str,
    saved_profile_name: Option<&str>,
) -> Result<()> {
    if let Some(saved_profile_name) = saved_profile_name {
        outcome.record_saved_profile(saved_profile_name);
        outcome.set_default_assignment(saved_profile_name, DefaultScope::Setup);
    }

    if outcome.is_dry_run() {
        return Ok(());
    }

    let applied_topology = outcome
        .applied_topology()
        .ok_or_else(|| anyhow!("virtual action completed without applied topology"))?;

    if let Some(saved_profile_name) = saved_profile_name {
        let profile_store =
            profile_store.ok_or_else(|| anyhow!("virtual save requested without profile store"))?;
        let setup_fingerprint = applied_topology.setup_fingerprint();
        let profile = workflow::profile_from_topology(saved_profile_name, applied_topology);

        profile_store.save(&profile, state_store)?;
        set_default_profile_for_fingerprint(profile_store, saved_profile_name, &setup_fingerprint)?;
    }

    persist_applied_profile_runtime_state(
        state_store,
        recorded_profile_name,
        outcome.backend_kind(),
        applied_topology,
    )
}

pub(super) fn execute_profile_action(
    backend: &(impl Backend + ?Sized),
    profile_store: &ProfileStore,
    state_store: &StateStore,
    profile: &Profile,
    dry_run: bool,
    make_default: bool,
    force: bool,
) -> Result<ActionOutcome> {
    validate_profile(profile)?;
    let backend_kind = backend.capabilities().backend;

    if dry_run {
        return build_dry_run_outcome(
            profile.name.clone(),
            ActionTargetType::Profile,
            make_default.then_some(DefaultScope::Setup),
            workflow::validate_profile_workflow(backend, state_store, profile)
                .map_err(anyhow::Error::from)?,
        );
    }

    let execution = workflow::apply_profile_workflow_with_policy(
        backend,
        state_store,
        profile,
        workflow::ApplyPolicy {
            allow_unsupported_validation: force,
        },
    )
    .map_err(anyhow::Error::from)?;

    if let workflow::ApplyExecution::Applied {
        applied_topology, ..
    } = &execution
    {
        persist_applied_profile_runtime_state(
            state_store,
            &profile.name,
            Some(backend_kind),
            applied_topology,
        )?;
        if make_default {
            set_default_profile_for_fingerprint(
                profile_store,
                &profile.name,
                &applied_topology.setup_fingerprint(),
            )?;
        }
    }

    build_apply_outcome(
        profile.name.clone(),
        ActionTargetType::Profile,
        make_default.then_some(DefaultScope::Setup),
        backend_kind,
        execution,
    )
}

pub(super) fn set_default_profile_for_fingerprint(
    store: &ProfileStore,
    profile_name: &str,
    setup_fingerprint: &str,
) -> Result<()> {
    store
        .set_setup_default_profile(setup_fingerprint, profile_name)
        .map_err(anyhow::Error::from)
}

pub(super) fn persist_applied_profile_runtime_state(
    state_store: &StateStore,
    profile_name: &str,
    backend: Option<BackendKind>,
    topology: &waytorandr_core::Topology,
) -> Result<()> {
    workflow::persist_applied_runtime_state(state_store, profile_name, backend, topology)
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
    use waytorandr_core::Profile;

    #[test]
    fn validate_profile_rejects_empty_layout() {
        let profile = Profile::new("desk", 0, Vec::new(), HashMap::new());

        assert!(validate_profile(&profile).is_err());
    }
}
