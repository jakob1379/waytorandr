use anyhow::Result;
use serde::Serialize;

use super::output::{heading, key, print_topology, status_label, value, warning, write_json};
use super::shared::{load_current_topology, topology_outputs, JsonOutputEntry};
use super::{version_text, OutputMode};
use waytorandr_core::model::{OutputIdentity, Topology};
use waytorandr_core::state::StateStore;
use waytorandr_core::store::{DefaultTarget, ProfileStore, ProfilesSettings, StoredProfile};
use waytorandr_core::workflow;

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
    fingerprint: String,
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
    fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    builtin_output: Option<OutputIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_setup_default: Option<DefaultTarget>,
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
    fingerprint: String,
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
    new_setup_default: Option<DefaultTarget>,
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
            fingerprint: view.topology.fingerprint(),
            setup_name: view.setup_name.clone(),
            builtin_output: view.builtin_output.clone(),
            new_setup_default: view.new_setup_default.clone(),
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
    if let Some(default_target) = &view.new_setup_default {
        println!(
            "{}: {}",
            key("Default for new setups"),
            value(describe_default_target(default_target))
        );
    }
    print_topology("Detected outputs:", &view.topology);
    print_profile_setups(&view.list, view.has_saved_profiles);

    Ok(())
}

fn load_status_view(show_all: bool) -> Result<StatusView> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let topology = load_current_topology(&state_store)?;
    let state = state_store.load_state()?.unwrap_or_default();
    let settings = store.settings()?;
    let current_setup = topology.setup_fingerprint();
    let current_setup_name = state
        .setup_name_for_setup(&current_setup)
        .map(str::to_string);
    let current_profiles = store.profiles_for_setup(&current_setup, &state_store)?;
    let all_profiles = store.list(&state_store)?;
    let has_saved_profiles = !all_profiles.is_empty();
    let listed_profiles: Vec<StoredProfile> = if show_all {
        all_profiles
    } else {
        store.list_for_setup(&current_setup, &state_store)?
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
        Some(current_setup),
        active_profile.as_deref(),
        &state,
        &settings,
    );

    Ok(StatusView {
        profile: active_profile,
        topology,
        setup_name: current_setup_name,
        builtin_output: settings.builtin_output.clone(),
        new_setup_default: settings.new_setup_default,
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
    current_setup: Option<String>,
    active_profile: Option<&str>,
    state: &waytorandr_core::state::State,
    settings: &ProfilesSettings,
) -> ListView {
    let mut setups = Vec::new();
    let mut current_fingerprint: Option<String> = None;
    let mut current_profiles: Vec<ListProfileView> = Vec::new();

    for stored in listed_profiles {
        if current_fingerprint.as_deref() != Some(stored.setup_fingerprint.as_str()) {
            if let Some(fingerprint) = current_fingerprint.take() {
                setups.push(ListSetupView {
                    setup_name: state.setup_name_for_setup(&fingerprint).map(str::to_string),
                    is_current: current_setup.as_deref() == Some(fingerprint.as_str()),
                    fingerprint,
                    profiles: current_profiles,
                });
                current_profiles = Vec::new();
            }
            current_fingerprint = Some(stored.setup_fingerprint.clone());
        }

        current_profiles.push(ListProfileView {
            name: stored.name.clone(),
            priority: stored.priority,
            is_default: settings.setup_default_profile(&stored.setup_fingerprint)
                == Some(stored.name.as_str()),
            is_active: current_setup.as_deref() == Some(stored.setup_fingerprint.as_str())
                && active_profile == Some(stored.name.as_str()),
        });
    }

    if let Some(fingerprint) = current_fingerprint {
        setups.push(ListSetupView {
            setup_name: state.setup_name_for_setup(&fingerprint).map(str::to_string),
            is_current: current_setup.as_deref() == Some(fingerprint.as_str()),
            fingerprint,
            profiles: current_profiles,
        });
    }

    ListView { show_all, setups }
}

fn describe_default_target(target: &DefaultTarget) -> String {
    match target {
        DefaultTarget::Profile { name } => format!("saved profile '{name}'"),
        DefaultTarget::Virtual { preset } => format!("virtual '{preset}'"),
    }
}

fn json_list_setups(setups: &[ListSetupView]) -> Vec<JsonListSetup> {
    setups
        .iter()
        .map(|setup| JsonListSetup {
            setup_name: setup.setup_name.clone(),
            fingerprint: setup.fingerprint.clone(),
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
                key("fingerprint"),
                setup.fingerprint,
                if setup.is_current { " [current]" } else { "" }
            ),
            None => println!(
                "  {}: {}{}",
                key("fingerprint"),
                setup.fingerprint,
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
mod tests {
    use super::*;
    use std::collections::HashMap;
    use waytorandr_core::model::{OutputState, Topology};

    fn output(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state
    }

    #[test]
    fn build_list_view_groups_profiles_by_setup() {
        let mut state = waytorandr_core::state::State::default();
        state
            .setup_names
            .insert("setup-a".to_string(), "office".to_string());
        let mut settings = ProfilesSettings::default();
        settings.set_setup_default_profile("setup-a", "desk");
        let listed_profiles = vec![
            ListEntry {
                setup_fingerprint: "setup-a".to_string(),
                name: "desk".to_string(),
                priority: 1,
            },
            ListEntry {
                setup_fingerprint: "setup-a".to_string(),
                name: "laptop".to_string(),
                priority: 2,
            },
            ListEntry {
                setup_fingerprint: "setup-b".to_string(),
                name: "dock".to_string(),
                priority: 3,
            },
        ];

        let view = build_list_view(
            &listed_profiles,
            true,
            Some("setup-a".to_string()),
            Some("desk"),
            &state,
            &settings,
        );

        assert_eq!(view.setups.len(), 2);
        assert!(view.setups[0].is_current);
        assert_eq!(view.setups[0].setup_name.as_deref(), Some("office"));
        assert_eq!(view.setups[0].profiles.len(), 2);
        assert!(view.setups[0].profiles[0].is_default);
        assert!(view.setups[0].profiles[0].is_active);
    }

    #[test]
    fn build_list_view_scopes_active_profile_to_current_setup() {
        let state = waytorandr_core::state::State::default();
        let listed_profiles = vec![
            ListEntry {
                setup_fingerprint: "setup-a".to_string(),
                name: "default".to_string(),
                priority: 0,
            },
            ListEntry {
                setup_fingerprint: "setup-b".to_string(),
                name: "default".to_string(),
                priority: 0,
            },
        ];

        let view = build_list_view(
            &listed_profiles,
            true,
            Some("setup-a".to_string()),
            Some("default"),
            &state,
            &ProfilesSettings::default(),
        );

        assert!(view.setups[0].profiles[0].is_active);
        assert!(!view.setups[1].profiles[0].is_active);
    }

    #[test]
    fn json_detected_outputs_are_sorted() {
        let topology = Topology {
            outputs: HashMap::from([
                ("eDP-1".to_string(), output("eDP-1")),
                ("DP-1".to_string(), output("DP-1")),
            ]),
        };

        let outputs = topology_outputs(&topology);
        let outputs = serde_json::to_value(outputs).unwrap();

        assert_eq!(outputs.as_array().unwrap().len(), 2);
        assert_eq!(outputs[0]["name"], "DP-1");
        assert_eq!(outputs[1]["name"], "eDP-1");
    }
}
