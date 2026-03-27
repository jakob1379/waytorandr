use anyhow::{anyhow, bail, Result};
use clap::Parser;
use serde::Serialize;

use crate::cli::{Cli, Commands};
use crate::output::{print_plan_summary, print_topology, print_validation_result};
use crate::preset::resolve_virtual_preset;
use waytorandr_core::engine::{Backend, ConfigFailureKind, TestResult};
use waytorandr_core::model::{OutputState, Topology};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::{Hooks, Profile};
use waytorandr_core::runtime;
use waytorandr_core::store::{ProfileStore, StateStore, StoredProfile};

#[derive(Clone, Copy)]
enum OutputMode {
    Text,
    Json,
}

impl OutputMode {
    fn from_json(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Text
        }
    }

    fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

#[derive(Clone, Copy)]
enum ActionTargetType {
    Profile,
    Virtual,
}

impl ActionTargetType {
    fn as_json(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Virtual => "virtual",
        }
    }

    fn as_human(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Virtual => "virtual configuration",
        }
    }
}

struct ActionOutcome {
    target: String,
    target_type: ActionTargetType,
    dry_run: bool,
    plan: LayoutPlan,
    validation: Option<TestResult>,
    default_set: bool,
}

#[derive(Serialize)]
struct JsonOutputEntry {
    name: String,
    state: OutputState,
}

