use anyhow::Result;
use serde::Serialize;

use super::output::{heading, key, print_topology, status_label, value, warning, write_json};
use super::shared::{load_current_topology, topology_outputs, JsonOutputEntry};
use super::{version_text, OutputMode};
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::workflow;
use waytorandr_core::{OutputIdentity, Topology};
use waytorandr_core::{ProfileQueryContext, ReadOnlyStateStore};
use waytorandr_core::{ProfileStore, ProfilesSettings, ReadOnlyProfileStore, StoredProfile};

#[derive(Serialize)]
struct JsonListProfile {
    name: String,
    priority: u32,
    is_default: bool,
    is_active: bool,
}

#[derive(Serialize)]
struct JsonListSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_name: Option<String>,
    setup_fingerprint: String,
    is_current: bool,
    profiles: Vec<JsonListProfile>,
}

#[derive(Serialize)]
struct JsonStatusResponse {
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    show_all: bool,
    has_saved_profiles: bool,
    topology_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    builtin_output: Option<OutputIdentity>,
    setup_fingerprint: String,
    outputs: Vec<JsonOutputEntry>,
    setups: Vec<JsonListSetup>,
}

#[derive(Serialize)]
struct JsonVersionResponse {
    command: &'static str,
    name: &'static str,
    version: &'static str,
}

struct ListProfileView {
    name: String,
    priority: u32,
    is_default: bool,
    is_active: bool,
}

struct ListEntry {
    setup_fingerprint: String,
    name: String,
    priority: u32,
}

struct ListSetupView {
    setup_name: Option<String>,
    setup_fingerprint: String,
    is_current: bool,
    profiles: Vec<ListProfileView>,
}

struct ListView {
    show_all: bool,
    setups: Vec<ListSetupView>,
}

struct StatusView {
    profile: Option<String>,
    topology: Topology,
    setup_name: Option<String>,
    builtin_output: Option<OutputIdentity>,
    list: ListView,
    has_saved_profiles: bool,
}

pub(super) fn cmd_status(show_all: bool, output_mode: OutputMode) -> Result<()> {
    let view = load_status_view(show_all)?;

    if output_mode.is_json() {
        return write_json(&JsonStatusResponse {
            command: "status",
            profile: view.profile.clone(),
            show_all: view.list.show_all,
            has_saved_profiles: view.has_saved_profiles,
            topology_fingerprint: view.topology.fingerprint(),
            setup_name: view.setup_name.clone(),
            builtin_output: view.builtin_output.clone(),
            setup_fingerprint: view.topology.setup_fingerprint(),
            outputs: topology_outputs(&view.topology),
            setups: json_list_setups(&view.list.setups),
        });
    }

    println!(
        "{}: {}",
        key("Current profile"),
        view.profile
            .as_deref()
            .map_or_else(|| status_label("none"), value)
    );
    if let Some(setup_name) = view.setup_name.as_deref() {
        println!(
            "{}: {} ({}: {})",
            key("Setup"),
            value(setup_name),
            key("fingerprint"),
            view.topology.setup_fingerprint()
        );
    } else {
        println!(
            "{}: {}",
            key("Setup fingerprint"),
            view.topology.setup_fingerprint()
        );
    }
    if let Some(builtin_output) = &view.builtin_output {
        println!(
            "{}: {}",
            key("Builtin display override"),
            value(builtin_output.primary_key())
        );
    }
    print_topology("Detected outputs:", &view.topology);
    print_profile_setups(&view.list, view.has_saved_profiles);

    Ok(())
}

fn load_status_view(show_all: bool) -> Result<StatusView> {
    let store: ReadOnlyProfileStore = ProfileStore::open_read_only()?;
    let state_store = ReadOnlyStateStore::open()?;
    let backend = connect_backend()?;
    let topology = load_current_topology(backend.as_ref(), &state_store)?;
    build_status_view(show_all, topology, &store, &state_store)
}

fn build_status_view(
    show_all: bool,
    topology: Topology,
    store: &ReadOnlyProfileStore,
    state_store: &ReadOnlyStateStore,
) -> Result<StatusView> {
    let state = state_store.load_state()?.unwrap_or_default();
    let query_context = ProfileQueryContext::from_state(&state);
    let settings = store.settings()?;
    let current_setup = topology.setup_fingerprint();
    let current_setup_name = state
        .setup_name_for_setup(&current_setup)
        .map(str::to_string);
    let current_profiles = store.profiles_for_setup(&current_setup, &query_context)?;
    let all_profiles = store.list(&query_context)?;
    let has_saved_profiles = !all_profiles.is_empty();
    let listed_profiles: Vec<StoredProfile> = if show_all {
        all_profiles
    } else {
        store.list_for_setup(&current_setup, &query_context)?
    };

    let entries: Vec<ListEntry> = listed_profiles
        .into_iter()
        .map(|stored| ListEntry {
            setup_fingerprint: stored.setup_fingerprint,
            name: stored.profile.name,
            priority: stored.profile.priority,
        })
        .collect();
    let active_profile = workflow::current_profile_name(&topology, &current_profiles, &state);
    let list = build_list_view(
        &entries,
        show_all,
        Some(current_setup.as_str()),
        active_profile.as_deref(),
        &state,
        &settings,
    );

    Ok(StatusView {
        profile: active_profile,
        topology,
        setup_name: current_setup_name,
        builtin_output: settings.builtin_output.clone(),
        list,
        has_saved_profiles,
    })
}

