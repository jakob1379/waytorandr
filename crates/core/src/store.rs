use std::fs;
use std::path::{Path, PathBuf};
use crate::profile::{Profile, default_profile_dir};

pub struct ProfileStore {
    dir: PathBuf,
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

    pub fn list(&self) -> anyhow::Result<Vec<Profile>> {
        let mut profiles = Vec::new();
        if !self.dir.exists() {
            return Ok(profiles);
        }

        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "toml").unwrap_or(false) {
                match Profile::load_from_file(&path) {
                    Ok(profile) => profiles.push(profile),
                    Err(e) => tracing::warn!("Failed to load profile {:?}: {}", path, e),
                }
            }
        }

        profiles.sort_by(|a, b| b.priority.cmp(&a.priority));
        Ok(profiles)
    }

    pub fn get(&self, name: &str) -> anyhow::Result<Option<Profile>> {
        let path = self.profile_path(name);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Profile::load_from_file(&path)?))
    }

    pub fn save(&self, profile: &Profile) -> anyhow::Result<()> {
        let path = self.profile_path(&profile.name);
        profile.save_to_file(&path)?;
        tracing::info!("Saved profile '{}' to {:?}", profile.name, path);
        Ok(())
    }

    pub fn remove(&self, name: &str) -> anyhow::Result<bool> {
        let path = self.profile_path(name);
        if path.exists() {
            fs::remove_file(&path)?;
            tracing::info!("Removed profile '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
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
    pub backend: Option<String>,
    pub daemon_enabled: bool,
}
