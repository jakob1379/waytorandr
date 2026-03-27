use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::output::{print_plan_summary, print_topology, print_validation_result};
use crate::preset::resolve_virtual_preset;
use waytorandr_core::engine::Backend;
use waytorandr_core::model::Topology;
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::{Hooks, Profile};
use waytorandr_core::runtime;
use waytorandr_core::store::{ProfileStore, StateStore, StoredProfile};

pub(crate) fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Set(args) => cmd_set(
            args.target.as_deref(),
            args.dry_run,
            args.make_default,
            args.reverse,
            args.largest,
        ),
        Commands::Save(args) => cmd_save(&args.name, args.dry_run, args.make_default),
        Commands::Remove(args) => cmd_remove(&args.name, args.dry_run),
        Commands::Cycle(args) => cmd_cycle(args.dry_run),
        Commands::List(args) => cmd_list(args.all),
        Commands::Current => cmd_current(),
        Commands::Detected => cmd_detected(),
    }
}

fn cmd_list(show_all: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles = store.list()?;

    if profiles.is_empty() {
        println!("No profiles saved");
        return Ok(());
    }

    let state_store = StateStore::new()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let current_topology = Some(load_current_topology(&state_store)?);
    let current_setup = current_topology.as_ref().map(Topology::setup_fingerprint);

    let listed_profiles: Vec<StoredProfile> = if show_all {
        profiles
    } else if let Some(setup) = current_setup.as_deref() {
        store.list_for_setup(setup)?
    } else {
        Vec::new()
    };

    if listed_profiles.is_empty() {
        println!("No profiles match the current topology");
        if let Some(setup) = &current_setup {
            println!("Current fingerprint: {}", setup);
        }
        return Ok(());
    }

    println!("Profiles:");
    let mut current_group: Option<&str> = None;
    for stored in &listed_profiles {
        if current_group != Some(stored.setup_fingerprint.as_str()) {
            current_group = Some(stored.setup_fingerprint.as_str());
            println!(
                "  fingerprint: {}{}",
                stored.setup_fingerprint,
                if current_setup.as_deref() == Some(stored.setup_fingerprint.as_str()) {
                    " [current]"
                } else {
                    ""
                }
            );
        }

        let is_default = runtime::default_profile_for_setup(&state, &stored.setup_fingerprint)
            == Some(stored.profile.name.as_str());
        let is_active = state.last_profile.as_ref() == Some(&stored.profile.name);
        let mut flags = Vec::new();
        if is_default {
            flags.push("default");
        }
        if is_active {
            flags.push("active");
        }

        println!(
            "    {} (priority: {}){}",
            stored.profile.name,
            stored.profile.priority,
            if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            }
        );
    }

    Ok(())
}

fn cmd_current() -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles = store.profiles()?;
    let backend = connect_backend()?;
    let state_store = StateStore::new()?;
    let topology = state_store.normalize_topology_and_persist(&backend.current_state()?)?;
    let state = state_store.load_state()?.unwrap_or_default();

    let current = runtime::current_profile_name(&topology, &profiles, &state)
        .unwrap_or_else(|| "none".to_string());
    println!("Current profile: {}", current);

    Ok(())
}

fn cmd_detected() -> Result<()> {
    let state_store = StateStore::new()?;
    let topology = load_current_topology(&state_store)?;
    print_topology("Detected outputs:", &topology);
    Ok(())
}

fn cmd_save(name: &str, dry_run: bool, make_default: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let state_store = StateStore::new()?;
    let topology = load_current_topology(&state_store)?;
    let setup_fingerprint = topology.setup_fingerprint();

    if topology.outputs.is_empty() {
        bail!("cannot save a profile from an empty topology")
    }

    let profile = runtime::profile_from_topology(name, &topology);

    if dry_run {
        println!("Would save profile '{}':", name);
        print_plan_summary(&LayoutPlan::new(
            profile
                .layout
                .iter()
                .map(|(output_name, config)| (output_name.clone(), config.state.clone()))
                .collect(),
        ));
        if make_default {
            println!("Would also set '{}' as the default profile", name);
        }
        return Ok(());
    }

    store.save(&profile, &setup_fingerprint)?;
    if make_default {
        let mut state = state_store.load_state()?.unwrap_or_default();
        runtime::set_default_profile_for_setup(&mut state, &setup_fingerprint, name);
        state_store.save_state(&state)?;
    }
    println!("Saved profile '{}'", name);
    if make_default {
        println!("Set '{}' as default profile", name);
    }
    Ok(())
}

