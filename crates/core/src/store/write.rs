use super::ProfileStore;
use crate::error::{CoreError, CoreResult};
use crate::model::OutputIdentity;
use crate::planning::canonicalize_profile;
use crate::profile::Profile;
use crate::state::StateStore;
use std::collections::HashMap;

impl ProfileStore {
    /// Sets the default saved profile for a setup fingerprint.
    ///
    /// # Errors
    /// Returns an error if profile storage cannot be read or written.
    pub fn set_setup_default_profile(
        &self,
        setup_fingerprint: &str,
        profile_name: &str,
    ) -> CoreResult<()> {
        self.update_profiles_file(|stored| {
            stored
                .settings
                .set_setup_default_profile(setup_fingerprint, profile_name);
            Ok(())
        })
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

    fn save_with_known_outputs(
        &self,
        profile: &Profile,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<()> {
        let profile = canonicalize_profile(profile, known_outputs);
        let setup_fingerprint = profile.setup_fingerprint();
        self.update_profiles_file(|stored| {
            stored.profiles.retain(|existing| {
                !(existing.name == profile.name
                    && canonicalize_profile(existing, known_outputs).setup_fingerprint()
                        == setup_fingerprint)
            });
            stored.profiles.push(profile.clone());
            Ok(())
        })?;

        tracing::info!(
            "Saved profile '{name}' to {path:?}",
            name = profile.name,
            path = self.path
        );
        Ok(())
    }

    fn remove_for_setup_with_known_outputs(
        &self,
        name: &str,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<bool> {
        let removed = self.update_profiles_file_maybe(|stored| {
            let original_len = stored.profiles.len();
            stored.profiles.retain(|profile| {
                !(profile.name == name
                    && canonicalize_profile(profile, known_outputs).setup_fingerprint()
                        == setup_fingerprint)
            });

            if stored.profiles.len() == original_len {
                return Ok((false, false));
            }

            stored
                .settings
                .clear_setup_default_if_matches(setup_fingerprint, name);
            Ok((true, true))
        })?;
        if removed {
            tracing::info!("Removed profile '{name}'");
        }
        Ok(removed)
    }

    /// Removes a uniquely named profile.
    ///
    /// # Errors
    /// Returns an error if the name is ambiguous or storage cannot be read or written.
    pub fn remove_unique(&self, name: &str) -> CoreResult<bool> {
        let removed = self.update_profiles_file_maybe(|stored| {
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
                return Ok((false, false));
            }

            stored.settings.clear_all_profile_references(name);
            Ok((true, true))
        })?;
        if removed {
            tracing::info!("Removed profile '{name}'");
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ProfilesSettings;

    fn store_in(dir: &std::path::Path) -> ProfileStore {
        ProfileStore {
            path: dir.join("profiles.json"),
        }
    }

    #[test]
    fn remove_unique_clears_matching_default_references() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        let stored = serde_json::json!({
            "profiles": [
                {"name": "desk", "priority": 0, "match_rules": [], "layout": {}}
            ],
            "settings": {
                "setup_defaults": {"setup-a": "desk", "setup-b": "sofa"}
            }
        });
        std::fs::write(&store.path, format!("{stored}\n")).expect("write profiles");

        assert!(store.remove_unique("desk").expect("remove profile"));

        let settings: ProfilesSettings = store.load_profiles_file().expect("profiles").settings;
        assert_eq!(settings.setup_default_profile("setup-a"), None);
        assert_eq!(settings.setup_default_profile("setup-b"), Some("sofa"));
    }
}
