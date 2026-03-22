use crate::profile::{default_profile_dir, Profile};
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
    pub fn new() -> anyhow::Result<Self> {
        let dir = default_profile_dir()?;
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn list(&self) -> anyhow::Result<Vec<StoredProfile>> {
        let mut profiles = Vec::new();
        if !self.dir.exists() {
            return Ok(profiles);
        }

        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let setup_fingerprint = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
                for nested in fs::read_dir(&path)? {
                    let nested = nested?;
                    let nested_path = nested.path();
                    if nested_path
                        .extension()
                        .map(|e| e == "toml")
                        .unwrap_or(false)
                    {
                        match self.load_stored_profile(&nested_path, setup_fingerprint.clone()) {
                            Ok(profile) => profiles.push(profile),
                            Err(e) => {
                                tracing::warn!("Failed to load profile {:?}: {}", nested_path, e)
                            }
                        }
                    }
                }
            } else if path.extension().map(|e| e == "toml").unwrap_or(false) {
                match self.load_stored_profile(&path, None) {
                    Ok(profile) => profiles.push(profile),
                    Err(e) => tracing::warn!("Failed to load profile {:?}: {}", path, e),
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

    pub fn list_for_setup(&self, setup_fingerprint: &str) -> anyhow::Result<Vec<StoredProfile>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|stored| stored.setup_fingerprint == setup_fingerprint)
            .collect())
    }

    pub fn get(
        &self,
        name: &str,
        setup_fingerprint: Option<&str>,
    ) -> anyhow::Result<Option<StoredProfile>> {
        let mut candidates: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|stored| stored.profile.name == name)
            .collect();

        if let Some(setup_fingerprint) = setup_fingerprint {
            candidates.retain(|stored| stored.setup_fingerprint == setup_fingerprint);
        }

        match candidates.len() {
            0 => Ok(None),
            1 => Ok(candidates.into_iter().next()),
            _ => anyhow::bail!("profile '{}' is ambiguous across setup fingerprints; use the matching hardware setup", name),
        }
    }

    pub fn save(&self, profile: &Profile, setup_fingerprint: &str) -> anyhow::Result<()> {
        let dir = self.dir.join(setup_fingerprint);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", profile.name));
        profile.save_to_file(&path)?;

        let legacy_path = self.profile_path(&profile.name);
        if legacy_path.exists() {
            if let Ok(legacy_profile) = Profile::load_from_file(&legacy_path) {
                if legacy_profile.setup_fingerprint() == setup_fingerprint {
                    let _ = fs::remove_file(&legacy_path);
                }
            }
        }

        tracing::info!("Saved profile '{}' to {:?}", profile.name, path);
        Ok(())
    }

    pub fn remove(&self, name: &str, setup_fingerprint: Option<&str>) -> anyhow::Result<bool> {
        if let Some(stored) = self.get(name, setup_fingerprint)? {
            fs::remove_file(&stored.path)?;
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
    ) -> anyhow::Result<StoredProfile> {
        let profile = Profile::load_from_file(path)?;
        Ok(StoredProfile {
            setup_fingerprint: setup_fingerprint.unwrap_or_else(|| profile.setup_fingerprint()),
            profile,
            path: path.to_path_buf(),
        })
    }

    fn profile_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.toml", name))
    }
}

impl StateStore {
    pub fn new() -> anyhow::Result<Self> {
        let state_dir = directories::BaseDirs::new()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine state directory"))?
            .state_dir()
            .map(|p| p.join("waytorandr"))
            .ok_or_else(|| anyhow::anyhow!("Cannot determine state directory path"))?;

        fs::create_dir_all(&state_dir)?;
        Ok(Self { dir: state_dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_state(&self, state: &State) -> anyhow::Result<()> {
        let path = self.dir.join("state.toml");
        let content = toml::to_string_pretty(state)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn load_state(&self) -> anyhow::Result<Option<State>> {
        let path = self.dir.join("state.toml");
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let state: State = toml::from_str(&content)?;
        Ok(Some(state))
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    pub last_profile: Option<String>,
    pub last_topology_fingerprint: Option<String>,
    pub default_profile: Option<String>,
    #[serde(default)]
    pub default_profiles: std::collections::HashMap<String, String>,
    pub backend: Option<String>,
    pub daemon_enabled: bool,
}