fn cmd_set(
    name: Option<&str>,
    dry_run: bool,
    make_default: bool,
    reverse: bool,
    largest: bool,
) -> Result<()> {
    if name.is_none() {
        if reverse {
            bail!("--reverse requires a virtual 'horizontal' or 'vertical' set target")
        }
        if largest {
            bail!("--largest requires the virtual 'common' set target")
        }
        if make_default {
            bail!("--default requires an explicit saved profile target")
        }
        return cmd_change(dry_run);
    }

    let name = name.expect("checked above");
    if let Some(preset) = resolve_virtual_preset(name, reverse, largest)? {
        if make_default {
            bail!("--default can only be used with saved profile targets")
        }
        return execute_virtual_action(&preset, dry_run);
    }

    let store = ProfileStore::new()?;
    let setup_fingerprint = current_setup_fingerprint()?;
    let profile = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.get_in_setup(name, setup_fingerprint)?
    } else {
        store.get_unique(name)?
    }
    .ok_or_else(|| anyhow!("profile '{}' not found", name))?;
    execute_profile_action(&profile.profile, dry_run)?;
    if make_default {
        set_default_profile_for_setup(&profile.profile.name, dry_run)?;
    }
    Ok(())
}

fn cmd_change(dry_run: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let state_store = StateStore::new()?;
    let topology = load_current_topology(&state_store)?;
    let profiles = store.profiles()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let profile = runtime::select_profile_for_topology(&topology, &profiles, &state)
        .ok_or_else(|| anyhow!("no matching profile and no default profile configured"))?;
    execute_profile_action(&profile, dry_run)
}

fn cmd_remove(name: &str, dry_run: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let setup_fingerprint = current_setup_fingerprint()?;
    let exists = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.get_in_setup(name, setup_fingerprint)?.is_some()
    } else {
        store.get_unique(name)?.is_some()
    };

    if dry_run {
        if exists {
            println!("Would remove profile '{}'", name);
        } else {
            println!("Profile '{}' not found", name);
        }
        return Ok(());
    }

    let removed = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.remove_in_setup(name, setup_fingerprint)?
    } else {
        store.remove_unique(name)?
    };

    if removed {
        println!("Removed profile '{}'", name);
    } else {
        println!("Profile '{}' not found", name);
    }
    Ok(())
}

fn cmd_cycle(dry_run: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles: Vec<Profile> = if let Some(setup) = current_setup_fingerprint()? {
        store.profiles_for_setup(&setup)?
    } else {
        store.profiles()?
    };
    if profiles.is_empty() {
        bail!("no profiles available to cycle")
    }

    let state_store = StateStore::new()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let next_idx = match state.last_profile.as_ref() {
        Some(current) => profiles
            .iter()
            .position(|profile| &profile.name == current)
            .map(|idx| (idx + 1) % profiles.len())
            .unwrap_or(0),
        None => 0,
    };

    execute_profile_action(&profiles[next_idx], dry_run)
}

fn execute_virtual_action(preset: &str, dry_run: bool) -> Result<()> {
    let backend = connect_backend()?;
    let hooks = Hooks::default();
    let state_store = StateStore::new()?;
    let cycle = runtime::execute_plan_cycle_with_backend(&backend, &hooks, dry_run, || {
        runtime::plan_preset_with_backend(&backend, &state_store, preset)
    })
    .map_err(anyhow::Error::from)?;
    let test = cycle.validation;

    if dry_run {
        println!("Dry run for virtual configuration '{}':", preset);
        print_plan_summary(&cycle.validation_plan);
        let validation = Ok(test.clone());
        print_validation_result(&validation);
        validation?;
        if !test.success {
            bail!(test
                .message
                .unwrap_or_else(|| "backend rejected configuration".to_string()));
        }
        return Ok(());
    }

    if !test.success {
        bail!(test
            .message
            .unwrap_or_else(|| "backend rejected configuration".to_string()));
    }

    let apply_topology = cycle
        .apply_topology
        .ok_or_else(|| anyhow!("missing apply topology"))?;
    let apply_plan = cycle
        .apply_plan
        .ok_or_else(|| anyhow!("missing apply plan"))?;
    let applied = cycle
        .apply_result
        .ok_or_else(|| anyhow!("missing apply result"))?;
    if !applied.success {
        bail!(applied
            .message
            .unwrap_or_else(|| "backend failed to apply configuration".to_string()));
    }

    let applied_topology = applied.applied_state.unwrap_or(apply_topology);
    save_runtime_state(preset, Some("wlroots"), &applied_topology)?;

    println!("Set virtual configuration '{}'", preset);
    print_plan_summary(&apply_plan);
    Ok(())
}

