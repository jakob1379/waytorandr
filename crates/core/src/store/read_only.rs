use super::legacy::{load_profiles_file_from_path, load_profiles_from_path};
use super::query::{
    profile_values_for_setup_with_known_outputs, stored_profiles_for_setup_with_known_outputs,
    stored_profiles_to_profiles, stored_profiles_with_known_outputs,
};
use super::{ProfileQueryContext, ProfilesSettings, ReadOnlyProfileStore, StoredProfile};
use crate::error::CoreResult;
use crate::model::OutputIdentity;
use crate::profile::Profile;
use std::collections::HashMap;
impl ReadOnlyProfileStore {
    /// Returns stored settings without bootstrap or migration side effects.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors.
    pub fn settings(&self) -> CoreResult<ProfilesSettings> {
        Ok(load_profiles_file_from_path(&self.path)?.settings)
    }

    /// Lists profiles with normalized state without bootstrap or migration side effects.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors, or read-only state-store read
    /// errors.
    pub fn list(&self, context: &ProfileQueryContext) -> CoreResult<Vec<StoredProfile>> {
        self.list_with_known_outputs(context.known_outputs())
    }

    /// Lists profiles for a setup with normalized state without bootstrap or migration side effects.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors, or read-only state-store read
    /// errors.
    pub fn list_for_setup(
        &self,
        setup_fingerprint: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Vec<StoredProfile>> {
        self.list_for_setup_with_known_outputs(setup_fingerprint, context.known_outputs())
    }

    /// Returns all profiles with normalized state without bootstrap or migration side effects.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors, or read-only state-store read
    /// errors.
    pub fn profiles(&self, context: &ProfileQueryContext) -> CoreResult<Vec<Profile>> {
        self.profiles_with_known_outputs(context.known_outputs())
    }

    /// Returns profiles for a setup with normalized state without bootstrap or migration side effects.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors, or read-only state-store read
    /// errors.
    pub fn profiles_for_setup(
        &self,
        setup_fingerprint: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Vec<Profile>> {
        self.profiles_for_setup_with_known_outputs(setup_fingerprint, context.known_outputs())
    }

    /// Lists stored profile names.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors.
    pub fn list_names(&self) -> CoreResult<Vec<String>> {
        let mut names = std::collections::BTreeSet::new();
        for profile in load_profiles_from_path(&self.path)? {
            names.insert(profile.name);
        }

        Ok(names.into_iter().collect())
    }

    /// Lists stored profile names for a normalized setup fingerprint.
    ///
    /// # Errors
    /// Returns profile-file read or parse errors, or read-only state-store read
    /// errors.
    pub fn list_names_for_setup(
        &self,
        setup_fingerprint: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Vec<String>> {
        let mut names = std::collections::BTreeSet::new();
        for stored in self.list_for_setup(setup_fingerprint, context)? {
            names.insert(stored.profile.name);
        }

        Ok(names.into_iter().collect())
    }

    fn list_with_known_outputs(
        &self,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<StoredProfile>> {
        Ok(stored_profiles_with_known_outputs(
            load_profiles_from_path(&self.path)?,
            known_outputs,
        ))
    }

    fn list_for_setup_with_known_outputs(
        &self,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<StoredProfile>> {
        Ok(stored_profiles_for_setup_with_known_outputs(
            load_profiles_from_path(&self.path)?,
            setup_fingerprint,
            known_outputs,
        ))
    }

    fn profiles_with_known_outputs(
        &self,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<Profile>> {
        Ok(stored_profiles_to_profiles(
            self.list_with_known_outputs(known_outputs)?,
        ))
    }

    fn profiles_for_setup_with_known_outputs(
        &self,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<Profile>> {
        Ok(profile_values_for_setup_with_known_outputs(
            load_profiles_from_path(&self.path)?,
            setup_fingerprint,
            known_outputs,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_names_returns_sorted_unique_profile_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("profiles.json");
        std::fs::write(
            &path,
            r#"{"profiles":[{"name":"desk","priority":0,"match_rules":[],"layout":{}},{"name":"desk","priority":1,"match_rules":[],"layout":{}},{"name":"sofa","priority":0,"match_rules":[],"layout":{}}],"settings":{}}"#,
        )
        .expect("write profiles");
        let store = ReadOnlyProfileStore { path };

        assert_eq!(
            store.list_names().expect("list names"),
            vec!["desk".to_string(), "sofa".to_string()]
        );
    }
}