#[derive(Serialize)]
struct JsonValidation {
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
struct JsonListProfile {
    name: String,
    priority: u32,
    is_default: bool,
    is_active: bool,
}

#[derive(Serialize)]
struct JsonListSetup {
    fingerprint: String,
    is_current: bool,
    profiles: Vec<JsonListProfile>,
}

#[derive(Serialize)]
struct JsonListResponse {
    command: &'static str,
    show_all: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_setup: Option<String>,
    setups: Vec<JsonListSetup>,
}

#[derive(Serialize)]
struct JsonSaveResponse {
    command: &'static str,
    profile: String,
    dry_run: bool,
    saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<Vec<JsonOutputEntry>>,
    #[serde(skip_serializing_if = "is_false")]
    default_set: bool,
}

#[derive(Serialize)]
struct JsonRemoveResponse {
    command: &'static str,
    profile: String,
    dry_run: bool,
    removed: bool,
}

#[derive(Serialize)]
struct JsonCurrentResponse {
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

#[derive(Serialize)]
struct JsonDetectedResponse {
    command: &'static str,
    fingerprint: String,
    setup_fingerprint: String,
    outputs: Vec<JsonOutputEntry>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn write_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn failure_kind_label(kind: ConfigFailureKind) -> &'static str {
    match kind {
        ConfigFailureKind::Rejected => "rejected",
        ConfigFailureKind::TopologyChanged => "topology_changed",
    }
}

fn json_validation(test: &TestResult) -> JsonValidation {
    JsonValidation {
        success: test.success,
        failure: test.failure.map(failure_kind_label),
        message: test.message.clone(),
    }
}

fn plan_outputs(plan: &LayoutPlan) -> Vec<JsonOutputEntry> {
    let mut outputs: Vec<JsonOutputEntry> = plan
        .outputs
        .iter()
        .map(|(name, state)| JsonOutputEntry {
            name: name.clone(),
            state: state.clone(),
        })
        .collect();
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    outputs
}

fn topology_outputs(topology: &Topology) -> Vec<JsonOutputEntry> {
    let mut outputs: Vec<JsonOutputEntry> = topology
        .outputs
        .iter()
        .map(|(name, state)| JsonOutputEntry {
            name: name.clone(),
            state: state.clone(),
        })
        .collect();
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    outputs
}

pub(crate) fn run() -> Result<()> {
    let cli = Cli::parse();
    let output_mode = OutputMode::from_json(cli.json);

    match cli.command {
        Commands::Set(args) => cmd_set(
            args.target.as_deref(),
            args.dry_run,
            args.make_default,
            args.reverse,
            args.largest,
            output_mode,
        ),
        Commands::Save(args) => cmd_save(&args.name, args.dry_run, args.make_default, output_mode),
        Commands::Remove(args) => cmd_remove(&args.name, args.dry_run, output_mode),
        Commands::Cycle(args) => cmd_cycle(args.dry_run, output_mode),
        Commands::List(args) => cmd_list(args.all, output_mode),
        Commands::Current => cmd_current(output_mode),
        Commands::Detected => cmd_detected(output_mode),
    }
}

fn cmd_list(show_all: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles = store.list()?;

    let state_store = StateStore::new()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let current_topology = Some(load_current_topology(&state_store)?);
    let current_setup = current_topology.as_ref().map(Topology::setup_fingerprint);

    if profiles.is_empty() {
        if output_mode.is_json() {
            return write_json(&JsonListResponse {
                command: "list",
                show_all,
                current_setup,
                setups: Vec::new(),
            });
        }
        println!("No profiles saved");
        return Ok(());
    }

    let listed_profiles: Vec<StoredProfile> = if show_all {
        profiles
    } else if let Some(setup) = current_setup.as_deref() {
        store.list_for_setup(setup)?
    } else {
        Vec::new()
    };

    if listed_profiles.is_empty() && !output_mode.is_json() {
        println!("No profiles match the current topology");
        if let Some(setup) = &current_setup {
            println!("Current fingerprint: {}", setup);
        }
        return Ok(());
    }

    if output_mode.is_json() {
        let mut setups = Vec::new();
        let mut current_fingerprint: Option<String> = None;
        let mut current_profiles: Vec<JsonListProfile> = Vec::new();

        for stored in &listed_profiles {
            if current_fingerprint.as_deref() != Some(stored.setup_fingerprint.as_str()) {
                if let Some(fingerprint) = current_fingerprint.take() {
                    setups.push(JsonListSetup {
                        is_current: current_setup.as_deref() == Some(fingerprint.as_str()),
                        fingerprint,
                        profiles: current_profiles,
                    });
                    current_profiles = Vec::new();
                }
                current_fingerprint = Some(stored.setup_fingerprint.clone());
            }

            current_profiles.push(JsonListProfile {
                name: stored.profile.name.clone(),
                priority: stored.profile.priority,
                is_default: runtime::default_profile_for_setup(&state, &stored.setup_fingerprint)
                    == Some(stored.profile.name.as_str()),
                is_active: state.last_profile.as_ref() == Some(&stored.profile.name),
            });
        }

        if let Some(fingerprint) = current_fingerprint {
            setups.push(JsonListSetup {
                is_current: current_setup.as_deref() == Some(fingerprint.as_str()),
                fingerprint,
                profiles: current_profiles,
            });
        }

        return write_json(&JsonListResponse {
            command: "list",
            show_all,
            current_setup,
            setups,
        });
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

fn cmd_current(output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles = store.profiles()?;
    let backend = connect_backend()?;
    let state_store = StateStore::new()?;
    let topology = state_store.normalize_topology_and_persist(&backend.current_state()?)?;
    let state = state_store.load_state()?.unwrap_or_default();

    let current = runtime::current_profile_name(&topology, &profiles, &state);
    if output_mode.is_json() {
        return write_json(&JsonCurrentResponse {
            command: "current",
            profile: current,
        });
    }

    println!(
        "Current profile: {}",
        current.unwrap_or_else(|| "none".to_string())
    );

    Ok(())
}

fn cmd_detected(output_mode: OutputMode) -> Result<()> {
    let state_store = StateStore::new()?;
    let topology = load_current_topology(&state_store)?;
    if output_mode.is_json() {
        return write_json(&JsonDetectedResponse {
            command: "detected",
            fingerprint: topology.fingerprint(),
            setup_fingerprint: topology.setup_fingerprint(),
            outputs: topology_outputs(&topology),
        });
    }
    print_topology("Detected outputs:", &topology);
    Ok(())
}

fn cmd_save(name: &str, dry_run: bool, make_default: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::new()?;
    let state_store = StateStore::new()?;
    let topology = load_current_topology(&state_store)?;
    let setup_fingerprint = topology.setup_fingerprint();

    if topology.outputs.is_empty() {
        bail!("cannot save a profile from an empty topology")
    }

    let profile = runtime::profile_from_topology(name, &topology);

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
                dry_run: true,
                saved: false,
                plan: Some(plan_outputs(&plan)),
                default_set: make_default,
            });
        }