pub(super) fn cmd_version(output_mode: OutputMode) -> Result<()> {
    if output_mode.is_json() {
        return write_json(&JsonVersionResponse {
            command: "version",
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
        });
    }

    println!("{}", version_text());
    Ok(())
}

fn build_list_view(
    listed_profiles: &[ListEntry],
    show_all: bool,
    current_setup: Option<&str>,
    active_profile: Option<&str>,
    state: &waytorandr_core::State,
    settings: &ProfilesSettings,
) -> ListView {
    let mut setups = Vec::new();
    let mut current_setup_fingerprint: Option<String> = None;
    let mut current_profiles: Vec<ListProfileView> = Vec::new();

    for stored in listed_profiles {
        if current_setup_fingerprint.as_deref() != Some(stored.setup_fingerprint.as_str()) {
            if let Some(setup_fingerprint) = current_setup_fingerprint.take() {
                push_setup_view(
                    &mut setups,
                    setup_fingerprint,
                    std::mem::take(&mut current_profiles),
                    current_setup,
                    state,
                );
            }
            current_setup_fingerprint = Some(stored.setup_fingerprint.clone());
        }

        current_profiles.push(ListProfileView {
            name: stored.name.clone(),
            priority: stored.priority,
            is_default: settings.setup_default_profile(&stored.setup_fingerprint)
                == Some(stored.name.as_str()),
            is_active: current_setup == Some(stored.setup_fingerprint.as_str())
                && active_profile == Some(stored.name.as_str()),
        });
    }

    if let Some(setup_fingerprint) = current_setup_fingerprint {
        push_setup_view(
            &mut setups,
            setup_fingerprint,
            current_profiles,
            current_setup,
            state,
        );
    }

    ListView { show_all, setups }
}

fn push_setup_view(
    setups: &mut Vec<ListSetupView>,
    setup_fingerprint: String,
    profiles: Vec<ListProfileView>,
    current_setup: Option<&str>,
    state: &waytorandr_core::State,
) {
    setups.push(ListSetupView {
        setup_name: state
            .setup_name_for_setup(&setup_fingerprint)
            .map(str::to_string),
        is_current: current_setup == Some(setup_fingerprint.as_str()),
        setup_fingerprint,
        profiles,
    });
}

fn json_list_setups(setups: &[ListSetupView]) -> Vec<JsonListSetup> {
    setups
        .iter()
        .map(|setup| JsonListSetup {
            setup_name: setup.setup_name.clone(),
            setup_fingerprint: setup.setup_fingerprint.clone(),
            is_current: setup.is_current,
            profiles: setup
                .profiles
                .iter()
                .map(|profile| JsonListProfile {
                    name: profile.name.clone(),
                    priority: profile.priority,
                    is_default: profile.is_default,
                    is_active: profile.is_active,
                })
                .collect(),
        })
        .collect()
}

fn print_profile_setups(view: &ListView, has_saved_profiles: bool) {
    if view.setups.is_empty() {
        if has_saved_profiles {
            println!("{}", warning("Profiles: none for current setup"));
        } else {
            println!("{}", warning("Profiles: none saved"));
        }
        return;
    }

    println!("{}", heading("Profiles:"));
    for setup in &view.setups {
        match setup.setup_name.as_deref() {
            Some(setup_name) => println!(
                "  {}: {} ({}: {}){}",
                key("setup"),
                value(setup_name),
                key("setup fingerprint"),
                setup.setup_fingerprint,
                if setup.is_current { " [current]" } else { "" }
            ),
            None => println!(
                "  {}: {}{}",
                key("setup fingerprint"),
                setup.setup_fingerprint,
                if setup.is_current { " [current]" } else { "" }
            ),
        }
        for profile in &setup.profiles {
            let mut flags = Vec::new();
            if profile.is_default {
                flags.push("default");
            }
            if profile.is_active {
                flags.push("active");
            }

            println!(
                "    {} ({}: {}){}",
                value(&profile.name),
                key("priority"),
                profile.priority,
                if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", flags.join(", "))
                }
            );
        }
    }
}

#[cfg(test)]
mod tests;
