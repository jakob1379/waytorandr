use anyhow::{anyhow, bail, Result};

use super::apply::{
    current_setup_fingerprint, emit_action_outcome, execute_profile_action, execute_virtual_action,
    save_runtime_state, set_default_profile_for_fingerprint, DefaultScope, JsonRemoveResponse,
    JsonSaveResponse,
};
use super::output::{failure, print_plan_summary, success, value, warning, write_json};
use super::shared::plan_outputs;
use super::OutputMode;
use crate::preset::resolve_virtual_preset;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::model::{BackendKind, Topology};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::Profile;
use waytorandr_core::state::StateStore;
use waytorandr_core::store::ProfileStore;
use waytorandr_core::workflow;

const DEFAULT_SAVED_PROFILE_NAME: &str = "default";

#[derive(Clone, Copy)]
pub(super) struct SetOptions {
    pub(super) dry_run: bool,
    pub(super) make_default: bool,
    pub(super) global_default: bool,
    pub(super) save: bool,
    pub(super) reverse: bool,
    pub(super) largest: bool,
}

struct SavedCurrentLayout {
    backend_kind: BackendKind,
    topology: Topology,
}

fn save_current_layout(
    name: &str,
    setup_name: Option<&str>,
    make_default: bool,
) -> Result<SavedCurrentLayout> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let backend_kind = backend.capabilities().backend;
    let topology = workflow::normalized_topology_from_backend(backend.as_ref(), &state_store)?;
    let setup_fingerprint = topology.setup_fingerprint();

    if topology.outputs.is_empty() {
        bail!("cannot save a profile from an empty topology")
    }

    let profile = workflow::profile_from_topology(name, &topology);
    let observed_topology =
        workflow::observed_topology_from_backend(backend.as_ref(), &state_store)?;

    store.save(&profile, &state_store)?;
    if let Some(setup_name) = setup_name {
        workflow::set_setup_name_for_setup_in_store(&state_store, &setup_fingerprint, setup_name)?;
    }
    if make_default {
        set_default_profile_for_fingerprint(name, &setup_fingerprint)?;
    }

    Ok(SavedCurrentLayout {
        backend_kind,
        topology: observed_topology,
    })
}

pub(super) fn cmd_save(
    name: &str,
    setup_name: Option<&str>,
    dry_run: bool,
    make_default: bool,
    output_mode: OutputMode,
) -> Result<()> {
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let topology = workflow::normalized_topology_from_backend(backend.as_ref(), &state_store)?;

    if topology.outputs.is_empty() {
        bail!("cannot save a profile from an empty topology")
    }

    let profile = workflow::profile_from_topology(name, &topology);

    if dry_run {
        let plan = LayoutPlan::new(
            profile
                .layout
                .iter()
                .map(|(output_name, config)| (output_name.clone(), config.state.clone()))
                .collect(),
        );
        if output_mode.is_json() {
            return write_json(&JsonSaveResponse {
                command: "save",
                profile: name.to_string(),
                setup_name: setup_name.map(str::to_string),
                dry_run: true,
                saved: false,
                plan: Some(plan_outputs(&plan)),
                default_set: make_default,
                default_scope: make_default.then_some(DefaultScope::Setup.as_json()),
            });
        }

        println!(
            "{} {}:",
            warning("Would save"),
            value(format!("profile '{name}'"))
        );
        print_plan_summary(&plan);
        if let Some(setup_name) = setup_name {
            println!(
                "{} {}",
                warning("Would also name this setup"),
                value(format!("'{setup_name}'"))
            );
        }
        if make_default {
            println!(
                "{} {}",
                warning("Would also set"),
                value(format!("'{name}' as the default profile for this setup"))
            );
        }
        return Ok(());
    }

    let _saved_layout = save_current_layout(name, setup_name, make_default)?;
    if output_mode.is_json() {
        return write_json(&JsonSaveResponse {
            command: "save",
            profile: name.to_string(),
            setup_name: setup_name.map(str::to_string),
            dry_run: false,
            saved: true,
            plan: None,
            default_set: make_default,
            default_scope: make_default.then_some(DefaultScope::Setup.as_json()),
        });
    }

    println!(
        "{} {}",
        success("Saved"),
        value(format!("profile '{name}'"))
    );
    if let Some(setup_name) = setup_name {
        println!(
            "{} {}",
            success("Named this setup"),
            value(format!("'{setup_name}'"))
        );
    }
    if make_default {
        println!(
            "{} {}",
            success("Set"),
            value(format!("'{name}' as the default profile for this setup"))
        );
    }
    Ok(())
}

