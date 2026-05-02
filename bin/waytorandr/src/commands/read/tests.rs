use super::*;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use waytorandr_core::{OutputIdentity, OutputState, Position, ReadOnlyStateStore, Topology};

fn read_output_state(connector: &str) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = true;
    state.position = Position::new(0, 0);
    state.identity = OutputIdentity::new(connector);
    state
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn scoped_env_var(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> ScopedEnvVar {
    let previous = std::env::var_os(key);
    std::env::set_var(key, value);
    ScopedEnvVar { key, previous }
}

struct ReadStatusTestEnv {
    _root: tempfile::TempDir,
    _config_home: ScopedEnvVar,
    _state_home: ScopedEnvVar,
    _backend_state: ScopedEnvVar,
    _backend_name: ScopedEnvVar,
    config_home: PathBuf,
    state_home: PathBuf,
}

impl ReadStatusTestEnv {
    fn new() -> anyhow::Result<Self> {
        let root = tempfile::tempdir()?;
        let config_home = root.path().join("config");
        let state_home = root.path().join("state");
        std::fs::create_dir_all(&config_home)?;
        std::fs::create_dir_all(&state_home)?;
        let backend_state_path = state_home.join("test-backend.json");

        Ok(Self {
            _config_home: scoped_env_var("XDG_CONFIG_HOME", &config_home),
            _state_home: scoped_env_var("XDG_STATE_HOME", &state_home),
            _backend_state: scoped_env_var("WAYTORANDR_TEST_BACKEND_STATE", &backend_state_path),
            _backend_name: scoped_env_var("WAYTORANDR_TEST_BACKEND_NAME", "test"),
            _root: root,
            config_home,
            state_home,
        })
    }

    fn state_store_dir(&self) -> PathBuf {
        self.state_home.join("waytorandr")
    }

    fn config_store_dir(&self) -> PathBuf {
        self.config_home.join("waytorandr")
    }

    fn state_file(&self) -> PathBuf {
        self.state_store_dir().join("state.toml")
    }

    fn profiles_file(&self) -> PathBuf {
        self.config_store_dir().join("waytorandr.json")
    }
}

#[test]
fn build_list_view_groups_profiles_by_setup() {
    let mut state = waytorandr_core::State::default();
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
        Some("setup-a"),
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
    let state = waytorandr_core::State::default();
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
        Some("setup-a"),
        Some("default"),
        &state,
        &ProfilesSettings::default(),
    );

    assert!(view.setups[0].profiles[0].is_active);
    assert!(!view.setups[1].profiles[0].is_active);
}

#[test]
fn json_detected_outputs_are_sorted() -> anyhow::Result<()> {
    let topology = Topology {
        outputs: HashMap::from([
            ("eDP-1".to_string(), read_output_state("eDP-1")),
            ("DP-1".to_string(), read_output_state("DP-1")),
        ]),
    };

    let outputs = topology_outputs(&topology);
    let outputs = serde_json::to_value(outputs)?;

    let Some(outputs_array) = outputs.as_array() else {
        anyhow::bail!("outputs should serialize as an array");
    };
    assert_eq!(outputs_array.len(), 2);
    assert_eq!(outputs[0]["name"], "DP-1");
    assert_eq!(outputs[1]["name"], "eDP-1");
    Ok(())
}

#[test]
fn load_status_view_does_not_create_store_files_for_read_only_status() -> anyhow::Result<()> {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let env = ReadStatusTestEnv::new()?;
    let topology = Topology {
        outputs: HashMap::from([("eDP-1".to_string(), read_output_state("eDP-1"))]),
    };
    let store: ReadOnlyProfileStore = ProfileStore::open_read_only()?;
    let state_store = ReadOnlyStateStore::open()?;

    let view = build_status_view(false, topology, &store, &state_store)?;

    assert!(view.topology.outputs.contains_key("eDP-1"));
    assert!(!env.state_store_dir().exists());
    assert!(!env.config_store_dir().exists());
    assert!(!env.state_file().exists());
    assert!(!env.profiles_file().exists());
    Ok(())
}