        println!("Would save profile '{}':", name);
        print_plan_summary(&plan);
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
    if output_mode.is_json() {
        return write_json(&JsonSaveResponse {
            command: "save",
            profile: name.to_string(),
            dry_run: false,
            saved: true,
            plan: None,
            default_set: make_default,
        });
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
    output_mode: OutputMode,
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
        return cmd_change(dry_run, output_mode);
    }

    let name = name.expect("checked above");
    if let Some(preset) = resolve_virtual_preset(name, reverse, largest)? {
        if make_default {
            bail!("--default can only be used with saved profile targets")
        }
        let outcome = execute_virtual_action(&preset, dry_run)?;
        return emit_action_outcome("set", Some("explicit"), &outcome, output_mode);
    }

    let store = ProfileStore::new()?;
    let setup_fingerprint = current_setup_fingerprint()?;
    let profile = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.get_in_setup(name, setup_fingerprint)?
    } else {
        store.get_unique(name)?
    }
    .ok_or_else(|| anyhow!("profile '{}' not found", name))?;
    let outcome = execute_profile_action(&profile.profile, dry_run, make_default)?;
    emit_action_outcome("set", Some("explicit"), &outcome, output_mode)
}

fn cmd_change(dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::new()?;
    let state_store = StateStore::new()?;
    let topology = load_current_topology(&state_store)?;
    let profiles = store.profiles()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let profile = runtime::select_profile_for_topology(&topology, &profiles, &state)
        .ok_or_else(|| anyhow!("no matching profile and no default profile configured"))?;
    let outcome = execute_profile_action(&profile, dry_run, false)?;
    emit_action_outcome("set", Some("auto"), &outcome, output_mode)
}

