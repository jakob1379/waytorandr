use super::ProfilesFile;
use crate::error::{CoreError, CoreResult};
use crate::profile::Profile;
use std::fs;
use std::path::{Path, PathBuf};
fn config_dir() -> CoreResult<PathBuf> {
    Ok(directories::BaseDirs::new()
        .ok_or(CoreError::MissingConfigDirectory)?
        .config_dir()
        .join("waytorandr"))
}

pub(super) fn profiles_path() -> CoreResult<PathBuf> {
    Ok(config_dir()?.join("waytorandr.json"))
}

pub(super) fn legacy_profiles_json_path() -> CoreResult<PathBuf> {
    Ok(config_dir()?.join("profiles.json"))
}

pub(super) fn legacy_profile_dir() -> CoreResult<PathBuf> {
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

pub(super) fn load_profiles_from_path(path: &Path) -> CoreResult<Vec<Profile>> {
    Ok(load_profiles_file_from_path(path)?.profiles)
}

pub(super) fn load_profiles_file_from_path(path: &Path) -> CoreResult<ProfilesFile> {
    if path.exists() {
        load_profiles_json_file(path)
    } else {
        load_legacy_profiles_file()
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

fn load_legacy_profiles_file() -> CoreResult<ProfilesFile> {
    if let Ok(legacy_path) = legacy_profiles_json_path() {
        if legacy_path.exists() {
            return load_profiles_json_file(&legacy_path);
        }
    }

    Ok(ProfilesFile {
        profiles: load_legacy_profiles()?,
        ..ProfilesFile::default()
    })
}

pub(super) fn load_legacy_profiles_from_dir(dir: &Path) -> CoreResult<Vec<(PathBuf, Profile)>> {
    legacy_profile_paths(dir)?
        .into_iter()
        .map(|path| {
            let profile = load_profile_from_file(&path)?;
            Ok((path, profile))
        })
        .collect()
}

fn legacy_profile_paths(dir: &Path) -> CoreResult<Vec<PathBuf>> {
    let mut paths = Vec::new();

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
            push_toml_profile_paths_from_dir(&path, &mut paths)?;
        } else {
            push_toml_profile_path(&mut paths, path);
        }
    }

    Ok(paths)
}

fn push_toml_profile_paths_from_dir(dir: &Path, paths: &mut Vec<PathBuf>) -> CoreResult<()> {
    for entry in fs::read_dir(dir).map_err(|source| CoreError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| CoreError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        push_toml_profile_path(paths, entry.path());
    }

    Ok(())
}

fn push_toml_profile_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path
        .extension()
        .is_some_and(|extension| extension == "toml")
    {
        paths.push(path);
    }
}

pub(super) fn merge_legacy_profile(
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

pub(super) fn remove_empty_legacy_directories(dir: &Path) -> CoreResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OutputIdentity, OutputState, Position};
    use crate::profile::{OutputMatcher, Profile};
    use std::collections::HashMap;

    fn legacy_profile(name: &str, connector: &str) -> Profile {
        let mut output = OutputState::new(connector);
        output.enabled = true;
        output.position = Position::new(0, 0);

        Profile::new(
            name,
            0,
            vec![OutputMatcher::new(
                OutputIdentity::new(connector),
                true,
                Some(Position::new(0, 0)),
            )],
            HashMap::from([(connector.to_string(), output.into())]),
        )
    }

    #[test]
    fn load_legacy_profiles_from_dir_loads_flat_and_nested_toml() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let nested = root.join("nested");
        fs::create_dir(&nested).expect("create nested dir");

        fs::write(
            root.join("desk.toml"),
            toml::to_string_pretty(&legacy_profile("desk", "DP-1")).expect("serialize desk"),
        )
        .expect("write flat profile");
        fs::write(
            nested.join("presentation.toml"),
            toml::to_string_pretty(&legacy_profile("presentation", "HDMI-A-1"))
                .expect("serialize presentation"),
        )
        .expect("write nested profile");
        fs::write(root.join("notes.txt"), "ignored").expect("write ignored file");

        let mut loaded = load_legacy_profiles_from_dir(root).expect("load legacy profiles");
        loaded.sort_by(|a, b| a.1.name.cmp(&b.1.name));

        assert_eq!(
            loaded
                .iter()
                .map(|(_, profile)| profile.name.as_str())
                .collect::<Vec<_>>(),
            ["desk", "presentation"]
        );
        assert!(loaded.iter().any(|(path, _)| path.ends_with("desk.toml")));
        assert!(loaded
            .iter()
            .any(|(path, _)| path.ends_with("nested/presentation.toml")));
    }
}
