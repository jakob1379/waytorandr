use crate::error::{CoreError, CoreResult};
use crate::model::{OutputIdentity, Topology};
use crate::profile::Profile;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ProfileStore {
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StoredProfile {
    pub profile: Profile,
    pub setup_fingerprint: String,
    path: PathBuf,
}

pub struct StateStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn new() -> CoreResult<Self> {
        let dir = profile_dir()?;
        fs::create_dir_all(&dir).map_err(|source| CoreError::CreateDir {
            path: dir.clone(),
            source,
        })?;
        let store = Self { dir };
        store.migrate_legacy_profiles()?;
        Ok(store)
    }

    pub fn open_read_only() -> CoreResult<Self> {
        Ok(Self { dir: profile_dir()? })
    }

    pub fn list(&self) -> CoreResult<Vec<StoredProfile>> {
        let mut profiles = Vec::new();
        let state = StateStore::new()?.load_state()?.unwrap_or_default();
        if !self.dir.exists() {
            return Ok(profiles);
        }

        for entry in fs::read_dir(&self.dir).map_err(|source| CoreError::ReadDir {
            path: self.dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| CoreError::ReadDir {
                path: self.dir.clone(),
                source,
            })?;
            let path = entry.path();
            if path.is_dir() {
                let setup_fingerprint = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
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
                        .map(|e| e == "toml")
                        .unwrap_or(false)
                    {
                        let mut profile =
                            self.load_stored_profile(&nested_path, setup_fingerprint.clone())?;
                        profile.profile = canonicalize_profile(&profile.profile, &state.known_outputs);
                        profile.setup_fingerprint = profile.profile.setup_fingerprint();
                        profiles.push(profile)
                    }
                }
            }
        }

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
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut names = std::collections::BTreeSet::new();
        for entry in fs::read_dir(&self.dir).map_err(|source| CoreError::ReadDir {
            path: self.dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| CoreError::ReadDir {
                path: self.dir.clone(),
                source,
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

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
                    if let Some(name) = nested_path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_string)
                    {
                        names.insert(name);
                    }
                }
            }
        }

        Ok(names.into_iter().collect())
    }

    pub fn get_in_setup(
        &self,
        name: &str,
        setup_fingerprint: &str,
    ) -> CoreResult<Option<StoredProfile>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|stored| {
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

    pub fn save(&self, profile: &Profile, setup_fingerprint: &str) -> CoreResult<()> {
        let dir = self.dir.join(setup_fingerprint);
        fs::create_dir_all(&dir).map_err(|source| CoreError::CreateDir {
            path: dir.clone(),
            source,
        })?;
        let path = dir.join(format!("{}.toml", profile.name));
        save_profile_to_file(profile, &path)?;

        tracing::info!("Saved profile '{}' to {:?}", profile.name, path);
        Ok(())
    }

    pub fn remove_in_setup(&self, name: &str, setup_fingerprint: &str) -> CoreResult<bool> {
        if let Some(stored) = self.get_in_setup(name, setup_fingerprint)? {
            fs::remove_file(&stored.path).map_err(|source| CoreError::WriteFile {
                path: stored.path.clone(),
                source,
            })?;
            tracing::info!("Removed profile '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn remove_unique(&self, name: &str) -> CoreResult<bool> {
        if let Some(stored) = self.get_unique(name)? {
            fs::remove_file(&stored.path).map_err(|source| CoreError::WriteFile {
                path: stored.path.clone(),
                source,
            })?;
            tracing::info!("Removed profile '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn load_stored_profile(
        &self,
        path: &Path,
        setup_fingerprint: Option<String>,
    ) -> CoreResult<StoredProfile> {
        let profile = load_profile_from_file(path)?;
        Ok(StoredProfile {
            setup_fingerprint: setup_fingerprint.unwrap_or_else(|| profile.setup_fingerprint()),
            profile,
            path: path.to_path_buf(),
        })
    }

    fn migrate_legacy_profiles(&self) -> CoreResult<()> {
        for entry in fs::read_dir(&self.dir).map_err(|source| CoreError::ReadDir {
            path: self.dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| CoreError::ReadDir {
                path: self.dir.clone(),
                source,
            })?;
            let legacy_path = entry.path();
            if legacy_path.is_dir()
                || !legacy_path
                    .extension()
                    .map(|extension| extension == "toml")
                    .unwrap_or(false)
            {
                continue;
            }

            let profile = load_profile_from_file(&legacy_path)?;
            let setup_fingerprint = profile.setup_fingerprint();
            let setup_dir = self.dir.join(&setup_fingerprint);
            fs::create_dir_all(&setup_dir).map_err(|source| CoreError::CreateDir {
                path: setup_dir.clone(),
                source,
            })?;

            let setup_path = setup_dir.join(format!("{}.toml", profile.name));
            if setup_path.exists() {
                let setup_profile = load_profile_from_file(&setup_path)?;
                let same_profile = setup_profile.name == profile.name
                    && setup_profile.setup_fingerprint() == setup_fingerprint
                    && setup_profile.layout_fingerprint() == profile.layout_fingerprint();

                if same_profile {
                    fs::remove_file(&legacy_path).map_err(|source| CoreError::WriteFile {
                        path: legacy_path.clone(),
                        source,
                    })?;
                    continue;
                }

                return Err(CoreError::LegacyProfileConflict {
                    name: profile.name,
                    legacy_path,
                    setup_path,
                });
            }

            fs::rename(&legacy_path, &setup_path).map_err(|source| CoreError::WriteFile {
                path: legacy_path,
                source,
            })?;
        }

        Ok(())
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
        let mut state: State = toml::from_str(&content).map_err(|source| CoreError::ParseToml {
            path,
            source,
        })?;
        if state.migrate_legacy_default_profile() {
            self.save_state(&state)?;
        }
        Ok(Some(state))
    }

    pub fn normalize_topology(&self, topology: &Topology) -> CoreResult<Topology> {
        let state = self.load_state()?.unwrap_or_default();
        Ok(normalize_topology_with_cache(topology, &state.known_outputs))
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

fn profile_dir() -> CoreResult<PathBuf> {
    let config_home = directories::BaseDirs::new()
        .ok_or(CoreError::MissingConfigDirectory)?
        .config_dir()
        .join("waytorandr")
        .join("profiles");
    Ok(config_home)
}

fn save_profile_to_file(profile: &Profile, path: &Path) -> CoreResult<()> {
    let content = toml::to_string_pretty(profile)?;
    fs::write(path, content).map_err(|source| CoreError::WriteFile {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
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
