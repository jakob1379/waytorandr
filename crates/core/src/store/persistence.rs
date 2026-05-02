use super::legacy::load_profiles_file_from_path;
use super::{ProfileStore, ProfilesFile};
use crate::error::{CoreError, CoreResult};
use crate::persistence::{atomic_write, with_exclusive_lock};
use crate::profile::Profile;

impl ProfileStore {
    pub(super) fn load_profiles(&self) -> CoreResult<Vec<Profile>> {
        Ok(self.load_profiles_file()?.profiles)
    }

    pub(super) fn load_profiles_file(&self) -> CoreResult<ProfilesFile> {
        load_profiles_file_from_path(&self.path)
    }

    fn save_profiles_file_unlocked(&self, profiles: &ProfilesFile) -> CoreResult<()> {
        let content =
            serde_json::to_string_pretty(profiles).map_err(|source| CoreError::SerializeJson {
                path: self.path.clone(),
                source,
            })?;
        atomic_write(&self.path, format!("{content}\n").as_bytes())
    }

    pub(super) fn update_profiles_file<T>(
        &self,
        update: impl FnOnce(&mut ProfilesFile) -> CoreResult<T>,
    ) -> CoreResult<T> {
        self.update_profiles_file_maybe(|stored| update(stored).map(|result| (result, true)))
    }

    pub(super) fn update_profiles_file_maybe<T>(
        &self,
        update: impl FnOnce(&mut ProfilesFile) -> CoreResult<(T, bool)>,
    ) -> CoreResult<T> {
        with_exclusive_lock(&self.path, || {
            let mut stored = self.load_profiles_file()?;
            let (result, changed) = update(&mut stored)?;
            if changed {
                self.save_profiles_file_unlocked(&stored)?;
            }
            Ok(result)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &std::path::Path) -> ProfileStore {
        ProfileStore {
            path: dir.join("profiles.json"),
        }
    }

    fn write_profiles_file(path: &std::path::Path, content: &str) -> std::io::Result<()> {
        std::fs::write(path, content)
    }

    #[test]
    fn update_profiles_file_maybe_skips_write_when_unchanged() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = store_in(dir.path());
        let original = "{\"profiles\":[],\"settings\":{}}\n";
        write_profiles_file(&store.path, original)?;

        let profile_count =
            store.update_profiles_file_maybe(|stored| Ok((stored.profiles.len(), false)))?;

        assert_eq!(profile_count, 0);
        assert_eq!(std::fs::read_to_string(&store.path)?, original);
        Ok(())
    }

    #[test]
    fn update_profiles_file_persists_changed_profile_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = store_in(dir.path());
        write_profiles_file(&store.path, "{\"profiles\":[],\"settings\":{}}\n")?;

        store.update_profiles_file(|stored| {
            stored
                .profiles
                .push(Profile::new("desk", 0, Vec::new(), Default::default()));
            Ok(())
        })?;

        let stored = store.load_profiles_file()?;
        assert_eq!(stored.profiles.len(), 1);
        assert_eq!(stored.profiles[0].name, "desk");
        assert!(std::fs::read_to_string(&store.path)?.ends_with('\n'));
        Ok(())
    }
}
