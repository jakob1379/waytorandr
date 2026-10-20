use anyhow::{anyhow, bail, Result};

use super::action::{
    emit_action_outcome, emit_remove_outcome, emit_save_outcome, execute_profile_action,
    execute_virtual_action, known_setup_fingerprint, persist_virtual_action_outcome,
    set_default_profile_for_fingerprint, DefaultScope, RemoveOutcome, SaveOutcome,
};
use super::shared::load_current_topology;
use super::OutputMode;
use crate::notify::notify;
use crate::preset::resolve_virtual_preset;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::detect_preset;
use waytorandr_core::validate_profile_name;
use waytorandr_core::workflow;
use waytorandr_core::LayoutPlan;
use waytorandr_core::Profile;
use waytorandr_core::ProfileStore;
use waytorandr_core::Topology;
use waytorandr_core::{ProfileQueryContext, State, StateStore, VirtualPreset};

const DEFAULT_SAVED_PROFILE_NAME: &str = "default";
const CUSTOM_SAVED_PROFILE_NAME: &str = "custom";
const AUTO_SET_TARGET: &str = "auto";

// SetOptions mirrors the independent set/cycle/save flags carried through workflows.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub(super) struct SetOptions {
    pub(super) dry_run: bool,
    pub(super) make_default: bool,
    pub(super) save: bool,
    pub(super) reverse: bool,
    pub(super) force: bool,
}

fn save_layout_for_topology(
    store: &ProfileStore,
    state_store: &StateStore,
    name: &str,
    setup_name: Option<&str>,
    make_default: bool,
    topology: &Topology,
) -> Result<()> {
    validate_cli_profile_name(name)?;
    let setup_fingerprint = topology.setup_fingerprint();
    let profile = workflow::profile_from_topology(name, topology);

    store.save(&profile, state_store)?;
    if let Some(setup_name) = setup_name {
        workflow::set_setup_name_for_setup_in_store(state_store, &setup_fingerprint, setup_name)?;
    }
    if make_default {
        set_default_profile_for_fingerprint(store, name, &setup_fingerprint)?;
    }

    Ok(())
}

pub(super) fn cmd_save(
    name: Option<&str>,
    setup_name: Option<&str>,
    dry_run: bool,
    make_default: bool,
    output_mode: OutputMode,
) -> Result<()> {
    if let Some(name) = name {
        validate_cli_profile_name(name)?;
    }
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let topology = workflow::normalized_topology_from_backend(backend.as_ref(), &state_store)?;

    if topology.outputs.is_empty() {
        bail!("cannot save a profile from an empty topology")
    }

    let name = match name {
        Some(name) => name,
        None if make_default => DEFAULT_SAVED_PROFILE_NAME,
        None => detect_preset(&topology).map_or(CUSTOM_SAVED_PROFILE_NAME, VirtualPreset::as_str),
    };
    let make_default = make_default
        || is_implicit_default_for_setup(name, &topology.setup_fingerprint(), &state_store)?;

    let profile = workflow::profile_from_topology(name, &topology);

    if dry_run {
        let plan = LayoutPlan::new(
            profile
                .layout
                .iter()
                .map(|(output_name, config)| (output_name.clone(), config.state.clone()))
                .collect(),
        );
        let outcome = SaveOutcome::dry_run(
            name,
            setup_name,
            plan,
            make_default.then_some(DefaultScope::Setup),
        );
        return emit_save_outcome(&outcome, output_mode);
    }

    let store = ProfileStore::bootstrap()?;
    save_layout_for_topology(
        &store,
        &state_store,
        name,
        setup_name,
        make_default,
        &topology,
    )?;
    let outcome = SaveOutcome::saved(
        name,
        setup_name,
        make_default.then_some(DefaultScope::Setup),
    );
    emit_save_outcome(&outcome, output_mode)
}

fn is_implicit_default_for_setup(
    name: &str,
    setup_fingerprint: &str,
    state_store: &StateStore,
) -> Result<bool> {
    let store = ProfileStore::open_read_only()?;
    if store
        .settings()?
        .setup_default_layout(setup_fingerprint)
        .is_some()
    {
        return Ok(false);
    }
    Ok(store
        .profiles_for_setup(setup_fingerprint, &ProfileQueryContext::load(state_store)?)?
        .iter()
        .all(|profile| profile.name == name))
}

fn validate_cli_profile_name(name: &str) -> Result<()> {
    validate_profile_name(name).map_err(|reason| anyhow!("invalid profile name '{name}': {reason}"))
}

pub(super) fn cmd_set(
    target: Option<&str>,
    forced_profile: Option<&str>,
    options: SetOptions,
    output_mode: OutputMode,
) -> Result<()> {
    let SetOptions {
        dry_run,
        make_default,
        save,
        reverse,
        force,
    } = options;

    let (name, force_saved_profile) = match (target, forced_profile) {
        (Some(name), None) => (name, false),
        (None, Some(name)) => (name, true),
        _ => bail!("set requires either a target or --profile"),
    };

    if !force_saved_profile && name == AUTO_SET_TARGET {
        return handle_auto_set(reverse, make_default || save, dry_run, force, output_mode);
    }

    if !force_saved_profile {
        if let Some(preset) = resolve_virtual_preset(name, reverse)? {
            return handle_virtual_set(preset, options, output_mode);
        }
    }

    if save {
        bail!("--save can only be used with virtual set targets")
    }

    handle_profile_set(name, dry_run, make_default, force, output_mode)
}

