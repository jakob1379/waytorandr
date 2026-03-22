use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};
use clap_complete::env::CompleteEnv;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_core::store::{ProfileStore, StateStore};
use waytorandr_core::{Backend, LayoutPlan, MatchResult, Matcher, OutputMatcher, Planner, Profile, Topology};

#[derive(Parser)]
#[command(name = "waytorandr")]
#[command(about = "Wayland-native display profile manager")]
#[command(long_about = "Save, set, and switch Wayland display layouts.")]
#[command(subcommand_required = true)]
#[command(arg_required_else_help = true)]
#[command(after_long_help = "Run `waytorandr set --help` or `waytorandr save --help` for command-specific examples.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Set a saved profile, virtual configuration, or matching/default profile")]
    #[command(after_long_help = "Virtual configurations:
  off        Disable all outputs
  common     Place all connected outputs at a common resolution on the same origin
  mirror     Reserved name; prints guidance to use wl-mirror for real mirroring
  horizontal Extend all connected outputs horizontally
  vertical   Extend all connected outputs vertically

Examples:
  waytorandr set
  waytorandr set docked
  waytorandr set common --dry-run
  waytorandr set common --largest --dry-run
  waytorandr set vertical --reverse --dry-run

For true mirroring, use `wl-mirror` until output-management protocols grow real mirroring support.")]
    Set(SetArgs),

    #[command(about = "Save the current compositor layout as a profile")]
    #[command(after_long_help = "Examples:
  waytorandr save
  waytorandr save docked
  waytorandr save --default
  waytorandr save docked --default
  waytorandr save docked --dry-run

If the profile name is omitted, `default` is used.
Use `--default` together with `save` when the current screen setup may match multiple saved profiles and you want this saved layout to become the preferred default.")]
    Save(SaveArgs),

    #[command(about = "Remove a saved profile")]
    Remove(RemoveArgs),

    #[command(about = "Set the next saved profile")]
    Cycle(MutatingArgs),

    #[command(about = "List all saved profiles")]
    List,

    #[command(about = "Show the active or currently matched profile")]
    Current,

    #[command(about = "Show detected outputs and current geometry")]
    Detected,

    #[command(about = "Print the current hardware topology fingerprint")]
    Fingerprint,
}

#[derive(Args)]
struct SetArgs {
    #[arg(
        value_name = "profile",
        help = "Saved profile or virtual configuration; omit to set the matching profile or default",
        add = ArgValueCompleter::new(complete_set_targets)
    )]
    target: Option<String>,

    #[arg(short = 'n', long = "dry-run", help = "Preview without applying the layout")]
    dry_run: bool,

    #[arg(short = 'l', long = "largest", help = "Only with `common`: use the largest available shared target mode")]
    largest: bool,

    #[arg(short = 'r', long = "reverse", help = "Only with `horizontal` or `vertical`: reverse ordering")]
    reverse: bool,
}

#[derive(Args)]
struct SaveArgs {
    #[arg(value_name = "profile", default_value = "default", help = "Profile name to save; defaults to `default`")]
    name: String,

    #[arg(short = 'd', long = "default", help = "Also set the saved profile as the default profile")]
    make_default: bool,

    #[arg(short = 'n', long = "dry-run", help = "Preview the profile that would be saved")]
    dry_run: bool,
}

#[derive(Args)]
struct RemoveArgs {
    #[arg(
        value_name = "profile",
        help = "Profile name to remove",
        add = ArgValueCompleter::new(complete_saved_profiles)
    )]
    name: String,

    #[arg(short = 'n', long = "dry-run", help = "Preview without removing the profile")]
    dry_run: bool,
}

#[derive(Args)]
struct MutatingArgs {
    #[arg(short = 'n', long = "dry-run", help = "Preview without applying changes")]
    dry_run: bool,
}

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Set(args) => cmd_set(args.target.as_deref(), args.dry_run, args.reverse, args.largest),
        Commands::Save(args) => cmd_save(&args.name, args.dry_run, args.make_default),
        Commands::Remove(args) => cmd_remove(&args.name, args.dry_run),
        Commands::Cycle(args) => cmd_cycle(args.dry_run),
        Commands::List => cmd_list(),
        Commands::Current => cmd_current(),
        Commands::Detected => cmd_detected(),
        Commands::Fingerprint => cmd_fingerprint(),
    }
}

