use super::legacy::{
    legacy_profile_dir, legacy_profiles_json_path, load_legacy_profiles_from_dir,
    merge_legacy_profile, remove_empty_legacy_directories,
};
use super::ProfileStore;
use crate::error::{CoreError, CoreResult};
use crate::state::StateStore;
use std::fs;

pub(super) fn bootstrap_profile_store(store: ProfileStore) -> CoreResult<ProfileStore> {
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

    fn migrate_legacy_profiles(&self) -> CoreResult<()> {
        let legacy_dir = legacy_profile_dir()?;
        if !legacy_dir.exists() {
            return Ok(());
        }

        let migrated_paths = self.update_profiles_file(|stored| {
            let mut migrated_paths = Vec::new();

            for (legacy_path, profile) in load_legacy_profiles_from_dir(&legacy_dir)? {
                merge_legacy_profile(&mut stored.profiles, profile, &legacy_path, &self.path)?;
                migrated_paths.push(legacy_path);
            }

            Ok(migrated_paths)
        })?;

        if migrated_paths.is_empty() {
            return Ok(());
        }

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
        let Some(state) = state_store.load_state()? else {
            return Ok(());
        };

        if state.default_profiles.is_empty() {
            return Ok(());
        }

        self.update_profiles_file(|stored| {
            for (setup_fingerprint, profile_name) in &state.default_profiles {
                if setup_fingerprint.starts_with("__") {
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
                }
            }
            Ok(())
        })?;

        state_store.update_state(|state| {
            state.default_profiles.clear();
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_profile_store_creates_parent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProfileStore {
            path: dir.path().join("nested").join("profiles.json"),
        };

        let bootstrapped = bootstrap_profile_store(store).expect("bootstrap store");

        assert!(bootstrapped.path.parent().expect("parent").is_dir());
    }
}
