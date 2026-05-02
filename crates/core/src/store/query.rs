use super::{ProfileQueryContext, ProfileStore, ProfilesSettings, StoredProfile};
use crate::error::{CoreError, CoreResult};
use crate::model::OutputIdentity;
use crate::planning::canonicalize_profile;
use crate::profile::Profile;
use std::collections::HashMap;

impl ProfileStore {
    /// Lists profiles after applying known-output normalization.
    ///
    /// # Errors
    /// Returns profile-file read, parse, migration, or state-store read errors.
    pub fn list(&self, context: &ProfileQueryContext) -> CoreResult<Vec<StoredProfile>> {
        self.list_with_known_outputs(context.known_outputs())
    }

    /// Lists profiles for one setup after applying known-output normalization.
    ///
    /// # Errors
    /// Returns profile-file read, parse, migration, or state-store read errors.
    pub fn list_for_setup(
        &self,
        setup_fingerprint: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Vec<StoredProfile>> {
        self.list_for_setup_with_known_outputs(setup_fingerprint, context.known_outputs())
    }

    /// Returns profile values after applying known-output normalization.
    ///
    /// # Errors
    /// Returns profile-file read, parse, migration, or state-store read errors.
    pub fn profiles(&self, context: &ProfileQueryContext) -> CoreResult<Vec<Profile>> {
        self.profiles_with_known_outputs(context.known_outputs())
    }

    /// Returns profile values for one setup after known-output normalization.
    ///
    /// # Errors
    /// Returns profile-file read, parse, migration, or state-store read errors.
    pub fn profiles_for_setup(
        &self,
        setup_fingerprint: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Vec<Profile>> {
        self.profiles_for_setup_with_known_outputs(setup_fingerprint, context.known_outputs())
    }

    /// Finds a profile for a setup after known-output normalization.
    ///
    /// # Errors
    /// Returns profile-file read, parse, migration, or state-store read errors.
    pub fn get_for_setup(
        &self,
        name: &str,
        setup_fingerprint: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Option<StoredProfile>> {
        self.get_for_setup_with_known_outputs(name, setup_fingerprint, context.known_outputs())
    }

    /// Returns stored settings.
    ///
    /// # Errors
    /// Returns profile-file read, parse, or migration errors.
    pub fn settings(&self) -> CoreResult<ProfilesSettings> {
        Ok(self.load_profiles_file()?.settings)
    }

    pub(super) fn list_with_known_outputs(
        &self,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<StoredProfile>> {
        Ok(stored_profiles_with_known_outputs(
            self.load_profiles()?,
            known_outputs,
        ))
    }

    pub(super) fn list_for_setup_with_known_outputs(
        &self,
        setup_fingerprint: &str,
        known_outputs: &HashMap<String, OutputIdentity>,
    ) -> CoreResult<Vec<StoredProfile>> {
        Ok(stored_profiles_for_setup_with_known_outputs(
            self.load_profiles()?,
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
            self.load_profiles()?,
            setup_fingerprint,
            known_outputs,
        ))
    }

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

    /// Finds a uniquely named profile, rejecting ambiguous names across setups.
    ///
    /// # Errors
    /// Returns profile-file read, parse, migration, or state-store read errors.
    /// Also returns `CoreError::AmbiguousProfile` when multiple stored profiles
    /// share `name` across setups.
    pub fn get_unique(
        &self,
        name: &str,
        context: &ProfileQueryContext,
    ) -> CoreResult<Option<StoredProfile>> {
        self.get_unique_with_known_outputs(name, context.known_outputs())
    }

    pub(super) fn get_unique_with_known_outputs(
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
}

fn filter_profiles_for_setup(
    stored_profiles: Vec<StoredProfile>,
    setup_fingerprint: &str,
) -> Vec<StoredProfile> {
    stored_profiles
        .into_iter()
        .filter(|stored| stored.setup_fingerprint == setup_fingerprint)
        .collect()
}

pub(super) fn stored_profiles_for_setup_with_known_outputs(
    profiles: Vec<Profile>,
    setup_fingerprint: &str,
    known_outputs: &HashMap<String, OutputIdentity>,
) -> Vec<StoredProfile> {
    filter_profiles_for_setup(
        stored_profiles_with_known_outputs(profiles, known_outputs),
        setup_fingerprint,
    )
}

pub(super) fn profile_values_for_setup_with_known_outputs(
    profiles: Vec<Profile>,
    setup_fingerprint: &str,
    known_outputs: &HashMap<String, OutputIdentity>,
) -> Vec<Profile> {
    stored_profiles_to_profiles(stored_profiles_for_setup_with_known_outputs(
        profiles,
        setup_fingerprint,
        known_outputs,
    ))
}

pub(super) fn stored_profiles_with_known_outputs(
    profiles: Vec<Profile>,
    known_outputs: &HashMap<String, OutputIdentity>,
) -> Vec<StoredProfile> {
    let mut profiles: Vec<_> = profiles
        .into_iter()
        .map(|profile| {
            let profile = canonicalize_profile(&profile, known_outputs);
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
    profiles
}

pub(super) fn stored_profiles_to_profiles(stored_profiles: Vec<StoredProfile>) -> Vec<Profile> {
    stored_profiles
        .into_iter()
        .map(|stored| stored.profile)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OutputState;
    use crate::profile::OutputMatcher;

    fn query_profile(name: &str, connector: &str) -> Profile {
        Profile::new(
            name,
            0,
            vec![OutputMatcher::new(
                OutputIdentity::new(connector),
                true,
                None,
            )],
            HashMap::from([(connector.to_string(), OutputState::new(connector).into())]),
        )
    }

    #[test]
    fn filters_profiles_for_matching_setup_fingerprint() {
        let matching = query_profile("desk", "DP-1");
        let other = query_profile("sofa", "HDMI-A-1");
        let setup_fingerprint = matching.setup_fingerprint();

        let stored = stored_profiles_for_setup_with_known_outputs(
            vec![matching, other],
            &setup_fingerprint,
            &HashMap::new(),
        );

        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].profile.name, "desk");
        assert_eq!(stored[0].setup_fingerprint, setup_fingerprint);
    }
}
