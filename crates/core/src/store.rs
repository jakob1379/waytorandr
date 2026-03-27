use crate::error::{CoreError, CoreResult};
use crate::model::{OutputIdentity, Topology};
use crate::profile::Profile;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ProfileStore {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StoredProfile {
    pub profile: Profile,
    pub setup_fingerprint: String,
}

pub struct StateStore {
    dir: PathBuf,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<Profile>,
}

impl ProfileStore {
    pub fn new() -> CoreResult<Self> {
        let path = profiles_path()?;
        let dir = path
            .parent()
            .ok_or(CoreError::MissingConfigDirectory)?
            .to_path_buf();
        fs::create_dir_all(&dir).map_err(|source| CoreError::CreateDir { path: dir, source })?;
        let store = Self { path };
        store.migrate_legacy_profiles()?;
        Ok(store)
    }

    pub fn open_read_only() -> CoreResult<Self> {
        Ok(Self {
            path: profiles_path()?,
        })
    }

    pub fn list(&self) -> CoreResult<Vec<StoredProfile>> {
        let state = StateStore::new()?.load_state()?.unwrap_or_default();
        let mut profiles: Vec<_> = self
            .load_profiles()?
            .into_iter()
            .map(|profile| {
                let profile = canonicalize_profile(&profile, &state.known_outputs);
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

    pub fn list_for_setup(&self, setup_fingerprint: &str) -> CoreResult<Vec<StoredProfile>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|stored| stored.setup_fingerprint == setup_fingerprint)
            .collect())
    }

    pub fn profiles(&self) -> CoreResult<Vec<Profile>> {
        Ok(self
            .list()?
            .into_iter()
            .map(|stored| stored.profile)
            .collect())
    }

    pub fn profiles_for_setup(&self, setup_fingerprint: &str) -> CoreResult<Vec<Profile>> {
        Ok(self
            .list_for_setup(setup_fingerprint)?
            .into_iter()
            .map(|stored| stored.profile)
            .collect())
    }

    pub fn list_names(&self) -> CoreResult<Vec<String>> {
        let mut names = std::collections::BTreeSet::new();
        for profile in self.load_profiles()? {
            names.insert(profile.name);
        }

        Ok(names.into_iter().collect())
    }

    pub fn get_in_setup(
        &self,
        name: &str,
        setup_fingerprint: &str,
    ) -> CoreResult<Option<StoredProfile>> {
        Ok(self.list()?.into_iter().find(|stored| {
            stored.profile.name == name && stored.setup_fingerprint == setup_fingerprint
        }))
    }

    pub fn get_unique(&self, name: &str) -> CoreResult<Option<StoredProfile>> {
        let candidates: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|stored| stored.profile.name == name)
            .collect();

        match candidates.len() {
            0 => Ok(None),
            1 => Ok(candidates.into_iter().next()),
            _ => Err(CoreError::AmbiguousProfile(name.to_string())),
        }
    }

    pub fn save(&self, profile: &Profile, _setup_fingerprint: &str) -> CoreResult<()> {
        let state = StateStore::new()?.load_state()?.unwrap_or_default();
        let setup_fingerprint =
            canonicalize_profile(profile, &state.known_outputs).setup_fingerprint();
        let mut stored = self.load_profiles_file()?;
        stored.profiles.retain(|existing| {
            !(existing.name == profile.name
                && canonicalize_profile(existing, &state.known_outputs).setup_fingerprint()
                    == setup_fingerprint)
        });
        stored.profiles.push(profile.clone());
        self.save_profiles_file(&stored)?;

        tracing::info!("Saved profile '{}' to {:?}", profile.name, self.path);
        Ok(())
    }

    pub fn remove_in_setup(&self, name: &str, setup_fingerprint: &str) -> CoreResult<bool> {
        let state = StateStore::new()?.load_state()?.unwrap_or_default();
        let mut stored = self.load_profiles_file()?;
        let original_len = stored.profiles.len();
        stored.profiles.retain(|profile| {
            !(profile.name == name
                && canonicalize_profile(profile, &state.known_outputs).setup_fingerprint()
                    == setup_fingerprint)
        });

        if stored.profiles.len() != original_len {
            self.save_profiles_file(&stored)?;
            tracing::info!("Removed profile '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

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

        if stored.profiles.len() != original_len {
            self.save_profiles_file(&stored)?;
            tracing::info!("Removed profile '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_profiles(&self) -> CoreResult<Vec<Profile>> {
        if self.path.exists() {
            return Ok(self.load_profiles_file()?.profiles);
        }

        self.load_legacy_profiles()
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

    fn load_profiles_file(&self) -> CoreResult<ProfilesFile> {
        if !self.path.exists() {
            return Ok(ProfilesFile::default());
        }

        let content = fs::read_to_string(&self.path).map_err(|source| CoreError::ReadFile {
            path: self.path.clone(),
            source,
        })?;
        serde_json::from_str(&content).map_err(|source| CoreError::ParseJson {
            path: self.path.clone(),
            source,
        })
    }

    fn save_profiles_file(&self, profiles: &ProfilesFile) -> CoreResult<()> {
        let content = serde_json::to_string_pretty(profiles).map_err(CoreError::SerializeJson)?;
        fs::write(&self.path, format!("{content}\n")).map_err(|source| CoreError::WriteFile {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }

    fn load_legacy_profiles(&self) -> CoreResult<Vec<Profile>> {
        let legacy_dir = legacy_profile_dir()?;
        if !legacy_dir.exists() {
            return Ok(Vec::new());
        }

        Ok(load_legacy_profiles_from_dir(&legacy_dir)?
            .into_iter()
            .map(|(_, profile)| profile)
            .collect())
    }
}

impl StateStore {
    pub fn new() -> CoreResult<Self> {
        let state_dir = directories::BaseDirs::new()
            .ok_or(CoreError::MissingStateDirectory)?
            .state_dir()
            .map(|p| p.join("waytorandr"))
            .ok_or(CoreError::MissingStateDirectoryPath)?;

        fs::create_dir_all(&state_dir).map_err(|source| CoreError::CreateDir {
            path: state_dir.clone(),
            source,
        })?;
        Ok(Self { dir: state_dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_state(&self, state: &State) -> CoreResult<()> {
        let path = self.dir.join("state.toml");
        let content = toml::to_string_pretty(state)?;
        fs::write(&path, content).map_err(|source| CoreError::WriteFile {
            path: path.clone(),
            source,
        })?;
        Ok(())
    }

    pub fn load_state(&self) -> CoreResult<Option<State>> {
        let path = self.dir.join("state.toml");
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|source| CoreError::ReadFile {
            path: path.clone(),
            source,
        })?;
        let mut state: State =
            toml::from_str(&content).map_err(|source| CoreError::ParseToml { path, source })?;
        if state.migrate_legacy_default_profile() {
            self.save_state(&state)?;
        }
        Ok(Some(state))
    }

    pub fn normalize_topology(&self, topology: &Topology) -> CoreResult<Topology> {
        let state = self.load_state()?.unwrap_or_default();
        Ok(normalize_topology_with_cache(
            topology,
            &state.known_outputs,
        ))
    }

    pub fn normalize_topology_and_persist(&self, topology: &Topology) -> CoreResult<Topology> {
        let mut state = self.load_state()?.unwrap_or_default();
        let mut normalized = normalize_topology_with_cache(topology, &state.known_outputs);
        let mut changed = false;

        for (name, output) in &mut normalized.outputs {
            if state.known_outputs.get(name) != Some(&output.identity) {
                state
                    .known_outputs
                    .insert(name.clone(), output.identity.clone());
                changed = true;
            }
        }

        if changed {
            self.save_state(&state)?;
        }

        Ok(normalized)
    }

    pub fn normalize_profile(&self, profile: &Profile) -> CoreResult<Profile> {
        let state = self.load_state()?.unwrap_or_default();
        Ok(canonicalize_profile(profile, &state.known_outputs))
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    pub last_profile: Option<String>,
    pub last_topology_fingerprint: Option<String>,
    #[serde(default)]
    pub default_profiles: std::collections::HashMap<String, String>,
    #[serde(default, rename = "default_profile", skip_serializing)]
    legacy_default_profile: Option<String>,
    #[serde(default)]
    pub known_outputs: std::collections::HashMap<String, OutputIdentity>,
    pub backend: Option<String>,
    pub daemon_enabled: bool,
}

impl State {
    pub const GLOBAL_DEFAULT_PROFILE_KEY: &'static str = "__global__";

    pub fn global_default_profile(&self) -> Option<&str> {
        self.default_profiles
            .get(Self::GLOBAL_DEFAULT_PROFILE_KEY)
            .map(String::as_str)
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
}

fn normalize_profile_with_cache(
    profile: &Profile,
    known_outputs: &std::collections::HashMap<String, OutputIdentity>,
) -> Profile {
    let mut normalized = profile.clone();

    for matcher in &mut normalized.match_rules {
        if let Some(connector) = matcher.identity.connector.as_deref() {
            if let Some(cached) = known_outputs.get(connector) {
                matcher.identity = matcher.identity.with_fallback(cached);
            }
        }
    }

    for (connector, config) in &mut normalized.layout {
        if let Some(cached) = known_outputs.get(connector) {
            config.state.identity = config.state.identity.with_fallback(cached);
        } else if let Some(connector) = config.state.identity.connector.as_deref() {
            if let Some(cached) = known_outputs.get(connector) {
                config.state.identity = config.state.identity.with_fallback(cached);
            }
        }
    }

    normalized
}

fn normalize_topology_with_cache(
    topology: &Topology,
    known_outputs: &std::collections::HashMap<String, OutputIdentity>,
) -> Topology {
    let mut normalized = topology.clone();

    for (name, output) in &mut normalized.outputs {
        if let Some(cached) = known_outputs.get(name) {
            output.identity = output.identity.with_fallback(cached);
        }
    }

    normalized
}

fn config_dir() -> CoreResult<PathBuf> {
    Ok(directories::BaseDirs::new()
        .ok_or(CoreError::MissingConfigDirectory)?
        .config_dir()
        .join("waytorandr"))
}

fn profiles_path() -> CoreResult<PathBuf> {
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

fn canonicalize_profile(
    profile: &Profile,
    known_outputs: &std::collections::HashMap<String, OutputIdentity>,
) -> Profile {
    normalize_profile_with_cache(&profile.with_inferred_match_rules(), known_outputs)
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
                    .map(|extension| extension == "toml")
                    .unwrap_or(false)
                {
                    profiles.push((nested_path.clone(), load_profile_from_file(&nested_path)?));
                }
            }
            continue;
        }

        if path
            .extension()
            .map(|extension| extension == "toml")
            .unwrap_or(false)
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