fn execute_profile_action(profile: &Profile, dry_run: bool) -> Result<()> {
    validate_profile(profile)?;
    let backend = connect_backend()?;
    let state_store = StateStore::new()?;
    let cycle = runtime::execute_plan_cycle_with_backend(&backend, &profile.hooks, dry_run, || {
        runtime::plan_profile_with_backend(&backend, &state_store, profile)
    })
    .map_err(anyhow::Error::from)?;
    let test = cycle.validation;

    if dry_run {
        println!("Dry run for profile '{}':", profile.name);
        print_plan_summary(&cycle.validation_plan);
        let validation = Ok(test.clone());
        print_validation_result(&validation);
        validation?;
        if !test.success {
            bail!(test
                .message
                .unwrap_or_else(|| "backend rejected configuration".to_string()));
        }
        return Ok(());
    }

    if !test.success {
        bail!(test
            .message
            .unwrap_or_else(|| "backend rejected configuration".to_string()));
    }

    let apply_topology = cycle
        .apply_topology
        .ok_or_else(|| anyhow!("missing apply topology"))?;
    let apply_plan = cycle
        .apply_plan
        .ok_or_else(|| anyhow!("missing apply plan"))?;
    let applied = cycle
        .apply_result
        .ok_or_else(|| anyhow!("missing apply result"))?;
    if !applied.success {
        bail!(applied
            .message
            .unwrap_or_else(|| "backend failed to apply configuration".to_string()));
    }

    let applied_topology = applied.applied_state.unwrap_or(apply_topology);
    save_runtime_state(&profile.name, Some("wlroots"), &applied_topology)?;

    println!("Set profile '{}'", profile.name);
    print_plan_summary(&apply_plan);
    Ok(())
}

fn validate_profile(profile: &Profile) -> Result<()> {
    if profile.layout.is_empty() {
        bail!(
            "profile '{}' contains no saved layout; re-save it with 'waytorandr save {}' before setting it",
            profile.name,
            profile.name
        );
    }

    Ok(())
}

fn connect_backend() -> Result<waytorandr_wlroots::backend::WlrootsBackend> {
    waytorandr_wlroots::backend::WlrootsBackend::connect()
        .context("failed to connect to wlroots output-management backend")
}

fn current_setup_fingerprint() -> Result<Option<String>> {
    let state_store = StateStore::new()?;
    load_current_topology(&state_store).map(|topology| Some(topology.setup_fingerprint()))
}

fn set_default_profile_for_setup(profile_name: &str, dry_run: bool) -> Result<()> {
    let state_store = StateStore::new()?;
    let setup_fingerprint = load_current_topology(&state_store)?.setup_fingerprint();

    if dry_run {
        println!(
            "Would also set '{}' as default profile for this hardware setup",
            profile_name
        );
        return Ok(());
    }

    let mut state = state_store.load_state()?.unwrap_or_default();
    runtime::set_default_profile_for_setup(&mut state, &setup_fingerprint, profile_name);
    state_store.save_state(&state)?;
    println!("Set '{}' as default profile", profile_name);
    Ok(())
}

fn load_current_topology(state_store: &StateStore) -> Result<Topology> {
    let backend = connect_backend()?;
    Ok(runtime::normalized_topology_from_backend(&backend, state_store)?)
}

fn save_runtime_state(
    profile_name: &str,
    backend: Option<&str>,
    topology: &Topology,
) -> Result<()> {
    let state_store = StateStore::new()?;
    let mut state = state_store.load_state()?.unwrap_or_default();
    runtime::record_applied_profile(&mut state, profile_name, backend, topology);
    state_store.save_state(&state)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use waytorandr_core::model::OutputState;
    use waytorandr_core::profile::{Hooks, OutputConfig, ProfileOptions};

    fn output(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state
    }

    #[test]
    fn validate_profile_rejects_empty_layout() {
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: Vec::new(),
            layout: HashMap::new(),
            hooks: Hooks::default(),
            options: ProfileOptions::default(),
        };

        assert!(validate_profile(&profile).is_err());
    }

    #[test]
    fn resolve_profile_plan_accepts_canonical_profile() {
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: Vec::new(),
            layout: HashMap::from([(
                "DP-1".to_string(),
                OutputConfig {
                    state: output("DP-1"),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
            options: ProfileOptions::default(),
        };
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };

        let canonical = profile.with_inferred_match_rules();

        assert!(runtime::plan_profile_for_topology(&canonical, &topology).is_ok());
    }
}
