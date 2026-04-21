use crate::error::{CoreError, CoreResult};
use crate::model::{OutputIdentity, VirtualPreset};
use crate::normalize::normalize_profile_with_known_outputs;
use crate::profile::Profile;
use crate::state::{State, StateStore};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ProfileStore {
    path: PathBuf,
}

pub struct ReadOnlyProfileStore {
    path: PathBuf,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProfilesSettings {
    #[serde(default)]
    pub setup_defaults: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_output: Option<OutputIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_setup_default: Option<DefaultTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefaultTarget {
    Profile { name: String },
    Virtual { preset: VirtualPreset },
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

    fn clear_new_setup_default_if_profile_matches(&mut self, profile_name: &str) {
        if matches!(
            self.new_setup_default.as_ref(),
            Some(DefaultTarget::Profile { name }) if name == profile_name
        ) {
            self.new_setup_default = None;
        }
    }

    fn clear_all_profile_references(&mut self, profile_name: &str) {
        self.setup_defaults
            .retain(|_, stored_name| stored_name != profile_name);
        self.clear_new_setup_default_if_profile_matches(profile_name);
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
        let store = Self::open()?;
        let dir = store
            .path
            .parent()
            .ok_or(CoreError::MissingConfigDirectory)?
            .to_path_buf();
        fs::create_dir_all(&dir).map_err(|source| CoreError::CreateDir { path: dir, source })?;
        store.migrate_legacy_profiles_json()?;
        store.migrate_legacy_profiles()?;
        store.migrate_legacy_defaults_from_state()?;
        Ok(store)
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

    /// Lists profiles with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read.
    pub fn list(&self, state_store: &StateStore) -> CoreResult<Vec<StoredProfile>> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.list_with_known_outputs(&state.known_outputs)
    }

    /// Lists profiles for a setup with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read.
    pub fn list_for_setup(
        &self,
        setup_fingerprint: &str,
        state_store: &StateStore,
    ) -> CoreResult<Vec<StoredProfile>> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.list_for_setup_with_known_outputs(setup_fingerprint, &state.known_outputs)
    }

    /// Returns all profiles with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read.
    pub fn profiles(&self, state_store: &StateStore) -> CoreResult<Vec<Profile>> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.profiles_with_known_outputs(&state.known_outputs)
    }

