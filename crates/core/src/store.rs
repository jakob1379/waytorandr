use crate::error::CoreResult;
use crate::model::OutputIdentity;
use crate::profile::Profile;
use crate::state::{ReadOnlyStateStore, State, StateReader, StateStore};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod legacy;
mod migration;
mod persistence;
mod query;
mod read_only;
mod write;

use legacy::profiles_path;

pub struct ProfileStore {
    path: PathBuf,
}

pub struct ReadOnlyProfileStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileQueryContext {
    known_outputs: HashMap<String, OutputIdentity>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StoredProfile {
    pub profile: Profile,
    pub setup_fingerprint: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<Profile>,
    #[serde(default)]
    settings: ProfilesSettings,
}

#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProfilesSettings {
    #[serde(default)]
    pub setup_defaults: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_output: Option<OutputIdentity>,
}

impl ProfilesSettings {
    #[must_use]
    pub fn setup_default_profile(&self, setup_fingerprint: &str) -> Option<&str> {
        self.setup_defaults
            .get(setup_fingerprint)
            .map(String::as_str)
    }

    pub fn set_setup_default_profile(&mut self, setup_fingerprint: &str, profile_name: &str) {
        self.setup_defaults
            .insert(setup_fingerprint.to_string(), profile_name.to_string());
    }

    fn clear_setup_default_if_matches(&mut self, setup_fingerprint: &str, profile_name: &str) {
        if self.setup_default_profile(setup_fingerprint) == Some(profile_name) {
            self.setup_defaults.remove(setup_fingerprint);
        }
    }

    fn clear_all_profile_references(&mut self, profile_name: &str) {
        self.setup_defaults
            .retain(|_, stored_name| stored_name != profile_name);
    }
}

impl ProfileQueryContext {
    #[must_use]
    pub fn from_state(state: &State) -> Self {
        Self {
            known_outputs: state.known_outputs.clone(),
        }
    }

    /// Loads a profile query context from writable state storage.
    ///
    /// # Errors
    /// Returns an error if state storage cannot be read or parsed.
    pub fn load(state_store: &StateStore) -> CoreResult<Self> {
        Self::load_from(state_store)
    }

    /// Loads a profile query context from read-only state storage.
    ///
    /// # Errors
    /// Returns an error if state storage cannot be read or parsed.
    pub fn load_read_only(state_store: &ReadOnlyStateStore) -> CoreResult<Self> {
        Self::load_from(state_store)
    }

    /// Loads a profile query context from any read-capable state store.
    ///
    /// # Errors
    /// Returns an error if state storage cannot be read or parsed.
    pub fn load_from(state_store: &impl StateReader) -> CoreResult<Self> {
        let state = state_store.load_state()?.unwrap_or_default();
        Ok(Self::from_state(&state))
    }

    #[must_use]
    pub fn known_outputs(&self) -> &HashMap<String, OutputIdentity> {
        &self.known_outputs
    }
}

impl ProfileStore {
    /// Opens the profile store.
    ///
    /// # Errors
    /// Returns an error if the config directory cannot be determined.
    pub fn open() -> CoreResult<Self> {
        let path = profiles_path()?;
        Ok(Self { path })
    }

    /// Creates the profile store directory if needed.
    ///
    /// # Errors
    /// Returns an error if the store directory cannot be created or migrated.
    pub fn bootstrap() -> CoreResult<Self> {
        migration::bootstrap_profile_store(Self::open()?)
    }

    /// Opens a read-only profile store.
    ///
    /// # Errors
    /// Returns an error if the config directory cannot be determined.
    pub fn open_read_only() -> CoreResult<ReadOnlyProfileStore> {
        Ok(ReadOnlyProfileStore {
            path: profiles_path()?,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}
