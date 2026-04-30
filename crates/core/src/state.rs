use crate::atomic::{atomic_write, with_exclusive_lock};
use crate::error::{CoreError, CoreResult};
use crate::model::{BackendKind, OutputIdentity, Topology};
use crate::normalize::normalize_topology_with_known_outputs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct StateStore {
    dir: PathBuf,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct State {
    pub last_profile: Option<String>,
    pub last_topology_fingerprint: Option<String>,
    #[serde(default)]
    pub default_profiles: HashMap<String, String>,
    #[serde(default, rename = "default_profile", skip_serializing)]
    legacy_default_profile: Option<String>,
    #[serde(default)]
    pub setup_names: HashMap<String, String>,
    #[serde(default)]
    pub known_outputs: HashMap<String, OutputIdentity>,
    #[serde(default)]
    pub remembered_setups: HashMap<String, Topology>,
    pub backend: Option<BackendKind>,
    pub daemon_enabled: bool,
}

impl State {
    pub const GLOBAL_DEFAULT_PROFILE_KEY: &'static str = "__global__";

    #[must_use]
    pub fn global_default_profile(&self) -> Option<&str> {
        self.default_profiles
            .get(Self::GLOBAL_DEFAULT_PROFILE_KEY)
            .map(String::as_str)
    }

    #[must_use]
    pub fn setup_default_profile(&self, setup_fingerprint: &str) -> Option<&str> {
        self.default_profiles
            .get(setup_fingerprint)
            .map(String::as_str)
    }

    #[must_use]
    pub fn effective_default_profile_for_setup(&self, setup_fingerprint: &str) -> Option<&str> {
        self.setup_default_profile(setup_fingerprint)
            .or_else(|| self.global_default_profile())
    }

    #[must_use]
    pub fn remembered_topology_for_setup(&self, setup_fingerprint: &str) -> Option<&Topology> {
        self.remembered_setups.get(setup_fingerprint)
    }

    fn migrate_legacy_default_profile(&mut self) -> bool {
        let Some(profile_name) = self.legacy_default_profile.take() else {
            return false;
        };
        self.default_profiles
            .entry(Self::GLOBAL_DEFAULT_PROFILE_KEY.to_string())
            .or_insert(profile_name);
        true
    }

    pub fn record_applied_profile(
        &mut self,
        profile_name: &str,
        backend: Option<BackendKind>,
        topology: &Topology,
    ) {
        self.last_profile = Some(profile_name.to_string());
        self.last_topology_fingerprint = Some(topology.fingerprint());
        if topology.has_enabled_real_outputs() {
            self.remembered_setups
                .insert(topology.setup_fingerprint(), topology.clone());
        }
        self.backend = backend;
    }

    pub fn record_observed_topology(&mut self, backend: Option<BackendKind>, topology: &Topology) {
        self.last_profile = None;
        self.last_topology_fingerprint = Some(topology.fingerprint());
        if topology.has_enabled_real_outputs() {
            self.remembered_setups
                .insert(topology.setup_fingerprint(), topology.clone());
        }
        self.backend = backend;
    }

    pub fn set_default_profile_for_setup(&mut self, setup_fingerprint: &str, profile_name: &str) {
        self.default_profiles
            .insert(setup_fingerprint.to_string(), profile_name.to_string());
    }

    #[must_use]
    pub fn setup_name_for_setup(&self, setup_fingerprint: &str) -> Option<&str> {
        self.setup_names.get(setup_fingerprint).map(String::as_str)
    }

    pub fn set_setup_name_for_setup(&mut self, setup_fingerprint: &str, setup_name: &str) {
        self.setup_names
            .insert(setup_fingerprint.to_string(), setup_name.to_string());
    }

    pub fn record_daemon_started(&mut self, backend: BackendKind) {
        self.daemon_enabled = true;
        self.backend = Some(backend);
    }
}

impl StateStore {
    /// Opens the state directory.
    ///
    /// # Errors
    /// Returns an error if the base state directory cannot be determined.
    pub fn open() -> CoreResult<Self> {
        let state_dir = directories::BaseDirs::new()
            .ok_or(CoreError::MissingStateDirectory)?
            .state_dir()
            .map(|p| p.join("waytorandr"))
            .ok_or(CoreError::MissingStateDirectoryPath)?;

        Ok(Self { dir: state_dir })
    }

    /// Creates the state directory if needed.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created.
    pub fn bootstrap() -> CoreResult<Self> {
        let store = Self::open()?;
        fs::create_dir_all(&store.dir).map_err(|source| CoreError::CreateDir {
            path: store.dir.clone(),
            source,
        })?;
        Ok(store)
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Saves the current state.
    ///
    /// # Errors
    /// Returns an error if the state file cannot be written.
    pub fn save_state(&self, state: &State) -> CoreResult<()> {
        with_exclusive_lock(&self.state_path(), || self.save_state_unlocked(state))
    }

    fn save_state_unlocked(&self, state: &State) -> CoreResult<()> {
        let path = self.dir.join("state.toml");
        let content = toml::to_string_pretty(state)?;
        atomic_write(&path, content.as_bytes())
    }

    /// Loads the current state.
    ///
    /// # Errors
    /// Returns an error if the state file cannot be read or parsed.
    pub fn load_state(&self) -> CoreResult<Option<State>> {
        self.load_state_unlocked()
    }

    fn load_state_unlocked(&self) -> CoreResult<Option<State>> {
        let path = self.state_path();
        if path.exists() {
            let content = fs::read_to_string(&path).map_err(|source| CoreError::ReadFile {
                path: path.clone(),
                source,
            })?;
            let mut state: State =
                toml::from_str(&content).map_err(|source| CoreError::ParseToml { path, source })?;
            state.migrate_legacy_default_profile();
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    /// Updates state under the store lock.
    ///
    /// # Errors
    /// Returns an error if state loading, mutation, locking, or saving fails.
    pub fn update_state<T>(
        &self,
        update: impl FnOnce(&mut State) -> CoreResult<T>,
    ) -> CoreResult<T> {
        with_exclusive_lock(&self.state_path(), || {
            let mut state = self.load_state_unlocked()?.unwrap_or_default();
            let result = update(&mut state)?;
            self.save_state_unlocked(&state)?;
            Ok(result)
        })
    }

    /// Updates state after normalizing and caching observed topology under one lock.
    ///
    /// # Errors
    /// Returns an error if state loading, mutation, locking, or saving fails.
    pub fn update_observed_topology<T>(
        &self,
        topology: &Topology,
        update: impl FnOnce(&mut State, &Topology) -> CoreResult<T>,
    ) -> CoreResult<T> {
        self.update_state(|state| {
            let normalized = normalize_topology_with_known_outputs(topology, &state.known_outputs);
            for (name, output) in &normalized.outputs {
                if state.known_outputs.get(name.as_str()) != Some(&output.identity) {
                    state
                        .known_outputs
                        .insert(name.clone(), output.identity.clone());
                }
            }
            update(state, &normalized)
        })
    }

    /// Observes a topology and updates cached outputs.
    ///
    /// # Errors
    /// Returns an error if state loading, saving, or persistence fails.
    pub fn observe_topology_and_persist_known_outputs(
        &self,
        topology: &Topology,
    ) -> CoreResult<Topology> {
        self.update_observed_topology(topology, |_, normalized| Ok(normalized.clone()))
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OutputState;
    use std::collections::HashMap;

    #[test]
    fn deserializes_legacy_backend_strings() {
        let state: State = toml::from_str(
            r#"
backend = "wlroots"
daemon_enabled = false
[default_profiles]
[known_outputs]
[remembered_setups]
"#,
        )
        .expect("legacy state should load");

        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert!(state.setup_names.is_empty());
    }

    #[test]
    fn record_observed_topology_skips_blank_real_output_layouts() {
        let mut state = State {
            last_profile: Some("desk".to_string()),
            ..State::default()
        };

        let mut disabled = OutputState::new("DP-1");
        disabled.enabled = false;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), disabled)]),
        };

        state.record_observed_topology(Some(BackendKind::Wlroots), &topology);

        assert_eq!(state.last_profile, None);
        assert_eq!(state.last_topology_fingerprint.as_deref(), Some("DP-1:off"));
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert!(state.remembered_setups.is_empty());
    }

    #[test]
    fn record_applied_profile_skips_blank_real_output_layouts() {
        let mut state = State::default();

        let mut disabled = OutputState::new("DP-1");
        disabled.enabled = false;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), disabled)]),
        };

        state.record_applied_profile("desk", Some(BackendKind::Wlroots), &topology);

        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        assert_eq!(state.last_topology_fingerprint.as_deref(), Some("DP-1:off"));
        assert!(!state
            .remembered_setups
            .contains_key(&topology.setup_fingerprint()));
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
    }
}