    /// Returns profiles for a setup with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read.
    pub fn profiles_for_setup(
        &self,
        setup_fingerprint: &str,
        state_store: &StateStore,
    ) -> CoreResult<Vec<Profile>> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.profiles_for_setup_with_known_outputs(setup_fingerprint, &state.known_outputs)
    }

    /// Finds a profile for a setup.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read.
    pub fn get_for_setup(
        &self,
        name: &str,
        setup_fingerprint: &str,
        state_store: &StateStore,
    ) -> CoreResult<Option<StoredProfile>> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.get_for_setup_with_known_outputs(name, setup_fingerprint, &state.known_outputs)
    }

    /// Returns stored settings.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    pub fn settings(&self) -> CoreResult<ProfilesSettings> {
        Ok(self.load_profiles_file()?.settings)
    }

    /// Sets the default saved profile for a setup fingerprint.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read or written.
    pub fn set_setup_default_profile(
        &self,
        setup_fingerprint: &str,
        profile_name: &str,
    ) -> CoreResult<()> {
        let mut stored = self.load_profiles_file()?;
        stored
            .settings
            .set_setup_default_profile(setup_fingerprint, profile_name);
        self.save_profiles_file(&stored)
    }

    /// Sets the default target used for new setups.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read or written.
    pub fn set_new_setup_default(&self, target: DefaultTarget) -> CoreResult<()> {
        let mut stored = self.load_profiles_file()?;
        stored.settings.new_setup_default = Some(target);
        self.save_profiles_file(&stored)
    }

    /// Lists profiles with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    fn list_with_known_outputs(
        &self,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<StoredProfile>> {
        let mut profiles: Vec<_> = self
            .load_profiles()?
            .into_iter()
            .map(|profile| {
                let profile = normalize_profile_with_known_outputs(&profile, known_outputs);
                StoredProfile {
                    setup_fingerprint: profile.setup_fingerprint(),
                    profile,
                }
            })
            .collect();

        profiles.sort_by(|a, b| {
            a.setup_fingerprint
                .cmp(&b.setup_fingerprint)
                .then(b.profile.priority.cmp(&a.profile.priority))
                .then(a.profile.name.cmp(&b.profile.name))
        });
        Ok(profiles)
    }

    /// Lists profiles for a setup with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    fn list_for_setup_with_known_outputs(
        &self,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<StoredProfile>> {
        Ok(self
            .list_with_known_outputs(known_outputs)?
            .into_iter()
            .filter(|stored| stored.setup_fingerprint == setup_fingerprint)
            .collect())
    }

    /// Returns all profiles with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    fn profiles_with_known_outputs(
        &self,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<Profile>> {
        Ok(self
            .list_with_known_outputs(known_outputs)?
            .into_iter()
            .map(|stored| stored.profile)
            .collect())
    }

    /// Returns profiles for a setup with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    fn profiles_for_setup_with_known_outputs(
        &self,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<Profile>> {
        Ok(self
            .list_for_setup_with_known_outputs(setup_fingerprint, known_outputs)?
            .into_iter()
            .map(|stored| stored.profile)
            .collect())
    }

    /// Finds a profile for a setup.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    fn get_for_setup_with_known_outputs(
        &self,
        name: &str,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Option<StoredProfile>> {
        Ok(self
            .list_with_known_outputs(known_outputs)?
            .into_iter()
            .find(|stored| {
                stored.profile.name == name && stored.setup_fingerprint == setup_fingerprint
            }))
    }

    /// Finds a uniquely named profile.
    ///
    /// # Errors
    /// Returns an error if the name is ambiguous or storage cannot be read.
    pub fn get_unique(
        &self,
        name: &str,
        state_store: &StateStore,
    ) -> CoreResult<Option<StoredProfile>> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.get_unique_with_known_outputs(name, &state.known_outputs)
    }

    /// Saves a profile with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read or written.
    pub fn save(&self, profile: &Profile, state_store: &StateStore) -> CoreResult<()> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.save_with_known_outputs(profile, &state.known_outputs)
    }

    /// Removes a profile for a setup.
    ///
    /// # Errors
    /// Returns an error if profile storage or state storage cannot be read or written.
    pub fn remove_for_setup(
        &self,
        name: &str,
        setup_fingerprint: &str,
        state_store: &StateStore,
    ) -> CoreResult<bool> {
        let state = state_store.load_state()?.unwrap_or_default();
        self.remove_for_setup_with_known_outputs(name, setup_fingerprint, &state.known_outputs)
    }

    /// Finds a uniquely named profile.
    ///
    /// # Errors
    /// Returns an error if the name is ambiguous or storage cannot be read.
    fn get_unique_with_known_outputs(
        &self,
        name: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Option<StoredProfile>> {
        let candidates: Vec<_> = self
            .list_with_known_outputs(known_outputs)?
            .into_iter()
            .filter(|stored| stored.profile.name == name)
            .collect();

        match candidates.len() {
            0 => Ok(None),
            1 => Ok(candidates.into_iter().next()),
            _ => Err(CoreError::AmbiguousProfile(name.to_string())),
        }
    }

    /// Saves a profile with normalized state.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read or written.
    fn save_with_known_outputs(
        &self,
        profile: &Profile,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<()> {
        let setup_fingerprint =
            normalize_profile_with_known_outputs(profile, known_outputs).setup_fingerprint();
        let mut stored = self.load_profiles_file()?;
        stored.profiles.retain(|existing| {
            !(existing.name == profile.name
                && normalize_profile_with_known_outputs(existing, known_outputs)
                    .setup_fingerprint()
                    == setup_fingerprint)
        });
        stored.profiles.push(profile.clone());
        self.save_profiles_file(&stored)?;

        tracing::info!(
            "Saved profile '{name}' to {path:?}",
            name = profile.name,
            path = self.path
        );
        Ok(())
    }

    /// Removes a profile for a setup.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read or written.
    fn remove_for_setup_with_known_outputs(
        &self,
        name: &str,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<bool> {
        let mut stored = self.load_profiles_file()?;
        let original_len = stored.profiles.len();
        stored.profiles.retain(|profile| {
            !(profile.name == name
                && normalize_profile_with_known_outputs(profile, known_outputs).setup_fingerprint()
                    == setup_fingerprint)
        });

        if stored.profiles.len() == original_len {
            return Ok(false);
        }

        stored
            .settings
            .clear_setup_default_if_matches(setup_fingerprint, name);
        if !stored.profiles.iter().any(|profile| profile.name == name) {
            stored
                .settings
                .clear_new_setup_default_if_profile_matches(name);
        }
        self.save_profiles_file(&stored)?;
        tracing::info!("Removed profile '{name}'");
        Ok(true)
    }

    /// Removes a uniquely named profile.
    ///
    /// # Errors
    /// Returns an error if the name is ambiguous or storage cannot be read or written.
    pub fn remove_unique(&self, name: &str) -> CoreResult<bool> {
        let mut stored = self.load_profiles_file()?;
        let matches = stored
            .profiles
            .iter()
            .filter(|profile| profile.name == name)
            .count();

        if matches > 1 {
            return Err(CoreError::AmbiguousProfile(name.to_string()));
        }

        let original_len = stored.profiles.len();
        stored.profiles.retain(|profile| profile.name != name);

        if stored.profiles.len() == original_len {
            return Ok(false);
        }

        stored.settings.clear_all_profile_references(name);
        self.save_profiles_file(&stored)?;
        tracing::info!("Removed profile '{name}'");
        Ok(true)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_profiles(&self) -> CoreResult<Vec<Profile>> {
        Ok(self.load_profiles_file()?.profiles)
    }

    fn migrate_legacy_profiles(&self) -> CoreResult<()> {
        let legacy_dir = legacy_profile_dir()?;
        if !legacy_dir.exists() {
            return Ok(());
        }

        let mut stored = self.load_profiles_file()?;
        let mut migrated_paths = Vec::new();

        for (legacy_path, profile) in load_legacy_profiles_from_dir(&legacy_dir)? {
            merge_legacy_profile(&mut stored.profiles, profile, &legacy_path, &self.path)?;
            migrated_paths.push(legacy_path);
        }

        if migrated_paths.is_empty() {
            return Ok(());
        }

        self.save_profiles_file(&stored)?;
        for path in migrated_paths {
            fs::remove_file(&path).map_err(|source| CoreError::WriteFile {
                path: path.clone(),
                source,
            })?;
        }
        remove_empty_legacy_directories(&legacy_dir)?;

        Ok(())
    }

    fn migrate_legacy_defaults_from_state(&self) -> CoreResult<()> {
        let state_store = StateStore::bootstrap()?;
        let Some(mut state) = state_store.load_state()? else {
            return Ok(());
        };

        if state.default_profiles.is_empty() {
            return Ok(());
        }

        let mut stored = self.load_profiles_file()?;
        let mut changed = false;

        if stored.settings.new_setup_default.is_none() {
            if let Some(profile_name) = state.global_default_profile() {
                stored.settings.new_setup_default = Some(DefaultTarget::Profile {
                    name: profile_name.to_string(),
                });
                changed = true;
            }
        }

        for (setup_fingerprint, profile_name) in &state.default_profiles {
            if setup_fingerprint == State::GLOBAL_DEFAULT_PROFILE_KEY {
                continue;
            }

            if !stored
                .settings
                .setup_defaults
                .contains_key(setup_fingerprint)
            {
                stored
                    .settings
                    .set_setup_default_profile(setup_fingerprint, profile_name);
                changed = true;
            }
        }

        if changed {
            self.save_profiles_file(&stored)?;
        }

        state.default_profiles.clear();
        state_store.save_state(&state)?;
        Ok(())
    }

    fn load_profiles_file(&self) -> CoreResult<ProfilesFile> {
        load_profiles_file_from_path(&self.path)
    }

    fn save_profiles_file(&self, profiles: &ProfilesFile) -> CoreResult<()> {
        let content = serde_json::to_string_pretty(profiles).map_err(CoreError::SerializeJson)?;
        fs::write(&self.path, format!("{content}\n")).map_err(|source| CoreError::WriteFile {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }
}

impl ReadOnlyProfileStore {
    /// Lists stored profile names.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read.
    pub fn list_names(&self) -> CoreResult<Vec<String>> {
        let mut names = std::collections::BTreeSet::new();
        for profile in load_profiles_from_path(&self.path)? {
            names.insert(profile.name);
        }

        Ok(names.into_iter().collect())
    }
}

fn config_dir() -> CoreResult<PathBuf> {
    Ok(directories::BaseDirs::new()
        .ok_or(CoreError::MissingConfigDirectory)?
        .config_dir()
        .join("waytorandr"))
}

fn profiles_path() -> CoreResult<PathBuf> {
    Ok(config_dir()?.join("waytorandr.json"))
}

fn legacy_profiles_json_path() -> CoreResult<PathBuf> {
    Ok(config_dir()?.join("profiles.json"))
}

fn legacy_profile_dir() -> CoreResult<PathBuf> {
    Ok(config_dir()?.join("profiles"))
}

fn load_profile_from_file(path: &Path) -> CoreResult<Profile> {
    let content = fs::read_to_string(path).map_err(|source| CoreError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let profile = toml::from_str(&content).map_err(|source| CoreError::ParseToml {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(profile)
}

fn load_profiles_from_path(path: &Path) -> CoreResult<Vec<Profile>> {
    Ok(load_profiles_file_from_path(path)?.profiles)
}

fn load_profiles_file_from_path(path: &Path) -> CoreResult<ProfilesFile> {
    if path.exists() {
        load_profiles_json_file(path)
    } else if let Ok(legacy_path) = legacy_profiles_json_path() {
        if legacy_path.exists() {
            load_profiles_json_file(&legacy_path)
        } else {
            Ok(ProfilesFile {
                profiles: load_legacy_profiles()?,
                ..ProfilesFile::default()
            })
        }
    } else {
        Ok(ProfilesFile {
            profiles: load_legacy_profiles()?,
            ..ProfilesFile::default()
        })
    }
}

fn load_profiles_json_file(path: &Path) -> CoreResult<ProfilesFile> {
    let content = fs::read_to_string(path).map_err(|source| CoreError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&content).map_err(|source| CoreError::ParseJson {
        path: path.to_path_buf(),
        source,
    })
}

impl ProfileStore {
    fn migrate_legacy_profiles_json(&self) -> CoreResult<()> {
        let legacy_path = legacy_profiles_json_path()?;
        if self.path.exists() || !legacy_path.exists() {
            return Ok(());
        }

        match fs::rename(&legacy_path, &self.path) {
            Ok(()) => Ok(()),
            Err(source) => {
                let legacy_missing = !legacy_path.exists();
                let target_exists = self.path.exists();

                if target_exists || legacy_missing {
                    Ok(())
                } else {
                    Err(CoreError::WriteFile {
                        path: self.path.clone(),
                        source,
                    })
                }
            }
        }?;

        Ok(())
    }
}

fn load_legacy_profiles_from_dir(dir: &Path) -> CoreResult<Vec<(PathBuf, Profile)>> {
    let mut profiles = Vec::new();

    for entry in fs::read_dir(dir).map_err(|source| CoreError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| CoreError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();

        if path.is_dir() {
            for nested in fs::read_dir(&path).map_err(|source| CoreError::ReadDir {
                path: path.clone(),
                source,
            })? {
                let nested = nested.map_err(|source| CoreError::ReadDir {
                    path: path.clone(),
                    source,
                })?;
                let nested_path = nested.path();
                if nested_path
                    .extension()
                    .is_some_and(|extension| extension == "toml")
                {
                    profiles.push((nested_path.clone(), load_profile_from_file(&nested_path)?));
                }
            }
            continue;
        }

        if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            profiles.push((path.clone(), load_profile_from_file(&path)?));
        }
    }

    Ok(profiles)
}

fn merge_legacy_profile(
    stored_profiles: &mut Vec<Profile>,
    profile: Profile,
    legacy_path: &Path,
    target_path: &Path,
) -> CoreResult<()> {
    let setup_fingerprint = profile.setup_fingerprint();
    if let Some(existing) = stored_profiles.iter().find(|existing| {
        existing.name == profile.name && existing.setup_fingerprint() == setup_fingerprint
    }) {
        let same_profile = existing.layout_fingerprint() == profile.layout_fingerprint();
        if same_profile {
            return Ok(());
        }

        return Err(CoreError::LegacyProfileConflict {
            name: profile.name,
            legacy_path: legacy_path.to_path_buf(),
            setup_path: target_path.to_path_buf(),
        });
    }

    stored_profiles.push(profile);
    Ok(())
}

fn load_legacy_profiles() -> CoreResult<Vec<Profile>> {
    let legacy_dir = legacy_profile_dir()?;
    if legacy_dir.exists() {
        Ok(load_legacy_profiles_from_dir(&legacy_dir)?
            .into_iter()
            .map(|(_, profile)| profile)
            .collect())
    } else {
        Ok(Vec::new())
    }
}

fn remove_empty_legacy_directories(dir: &Path) -> CoreResult<()> {
    for entry in fs::read_dir(dir).map_err(|source| CoreError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| CoreError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            let is_empty = fs::read_dir(&path)
                .map_err(|source| CoreError::ReadDir {
                    path: path.clone(),
                    source,
                })?
                .next()
                .is_none();
            if is_empty {
                fs::remove_dir(&path).map_err(|source| CoreError::WriteFile {
                    path: path.clone(),
                    source,
                })?;
            }
        }
    }

    let is_empty = fs::read_dir(dir)
        .map_err(|source| CoreError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?
        .next()
        .is_none();
    if is_empty {
        fs::remove_dir(dir).map_err(|source| CoreError::WriteFile {
            path: dir.to_path_buf(),
            source,
        })?;
    }

    Ok(())
}