fn handle_auto_set(
    reverse: bool,
    save_for_current_setup: bool,
    dry_run: bool,
    force: bool,
    output_mode: OutputMode,
) -> Result<()> {
    if reverse {
        bail!("--reverse cannot be used with `waytorandr set auto`")
    }
    if save_for_current_setup {
        bail!("--default and --save cannot be used with `waytorandr set auto`")
    }
    apply_auto_profile_selection(dry_run, force, output_mode)
}

fn handle_virtual_set(
    preset: waytorandr_core::VirtualPreset,
    options: SetOptions,
    output_mode: OutputMode,
) -> Result<()> {
    let SetOptions {
        dry_run,
        make_default,
        save,
        force,
        ..
    } = options;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let saved_profile_name = save.then_some(preset.as_str());
    let default_layout = make_default && !save;
    let recorded_profile_name = saved_profile_name.unwrap_or(preset.as_str());
    let profile_store = if (save || make_default) && !dry_run {
        Some(ProfileStore::bootstrap()?)
    } else {
        None
    };
    let assign_default = match saved_profile_name {
        Some(name) if !make_default => is_implicit_default_for_setup(
            name,
            &load_current_topology(backend.as_ref(), &state_store)?.setup_fingerprint(),
            &state_store,
        )?,
        _ => make_default,
    };
    let settings = ProfileStore::open_read_only()?.settings()?;
    let mut outcome = execute_virtual_action(
        backend.as_ref(),
        &state_store,
        preset,
        dry_run,
        settings.builtin_output.as_ref(),
        force,
    )?;

    persist_virtual_action_outcome(
        &state_store,
        profile_store.as_ref(),
        &mut outcome,
        recorded_profile_name,
        saved_profile_name,
        assign_default,
    )?;
    if default_layout {
        outcome.set_default_assignment(preset.as_str(), DefaultScope::Setup);
        if let (Some(store), Some(topology)) = (profile_store.as_ref(), outcome.applied_topology())
        {
            store.set_setup_default_layout(
                &known_setup_fingerprint(&state_store, topology)?,
                preset,
            )?;
        }
    }

    emit_action_outcome("set", Some("explicit"), &outcome, output_mode)
}

fn handle_profile_set(
    name: &str,
    dry_run: bool,
    make_default: bool,
    force: bool,
    output_mode: OutputMode,
) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let setup_fingerprint =
        load_current_topology(backend.as_ref(), &state_store)?.setup_fingerprint();
    let query_context = ProfileQueryContext::load(&state_store)?;
    let profile = store
        .get_for_setup(name, &setup_fingerprint, &query_context)?
        .ok_or_else(|| anyhow!("profile '{name}' not found"))?;
    let outcome = execute_profile_action(
        backend.as_ref(),
        &store,
        &state_store,
        &profile.profile,
        dry_run,
        make_default,
        force,
    )?;
    emit_action_outcome("set", Some("explicit"), &outcome, output_mode)
}

fn apply_auto_profile_selection(dry_run: bool, force: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let topology = load_current_topology(backend.as_ref(), &state_store)?;
    let query_context = ProfileQueryContext::load(&state_store)?;
    let profiles = store.profiles(&query_context)?;
    let settings = store.settings()?;
    let decision = workflow::select_profile_application_target(
        &topology,
        &profiles,
        &settings,
        &State::default(),
    );
    if let workflow::ProfileSelectionDecision::SetupDefaultLayout(preset) = decision {
        let mut outcome = execute_virtual_action(
            backend.as_ref(),
            &state_store,
            preset,
            dry_run,
            settings.builtin_output.as_ref(),
            force,
        )?;
        persist_virtual_action_outcome(
            &state_store,
            None,
            &mut outcome,
            preset.as_str(),
            None,
            false,
        )?;
        return emit_action_outcome("set", Some("auto"), &outcome, output_mode);
    }
    let profile = decision
        .selected_profile()
        .cloned()
        .ok_or_else(|| anyhow!("no matching profile configured"))?;

    let outcome = execute_profile_action(
        backend.as_ref(),
        &store,
        &state_store,
        &profile,
        dry_run,
        false,
        force,
    )?;

    emit_action_outcome("set", Some("auto"), &outcome, output_mode)
}

