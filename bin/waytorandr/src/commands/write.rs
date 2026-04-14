use anyhow::{anyhow, bail, Result};

use super::apply::{
    current_setup_fingerprint, emit_action_outcome, execute_profile_action, execute_virtual_action,
    set_default_profile_for_fingerprint, JsonRemoveResponse, JsonSaveResponse,
};
use super::output::{print_plan_summary, write_json};
use super::shared::plan_outputs;
use super::OutputMode;
use crate::preset::resolve_virtual_preset;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::Profile;
use waytorandr_core::state::StateStore;
use waytorandr_core::store::ProfileStore;
use waytorandr_core::workflow;

pub(super) fn cmd_save(
    name: &str,
    setup_name: Option<&str>,
    dry_run: bool,
    make_default: bool,
    output_mode: OutputMode,
) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let backend = connect_backend()?;
    let topology = workflow::normalized_topology_from_backend(backend.as_ref(), &state_store)?;
    let setup_fingerprint = topology.setup_fingerprint();

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
            });
        }

        println!("Would save profile '{name}':");
        print_plan_summary(&plan);
        if let Some(setup_name) = setup_name {
            println!("Would also name this setup '{setup_name}'");
        }
        if make_default {
            println!("Would also set '{name}' as the default profile for this setup");
        }
        return Ok(());
    }

    let _topology = workflow::observed_topology_from_backend(backend.as_ref(), &state_store)?;
    let state = state_store.load_state()?.unwrap_or_default();
    store.save_with_known_outputs(&profile, &state.known_outputs)?;
    if let Some(setup_name) = setup_name {
        workflow::set_setup_name_for_setup_in_store(&state_store, &setup_fingerprint, setup_name)?;
    }
    if make_default {
        set_default_profile_for_fingerprint(name, &setup_fingerprint)?;
    }
    if output_mode.is_json() {
        return write_json(&JsonSaveResponse {
            command: "save",
            profile: name.to_string(),
            setup_name: setup_name.map(str::to_string),
            dry_run: false,
            saved: true,
            plan: None,
            default_set: make_default,
        });
    }

    println!("Saved profile '{name}'");
    if let Some(setup_name) = setup_name {
        println!("Named this setup '{setup_name}'");
    }
    if make_default {
        println!("Set '{name}' as the default profile for this setup");
    }
    Ok(())
}

pub(super) fn cmd_set(
    name: Option<&str>,
    dry_run: bool,
    make_default: bool,
    reverse: bool,
    largest: bool,
    output_mode: OutputMode,
) -> Result<()> {
    if name.is_none() {
        if reverse {
            bail!("--reverse requires a virtual 'horizontal' or 'vertical' set target")
        }
        if largest {
            bail!("--largest is deprecated; use `waytorandr set largest`")
        }
        if make_default {
            bail!("--default requires an explicit saved profile target")
        }
        return cmd_change(dry_run, output_mode);
    }

    let name = name.expect("checked above");
    if let Some(preset) = resolve_virtual_preset(name, reverse, largest)? {
        if make_default {
            bail!("--default can only be used with saved profile targets")
        }
        let outcome = execute_virtual_action(preset, dry_run)?;
        return emit_action_outcome("set", Some("explicit"), &outcome, output_mode);
    }

    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let setup_fingerprint = current_setup_fingerprint()?;
    let profile = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.get_for_setup_with_known_outputs(name, setup_fingerprint, &state.known_outputs)?
    } else {
        store.get_unique_with_known_outputs(name, &state.known_outputs)?
    }
    .ok_or_else(|| anyhow!("profile '{name}' not found"))?;
    let outcome = execute_profile_action(&profile.profile, dry_run, make_default)?;
    emit_action_outcome("set", Some("explicit"), &outcome, output_mode)
}

pub(super) fn cmd_change(dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let topology = super::shared::load_current_topology(&state_store)?;
    let state = state_store.load_state()?.unwrap_or_default();
    let profiles = store.profiles_with_known_outputs(&state.known_outputs)?;
    let profile = workflow::select_profile_for_topology(&topology, &profiles, &state)
        .ok_or_else(|| anyhow!("no matching profile and no default profile configured"))?;
    let outcome = execute_profile_action(&profile, dry_run, false)?;
    emit_action_outcome("set", Some("auto"), &outcome, output_mode)
}

pub(super) fn cmd_remove(name: &str, dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let setup_fingerprint = current_setup_fingerprint()?;
    let exists = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store
            .get_for_setup_with_known_outputs(name, setup_fingerprint, &state.known_outputs)?
            .is_some()
    } else {
        store
            .get_unique_with_known_outputs(name, &state.known_outputs)?
            .is_some()
    };

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
            println!("Would remove profile '{name}'");
        } else {
            println!("Profile '{name}' not found");
        }
        return Ok(());
    }

    let removed = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.remove_for_setup_with_known_outputs(name, setup_fingerprint, &state.known_outputs)?
    } else {
        store.remove_unique_with_known_outputs(name, &state.known_outputs)?
    };

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
        println!("Removed profile '{name}'");
    } else {
        println!("Profile '{name}' not found");
    }
    Ok(())
}

pub(super) fn cmd_cycle(dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let profiles: Vec<Profile> = if let Some(setup) = current_setup_fingerprint()?.as_deref() {
        store.profiles_for_setup_with_known_outputs(setup, &state.known_outputs)?
    } else {
        store.profiles_with_known_outputs(&state.known_outputs)?
    };
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
