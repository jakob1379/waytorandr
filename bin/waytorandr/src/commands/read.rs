use anyhow::Result;
use serde::Serialize;

use super::output::{print_topology, write_json};
use super::shared::{load_current_topology, topology_outputs, JsonOutputEntry};
use super::{version_text, OutputMode};
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::model::Topology;
use waytorandr_core::state::StateStore;
use waytorandr_core::store::{ProfileStore, StoredProfile};
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
    fingerprint: String,
    is_current: bool,
    profiles: Vec<ListProfileView>,
}

struct ListView {
    show_all: bool,
    current_setup: Option<String>,
    setups: Vec<ListSetupView>,
}

pub(super) fn cmd_list(show_all: bool, output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;

    let state_store = StateStore::bootstrap()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let profiles = store.list_with_known_outputs(&state.known_outputs)?;
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
        store.list_for_setup_with_known_outputs(setup, &state.known_outputs)?
    } else {
        Vec::new()
    };

    if listed_profiles.is_empty() && !output_mode.is_json() {
        println!("No profiles match the current topology");
        if let Some(setup) = &current_setup {
            println!("Current fingerprint: {setup}");
        }
        return Ok(());
    }

    let entries: Vec<ListEntry> = listed_profiles
        .into_iter()
        .map(|stored| ListEntry {
            setup_fingerprint: stored.setup_fingerprint,
            name: stored.profile.name,
            priority: stored.profile.priority,
        })
        .collect();
    let view = build_list_view(&entries, show_all, current_setup, &state);

    if output_mode.is_json() {
        return write_json(&JsonListResponse {
            command: "list",
            show_all: view.show_all,
            current_setup: view.current_setup,
            setups: view
                .setups
                .into_iter()
                .map(|setup| JsonListSetup {
                    fingerprint: setup.fingerprint,
                    is_current: setup.is_current,
                    profiles: setup
                        .profiles
                        .into_iter()
                        .map(|profile| JsonListProfile {
                            name: profile.name,
                            priority: profile.priority,
                            is_default: profile.is_default,
                            is_active: profile.is_active,
                        })
                        .collect(),
                })
                .collect(),
        });
    }

    println!("Profiles:");
    for setup in view.setups {
        println!(
            "  fingerprint: {}{}",
            setup.fingerprint,
            if setup.is_current { " [current]" } else { "" }
        );
        for profile in setup.profiles {
            let mut flags = Vec::new();
            if profile.is_default {
                flags.push("default");
            }
            if profile.is_active {
                flags.push("active");
            }

            println!(
                "    {} (priority: {}){}",
                profile.name,
                profile.priority,
                if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", flags.join(", "))
                }
            );
        }
    }

    Ok(())
}

pub(super) fn cmd_current(output_mode: OutputMode) -> Result<()> {
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let state = state_store.load_state()?.unwrap_or_default();
    let profiles = store.profiles_with_known_outputs(&state.known_outputs)?;
    let backend = connect_backend()?;
    let topology = workflow::normalized_topology_from_backend(backend.as_ref(), &state_store)?;

    let current = workflow::current_profile_name(&topology, &profiles, &state);
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

pub(super) fn cmd_detected(output_mode: OutputMode) -> Result<()> {
    let state_store = StateStore::bootstrap()?;
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
    state: &waytorandr_core::state::State,
) -> ListView {
    let mut setups = Vec::new();
    let mut current_fingerprint: Option<String> = None;
    let mut current_profiles: Vec<ListProfileView> = Vec::new();

    for stored in listed_profiles {
        if current_fingerprint.as_deref() != Some(stored.setup_fingerprint.as_str()) {
            if let Some(fingerprint) = current_fingerprint.take() {
                setups.push(ListSetupView {
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
            is_default: workflow::default_profile_for_setup(state, &stored.setup_fingerprint)
                == Some(stored.name.as_str()),
            is_active: state.last_profile.as_ref() == Some(&stored.name),
        });
    }

    if let Some(fingerprint) = current_fingerprint {
        setups.push(ListSetupView {
            is_current: current_setup.as_deref() == Some(fingerprint.as_str()),
            fingerprint,
            profiles: current_profiles,
        });
    }

    ListView {
        show_all,
        current_setup,
        setups,
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
        let state = waytorandr_core::state::State::default();
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

        let view = build_list_view(&listed_profiles, true, Some("setup-a".to_string()), &state);

        assert_eq!(view.setups.len(), 2);
        assert!(view.setups[0].is_current);
        assert_eq!(view.setups[0].profiles.len(), 2);
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