fn cmd_list() -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles = store.list()?;

    if profiles.is_empty() {
        println!("No profiles saved");
        return Ok(());
    }

    let state = StateStore::new()?.load_state()?.unwrap_or_default();

    println!("Profiles:");
    for profile in &profiles {
        let is_default = state.default_profile.as_ref() == Some(&profile.name);
        let is_active = state.last_profile.as_ref() == Some(&profile.name);
        let mut flags = Vec::new();
        if is_default {
            flags.push("default");
        }
        if is_active {
            flags.push("active");
        }

        println!(
            "  {} (priority: {}){}",
            profile.name,
            profile.priority,
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
    let profiles: Vec<_> = store
        .list()?
        .into_iter()
        .map(|p| with_inferred_match_rules(&p))
        .collect();
    let backend = connect_backend()?;
    let topology = backend.current_state()?;
    let state = StateStore::new()?.load_state()?.unwrap_or_default();

    if let Some(last_profile) = state.last_profile {
        println!("Current profile: {}", last_profile);
    } else if let Some(matched) = Matcher::match_profile(&topology, &profiles) {
        println!("Current profile: {}", matched.profile.name);
    } else {
        println!("Current profile: none");
    }

    Ok(())
}

fn cmd_detected() -> Result<()> {
    let backend = connect_backend()?;
    let topology = backend.enumerate_outputs()?;
    print_topology("Detected outputs:", &topology);
    Ok(())
}

fn cmd_fingerprint() -> Result<()> {
    let backend = connect_backend()?;
    let topology = backend.enumerate_outputs()?;
    println!("{}", topology.fingerprint());
    Ok(())
}

fn cmd_save(name: &str, dry_run: bool, make_default: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let backend = connect_backend()?;
    let topology = backend.enumerate_outputs()?;

    if topology.outputs.is_empty() {
        bail!("cannot save a profile from an empty topology")
    }

    let profile = Profile {
        name: name.to_string(),
        priority: 0,
        match_rules: topology
            .outputs
            .values()
            .filter(|output| !output.identity.is_ignored && !output.identity.is_virtual)
            .map(|output| OutputMatcher {
                identity: output.identity.clone(),
                required: output.enabled,
                position_hint: Some(output.position),
            })
            .collect(),
        layout: topology
            .outputs
            .iter()
            .map(|(output_name, output)| (output_name.clone(), output.clone().into()))
            .collect(),
        hooks: Default::default(),
        options: Default::default(),
    };

    if dry_run {
        println!("Would save profile '{}':", name);
        print_plan_summary(&LayoutPlan {
            outputs: profile
                .layout
                .iter()
                .map(|(output_name, config)| (output_name.clone(), config.state.clone()))
                .collect(),
            preset_used: None,
        });
        if make_default {
            println!("Would also set '{}' as the default profile", name);
        }
        return Ok(());
    }

    store.save(&profile)?;
    if make_default {
        let state_store = StateStore::new()?;
        let mut state = state_store.load_state()?.unwrap_or_default();
        state.default_profile = Some(name.to_string());
        state_store.save_state(&state)?;
    }
    println!("Saved profile '{}'", name);
    if make_default {
        println!("Set '{}' as default profile", name);
    }
    Ok(())
}

fn cmd_set(name: Option<&str>, dry_run: bool, reverse: bool, largest: bool) -> Result<()> {
    if name.is_none() {
        if reverse {
            bail!("--reverse requires a virtual 'horizontal' or 'vertical' set target")
        }
        if largest {
            bail!("--largest requires the virtual 'common' set target")
        }
        return cmd_change(dry_run);
    }

    let name = name.expect("checked above");
    if let Some(preset) = resolve_virtual_preset(name, reverse, largest)? {
        return execute_virtual_action(&preset, dry_run);
    }

    let store = ProfileStore::new()?;
    let profile = store
        .get(name)?
        .ok_or_else(|| anyhow!("profile '{}' not found", name))?;
    execute_profile_action(&profile, dry_run)
}

fn cmd_change(dry_run: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let backend = connect_backend()?;
    let topology = backend.enumerate_outputs()?;
    let profile = select_profile_for_topology(&topology, &store, &StateStore::new()?)?
        .ok_or_else(|| anyhow!("no matching profile and no default profile configured"))?;
    execute_profile_action(&profile, dry_run)
}

fn cmd_remove(name: &str, dry_run: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let exists = store.get(name)?.is_some();

    if dry_run {
        if exists {
            println!("Would remove profile '{}'", name);
        } else {
            println!("Profile '{}' not found", name);
        }
        return Ok(());
    }

    if store.remove(name)? {
        println!("Removed profile '{}'", name);
    } else {
        println!("Profile '{}' not found", name);
    }
    Ok(())
}

fn cmd_cycle(dry_run: bool) -> Result<()> {
    let store = ProfileStore::new()?;
    let profiles = store.list()?;
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
    let topology = backend.enumerate_outputs()?;
    let plan = Planner::plan_from_preset(preset, &topology, None)?;
    let test = backend.test(&plan);

    if dry_run {
        println!("Dry run for virtual configuration '{}':", preset);
        print_plan_summary(&plan);
        print_validation_result(&test);
        return Ok(());
    }

    let test = test?;

    if !test.success {
        bail!(test.message.unwrap_or_else(|| "backend rejected configuration".to_string()));
    }

    let applied = backend.apply(&plan)?;
    if !applied.success {
        bail!(applied.message.unwrap_or_else(|| "backend failed to apply configuration".to_string()));
    }

    let applied_topology = applied.applied_state.unwrap_or(topology);
    save_runtime_state(preset, Some("wlroots"), &applied_topology)?;

    println!("Set virtual configuration '{}'", preset);
    print_plan_summary(&plan);
    Ok(())
}

fn execute_profile_action(profile: &Profile, dry_run: bool) -> Result<()> {
    validate_profile(profile)?;
    let backend = connect_backend()?;
    let topology = backend.enumerate_outputs()?;
    let result = resolve_profile_plan(profile, &topology)?;
    let plan = Planner::plan_from_profile(&result, &topology)?;
    let test = backend.test(&plan);

    if dry_run {
        println!("Dry run for profile '{}':", profile.name);
        print_plan_summary(&plan);
        print_validation_result(&test);
        return Ok(());
    }

    let test = test?;

    if !test.success {
        bail!(test.message.unwrap_or_else(|| "backend rejected configuration".to_string()));
    }

    let applied = backend.apply(&plan)?;
    if !applied.success {
        bail!(applied.message.unwrap_or_else(|| "backend failed to apply configuration".to_string()));
    }

    let applied_topology = applied.applied_state.unwrap_or_else(|| topology.clone());
    save_runtime_state(&profile.name, Some("wlroots"), &applied_topology)?;

    println!("Set profile '{}'", profile.name);
    print_plan_summary(&plan);
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

fn connect_backend() -> Result<waytorandr_wlroots::WlrootsBackend> {
    waytorandr_wlroots::WlrootsBackend::connect()
        .context("failed to connect to wlroots output-management backend")
}

fn resolve_virtual_preset(name: &str, reverse: bool, largest: bool) -> Result<Option<String>> {
    let preset = match name {
        "off" => Some(name.to_string()),
        "common" => Some(if largest {
            "common-largest".to_string()
        } else {
            "common".to_string()
        }),
        "mirror" => bail!(mirror_unavailable_message()),
        "horizontal" | "vertical" => Some(if reverse {
            format!("{}-reverse", name)
        } else {
            name.to_string()
        }),
        _ => None,
    };

    if reverse && preset.is_none() {
        bail!("--reverse can only be used with virtual 'horizontal' or 'vertical' set targets")
    }

    if largest && preset.is_none() {
        bail!("--largest can only be used with virtual 'common' set targets")
    }

    if largest && !matches!(name, "common") {
        bail!("--largest can only be used with virtual 'common' set targets")
    }

    Ok(preset)
}

fn mirror_unavailable_message() -> &'static str {
    "true display mirroring is not available through generic wlroots output-management today; use 'wl-mirror' for now. See https://github.com/swaywm/wlr-protocols/issues/101"
}

fn complete_set_targets(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let mut candidates: Vec<_> = virtual_completion_candidates(current)
        .into_iter()
        .chain(saved_profile_completion_candidates(current))
        .collect();
    candidates.sort();
    candidates
}

fn complete_saved_profiles(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let mut candidates = saved_profile_completion_candidates(current);
    candidates.sort();
    candidates
}

fn virtual_completion_candidates(current: &str) -> Vec<CompletionCandidate> {
    [
        ("off", "virtual"),
        ("common", "virtual"),
        ("mirror", "virtual"),
        ("horizontal", "virtual"),
        ("vertical", "virtual"),
    ]
    .into_iter()
    .filter(|(name, _)| name.starts_with(current))
    .map(|(name, tag)| CompletionCandidate::new(name).tag(Some(tag.into())))
    .collect()
}

fn saved_profile_completion_candidates(current: &str) -> Vec<CompletionCandidate> {
    ProfileStore::new()
        .and_then(|store| store.list())
        .unwrap_or_default()
        .into_iter()
        .filter(|profile| profile.name.starts_with(current))
        .map(|profile| CompletionCandidate::new(profile.name).tag(Some("profile".into())))
        .collect()
}

fn resolve_profile_plan(profile: &Profile, topology: &Topology) -> Result<MatchResult> {
    let profile = with_inferred_match_rules(profile);
    let profile_name = profile.name.clone();
    Matcher::match_profile(topology, &[profile])
        .ok_or_else(|| anyhow!("profile '{}' does not match the current topology", profile_name))
}

fn with_inferred_match_rules(profile: &Profile) -> Profile {
    if !profile.match_rules.is_empty() {
        return profile.clone();
    }

    let mut inferred = profile.clone();
    inferred.match_rules = profile
        .layout
        .values()
        .map(|config| OutputMatcher {
            identity: config.state.identity.clone(),
            required: config.state.enabled,
            position_hint: Some(config.state.position),
        })
        .collect();
    inferred
}

fn select_profile_for_topology(
    topology: &Topology,
    store: &ProfileStore,
    state_store: &StateStore,
) -> Result<Option<Profile>> {
    let profiles: Vec<_> = store
        .list()?
        .into_iter()
        .map(|p| with_inferred_match_rules(&p))
        .collect();
    if let Some(matched) = Matcher::match_profile(topology, &profiles) {
        return Ok(Some(matched.profile));
    }

    let state = state_store.load_state()?.unwrap_or_default();
    if let Some(default_name) = state.default_profile {
        return store.get(&default_name);
    }

    Ok(None)
}

fn save_runtime_state(profile_name: &str, backend: Option<&str>, topology: &Topology) -> Result<()> {
    let state_store = StateStore::new()?;
    let mut state = state_store.load_state()?.unwrap_or_default();
    state.last_profile = Some(profile_name.to_string());
    state.last_topology_fingerprint = Some(topology.fingerprint());
    state.backend = backend.map(str::to_string);
    state_store.save_state(&state)?;
    Ok(())
}

fn print_topology(title: &str, topology: &Topology) {
    println!("{title}");
    if topology.outputs.is_empty() {
        println!("  (no outputs detected)");
        return;
    }

    let mut outputs: Vec<_> = topology.outputs.iter().collect();
    outputs.sort_by(|a, b| a.0.cmp(b.0));

    for (name, state) in outputs {
        println!(
            "  {}: {} at ({},{}) scale {} mode {}",
            name,
            if state.enabled { "enabled" } else { "disabled" },
            state.position.x,
            state.position.y,
            state.scale,
            format_mode(state.mode)
        );
        if let Some(description) = &state.identity.description {
            println!("    description: {}", description);
        }
        if let Some(make) = &state.identity.make {
            println!("    make: {}", make);
        }
        if let Some(model) = &state.identity.model {
            println!("    model: {}", model);
        }
        if let Some(serial) = &state.identity.serial {
            println!("    serial: {}", serial);
        }
    }
}

fn print_plan_summary(plan: &LayoutPlan) {
    let mut outputs: Vec<_> = plan.outputs.iter().collect();
    outputs.sort_by(|a, b| a.0.cmp(b.0));
    for (name, state) in outputs {
        println!(
            "  {} -> {} at ({},{}) scale {} mode {} transform {}{}",
            name,
            if state.enabled { "enabled" } else { "disabled" },
            state.position.x,
            state.position.y,
            state.scale,
            format_mode(state.mode),
            state.transform,
            state
                .mirror_target
                .as_deref()
                .map(|target| format!(" mirror {}", target))
                .unwrap_or_default(),
        );
    }
}

fn print_validation_result(test: &Result<waytorandr_core::TestResult>) {
    match test {
        Ok(test) => println!(
            "Backend validation: {}{}",
            if test.success { "ok" } else { "failed" },
            test.message
                .as_deref()
                .map(|msg| format!(" ({msg})"))
                .unwrap_or_default()
        ),
        Err(err) => println!("Backend validation: failed ({})", err),
    }
}

fn format_mode(mode: Option<waytorandr_core::Mode>) -> String {
    mode.map(|mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh))
        .unwrap_or_else(|| "no mode".to_string())
}