fn cmd_remove(name: &str, dry_run: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::new()?;
    let setup_fingerprint = current_setup_fingerprint()?;
    let exists = if let Some(setup_fingerprint) = setup_fingerprint.as_deref() {
        store.get_in_setup(name, setup_fingerprint)?.is_some()
    } else {
        store.get_unique(name)?.is_some()
    };

    if dry_run {
        if output_mode.is_json() {
            return write_json(&JsonRemoveResponse {
                command: "remove",
                profile: name.to_string(),
                dry_run: true,
                removed: exists,
            });
        }

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

    if output_mode.is_json() {
        return write_json(&JsonRemoveResponse {
            command: "remove",
            profile: name.to_string(),
            dry_run: false,
            removed,
        });
    }

    if removed {
        println!("Removed profile '{}'", name);
    } else {
        println!("Profile '{}' not found", name);
    }
    Ok(())
}

fn cmd_cycle(dry_run: bool, output_mode: OutputMode) -> Result<()> {
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

    let outcome = execute_profile_action(&profiles[next_idx], dry_run, false)?;
    emit_action_outcome("cycle", None, &outcome, output_mode)
}

fn execute_virtual_action(preset: &str, dry_run: bool) -> Result<ActionOutcome> {
    let backend = connect_backend()?;
    let hooks = Hooks::default();
    let state_store = StateStore::new()?;
    let cycle = runtime::execute_plan_cycle_with_backend(&backend, &hooks, dry_run, || {
        runtime::plan_preset_with_backend(&backend, &state_store, preset)
    })
    .map_err(anyhow::Error::from)?;
    let test = cycle.validation;

    if dry_run {
        return Ok(ActionOutcome {
            target: preset.to_string(),
            target_type: ActionTargetType::Virtual,
            dry_run: true,
            plan: cycle.validation_plan,
            validation: Some(test),
            default_set: false,
        });
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

    Ok(ActionOutcome {
        target: preset.to_string(),
        target_type: ActionTargetType::Virtual,
        dry_run: false,
        plan: apply_plan,
        validation: None,
        default_set: false,
    })
}

fn execute_profile_action(
    profile: &Profile,
    dry_run: bool,
    make_default: bool,
) -> Result<ActionOutcome> {
    validate_profile(profile)?;
    let backend = connect_backend()?;
    let state_store = StateStore::new()?;
    let cycle = runtime::execute_plan_cycle_with_backend(&backend, &profile.hooks, dry_run, || {
        runtime::plan_profile_with_backend(&backend, &state_store, profile)
    })
    .map_err(anyhow::Error::from)?;
    let test = cycle.validation;

    if dry_run {
        return Ok(ActionOutcome {
            target: profile.name.clone(),
            target_type: ActionTargetType::Profile,
            dry_run: true,
            plan: cycle.validation_plan,
            validation: Some(test),
            default_set: make_default,
        });
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
    if make_default {
        set_default_profile_for_fingerprint(&profile.name, &applied_topology.setup_fingerprint())?;
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

fn emit_action_outcome(
    command: &'static str,
    selection: Option<&'static str>,
    outcome: &ActionOutcome,
    output_mode: OutputMode,
) -> Result<()> {
    let validation_failure = outcome
        .validation
        .as_ref()
        .filter(|test| !test.success)
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
            "Dry run for {} '{}':",
            outcome.target_type.as_human(),
            outcome.target
        );
        print_plan_summary(&outcome.plan);
        if let Some(test) = &outcome.validation {
            print_validation_result(&Ok(test.clone()));
        }
        if outcome.default_set {
            println!(
                "Would also set '{}' as default profile for this hardware setup",
                outcome.target
            );
        }
        if let Some(message) = validation_failure {
            bail!(message);
        }
        return Ok(());
    }

    println!(
        "Set {} '{}'",
        outcome.target_type.as_human(),
        outcome.target
    );
    print_plan_summary(&outcome.plan);
    if outcome.default_set {
        println!("Set '{}' as default profile", outcome.target);
    }
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
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".to_string());
    let xdg_runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "<unset>".to_string());
    let display_hint = if wayland_display.contains('/') {
        "; WAYLAND_DISPLAY should be a socket name like 'wayland-0', not a path"
    } else {
        ""
    };

    waytorandr_wlroots::backend::WlrootsBackend::connect().map_err(|err| {
        anyhow!(
            "failed to connect to wlroots output-management backend: {err} (WAYLAND_DISPLAY={wayland_display}, XDG_RUNTIME_DIR={xdg_runtime_dir}{display_hint})"
        )
    })
}

fn current_setup_fingerprint() -> Result<Option<String>> {
    let state_store = StateStore::new()?;
    load_current_topology(&state_store).map(|topology| Some(topology.setup_fingerprint()))
}

fn set_default_profile_for_fingerprint(profile_name: &str, setup_fingerprint: &str) -> Result<()> {
    let state_store = StateStore::new()?;

    let mut state = state_store.load_state()?.unwrap_or_default();
    runtime::set_default_profile_for_setup(&mut state, setup_fingerprint, profile_name);
    state_store.save_state(&state)?;
    Ok(())
}

fn load_current_topology(state_store: &StateStore) -> Result<Topology> {
    let backend = connect_backend()?;
    Ok(runtime::normalized_topology_from_backend(
        &backend,
        state_store,
    )?)
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

    #[test]
    fn plan_outputs_are_sorted_for_json() {
        let plan = LayoutPlan::new(HashMap::from([
            ("eDP-1".to_string(), output("eDP-1")),
            ("DP-1".to_string(), output("DP-1")),
        ]));

        let entries = plan_outputs(&plan);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "DP-1");
        assert_eq!(entries[1].name, "eDP-1");
    }

    #[test]
    fn json_validation_maps_failure_kind_label() {
        let mut test = TestResult::default();
        test.success = false;
        test.failure = Some(ConfigFailureKind::TopologyChanged);
        test.message = Some("changed".to_string());

        let validation = json_validation(&test);

        assert!(!validation.success);
        assert_eq!(validation.failure, Some("topology_changed"));
        assert_eq!(validation.message.as_deref(), Some("changed"));
    }
}