pub(super) fn cmd_remove(name: &str, dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let setup_fingerprint =
        load_current_topology(backend.as_ref(), &state_store)?.setup_fingerprint();
    let query_context = ProfileQueryContext::load(&state_store)?;
    let exists = store
        .get_for_setup(name, &setup_fingerprint, &query_context)?
        .is_some();
    let clears_default_layout = !exists
        && store
            .settings()?
            .setup_default_layout(&setup_fingerprint)
            .is_some_and(|preset| preset.as_str() == name);

    if dry_run {
        let outcome = RemoveOutcome::dry_run(name, exists || clears_default_layout)
            .with_default_layout(clears_default_layout);
        return emit_remove_outcome(&outcome, output_mode);
    }

    let removed = if clears_default_layout {
        store.clear_setup_default_layout(&setup_fingerprint)?;
        true
    } else {
        store.remove_for_setup(name, &setup_fingerprint, &state_store)?
    };
    let missing_profile_error = || anyhow!("profile '{name}' not found for the current setup");
    let outcome = RemoveOutcome::removed(name, removed).with_default_layout(clears_default_layout);

    emit_remove_outcome(&outcome, output_mode)?;

    if removed {
        Ok(())
    } else {
        Err(missing_profile_error())
    }
}

/// Selects the next saved profile or built-in preset for the current setup.
///
/// Emits the action outcome in the requested format and, after a successful
/// non-dry-run action, sends a best-effort notification naming the selected layout
/// unless `settings.notifications.enabled` is false. A settings file that cannot be
/// re-read at that point skips the notification instead of failing the command.
pub(super) fn cmd_cycle(dry_run: bool, output_mode: OutputMode) -> Result<()> {
    const CYCLE_PRESETS: [VirtualPreset; 2] = [VirtualPreset::Horizontal, VirtualPreset::Vertical];

    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let query_context = ProfileQueryContext::from_state(&state);
    let setup = load_current_topology(backend.as_ref(), &state_store)?.setup_fingerprint();
    let profiles: Vec<Profile> = store.profiles_for_setup(&setup, &query_context)?;
    let presets: Vec<VirtualPreset> = CYCLE_PRESETS
        .into_iter()
        .filter(|preset| {
            profiles
                .iter()
                .all(|profile| profile.name != preset.as_str())
        })
        .collect();

    let targets: Vec<&str> = profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .chain(presets.iter().map(|preset| preset.as_str()))
        .collect();

    let next_idx = state.last_profile.as_ref().map_or(0, |current| {
        targets
            .iter()
            .position(|target| target == current)
            .map_or(0, |idx| (idx + 1) % targets.len())
    });

    let outcome = if let Some(profile) = profiles.get(next_idx) {
        execute_profile_action(
            backend.as_ref(),
            &store,
            &state_store,
            profile,
            dry_run,
            false,
            false,
        )?
    } else {
        let preset = presets[next_idx - profiles.len()];
        let mut outcome = execute_virtual_action(
            backend.as_ref(),
            &state_store,
            preset,
            dry_run,
            store.settings()?.builtin_output.as_ref(),
            false,
        )?;
        persist_virtual_action_outcome(
            &state_store,
            None,
            &mut outcome,
            preset.as_str(),
            None,
            false,
        )?;
        outcome
    };
    emit_action_outcome("cycle", None, &outcome, output_mode)?;
    if !dry_run {
        if let Ok(settings) = store.settings() {
            if settings.notifications.enabled {
                notify(
                    &format!("Display layout: {}", targets[next_idx]),
                    &format!("{} of {}", next_idx + 1, targets.len()),
                    settings.notifications.timeout_ms,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_set_requires_explicit_target_for_reverse_flag() {
        let err = cmd_set(
            Some(AUTO_SET_TARGET),
            None,
            SetOptions {
                dry_run: false,
                make_default: false,
                save: false,
                reverse: true,
                force: false,
            },
            OutputMode::Text,
        )
        .expect_err("expected reverse validation to fail");

        assert_eq!(
            err.to_string(),
            "--reverse cannot be used with `waytorandr set auto`"
        );
    }

    #[test]
    fn cmd_set_rejects_default_without_explicit_target() {
        let err = cmd_set(
            Some(AUTO_SET_TARGET),
            None,
            SetOptions {
                dry_run: false,
                make_default: true,
                save: false,
                reverse: false,
                force: false,
            },
            OutputMode::Text,
        )
        .expect_err("expected default validation to fail");

        assert_eq!(
            err.to_string(),
            "--default and --save cannot be used with `waytorandr set auto`"
        );
    }

    #[test]
    fn cmd_set_rejects_reverse_for_unknown_target() {
        let err = cmd_set(
            Some("desk"),
            None,
            SetOptions {
                dry_run: false,
                make_default: false,
                save: false,
                reverse: true,
                force: false,
            },
            OutputMode::Text,
        )
        .expect_err("expected preset resolution to fail");

        assert_eq!(
            err.to_string(),
            "--reverse can only be used with virtual 'horizontal' or 'vertical' set targets"
        );
    }

    #[test]
    fn cmd_set_rejects_missing_target_and_profile() {
        let err = cmd_set(
            None,
            None,
            SetOptions {
                dry_run: false,
                make_default: false,
                save: false,
                reverse: false,
                force: false,
            },
            OutputMode::Text,
        )
        .expect_err("missing target should be rejected");

        assert_eq!(err.to_string(), "set requires either a target or --profile");
    }
}