pub(super) fn cmd_set(
    name: Option<&str>,
    options: SetOptions,
    output_mode: OutputMode,
) -> Result<()> {
    let SetOptions {
        dry_run,
        make_default,
        global_default,
        save,
        reverse,
        largest,
    } = options;
    let save_for_current_setup = make_default || save;

    if global_default && (make_default || save) {
        bail!("--global-default cannot be combined with --default or --save")
    }

    if name.is_none() {
        if reverse {
            bail!("--reverse requires a virtual 'horizontal' or 'vertical' set target")
        }
        if largest {
            bail!("--largest is deprecated; use `waytorandr set largest`")
        }
        if save_for_current_setup || global_default {
            bail!("--default, --global-default, and --save require an explicit set target")
        }
        return cmd_change(dry_run, output_mode);
    }

    let name = name.expect("checked above");
    if let Some(preset) = resolve_virtual_preset(name, reverse, largest)? {
        let mut outcome = execute_virtual_action(preset, dry_run, global_default, None)?;
        if save_for_current_setup {
            outcome.record_saved_profile(DEFAULT_SAVED_PROFILE_NAME);
            outcome.set_default_assignment(DEFAULT_SAVED_PROFILE_NAME, DefaultScope::Setup);
            if !dry_run {
                let saved_layout = save_current_layout(DEFAULT_SAVED_PROFILE_NAME, None, true)?;
                save_runtime_state(
                    DEFAULT_SAVED_PROFILE_NAME,
                    Some(saved_layout.backend_kind),
                    &saved_layout.topology,
                )?;
            }
        }
        return emit_action_outcome("set", Some("explicit"), &outcome, output_mode);
    }

    if global_default {
        bail!("--global-default can only be used with virtual set targets")
    }
    if save {
        bail!("--save can only be used with virtual set targets")
    }

    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let setup_fingerprint = current_setup_fingerprint()?;
    let profile = store
        .get_for_setup(name, &setup_fingerprint, &state_store)?
        .ok_or_else(|| anyhow!("profile '{name}' not found"))?;
    let outcome = execute_profile_action(&profile.profile, dry_run, make_default)?;
    emit_action_outcome("set", Some("explicit"), &outcome, output_mode)
}

pub(super) fn cmd_change(dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let topology = super::shared::load_current_topology(&state_store)?;
    let profiles = store.profiles(&state_store)?;
    let settings = store.settings()?;
    let target = workflow::select_target_for_topology(&topology, &profiles, &settings)
        .ok_or_else(|| anyhow!("no matching profile and no default target configured"))?;

    let outcome = match target {
        workflow::SelectedTarget::Profile(profile) => {
            execute_profile_action(&profile, dry_run, false)?
        }
        workflow::SelectedTarget::Virtual(preset) => {
            execute_virtual_action(preset, dry_run, false, settings.builtin_output.as_ref())?
        }
    };

    emit_action_outcome("set", Some("auto"), &outcome, output_mode)
}

pub(super) fn cmd_remove(name: &str, dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let setup_fingerprint = current_setup_fingerprint()?;
    let exists = store
        .get_for_setup(name, &setup_fingerprint, &state_store)?
        .is_some();

    if dry_run {
        if output_mode.is_json() {
            return write_json(&JsonRemoveResponse {
                command: "remove",
                profile: name.to_string(),
                dry_run: true,
                removed: None,
                would_remove: Some(exists),
            });
        }

        if exists {
            println!(
                "{} {}",
                warning("Would remove"),
                value(format!("profile '{name}'"))
            );
        } else {
            println!(
                "{} {}",
                failure("Profile not found"),
                value(format!("'{name}'"))
            );
        }
        return Ok(());
    }

    let removed = store.remove_for_setup(name, &setup_fingerprint, &state_store)?;

    if output_mode.is_json() {
        return write_json(&JsonRemoveResponse {
            command: "remove",
            profile: name.to_string(),
            dry_run: false,
            removed: Some(removed),
            would_remove: None,
        });
    }

    if removed {
        println!(
            "{} {}",
            success("Removed"),
            value(format!("profile '{name}'"))
        );
    } else {
        println!(
            "{} {}",
            failure("Profile not found"),
            value(format!("'{name}'"))
        );
    }
    Ok(())
}

pub(super) fn cmd_cycle(dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let setup = current_setup_fingerprint()?;
    let profiles: Vec<Profile> = store.profiles_for_setup(&setup, &state_store)?;
    if profiles.is_empty() {
        bail!("no profiles available to cycle")
    }

    let next_idx = state.last_profile.as_ref().map_or(0, |current| {
        profiles
            .iter()
            .position(|profile| &profile.name == current)
            .map_or(0, |idx| (idx + 1) % profiles.len())
    });

    let outcome = execute_profile_action(&profiles[next_idx], dry_run, false)?;
    emit_action_outcome("cycle", None, &outcome, output_mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_set_requires_explicit_target_for_reverse_flag() {
        let err = cmd_set(
            None,
            SetOptions {
                dry_run: false,
                make_default: false,
                global_default: false,
                save: false,
                reverse: true,
                largest: false,
            },
            OutputMode::Text,
        )
        .expect_err("expected reverse validation to fail");

        assert_eq!(
            err.to_string(),
            "--reverse requires a virtual 'horizontal' or 'vertical' set target"
        );
    }

    #[test]
    fn cmd_set_rejects_default_without_explicit_target() {
        let err = cmd_set(
            None,
            SetOptions {
                dry_run: false,
                make_default: true,
                global_default: false,
                save: false,
                reverse: false,
                largest: false,
            },
            OutputMode::Text,
        )
        .expect_err("expected default validation to fail");

        assert_eq!(
            err.to_string(),
            "--default, --global-default, and --save require an explicit set target"
        );
    }

    #[test]
    fn cmd_set_rejects_reverse_for_unknown_target() {
        let err = cmd_set(
            Some("desk"),
            SetOptions {
                dry_run: false,
                make_default: false,
                global_default: false,
                save: false,
                reverse: true,
                largest: false,
            },
            OutputMode::Text,
        )
        .expect_err("expected preset resolution to fail");

        assert_eq!(
            err.to_string(),
            "--reverse can only be used with virtual 'horizontal' or 'vertical' set targets"
        );
    }
}
