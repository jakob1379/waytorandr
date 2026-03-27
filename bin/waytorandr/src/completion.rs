use clap_complete::engine::CompletionCandidate;

use crate::preset::virtual_completion_candidates;
use waytorandr_core::store::ProfileStore;

pub(crate) fn complete_set_targets(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let mut candidates: Vec<_> = virtual_completion_candidates(current)
        .into_iter()
        .chain(saved_profile_completion_candidates(current))
        .collect();
    candidates.sort();
    candidates
}

pub(crate) fn complete_saved_profiles(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let mut candidates = saved_profile_completion_candidates(current);
    candidates.sort();
    candidates
}

fn saved_profile_completion_candidates(current: &str) -> Vec<CompletionCandidate> {
    let mut seen = std::collections::BTreeSet::new();
    ProfileStore::open_read_only()
        .and_then(|store| store.list_names())
        .unwrap_or_default()
        .into_iter()
        .filter(|name| name.starts_with(current))
        .filter(|name| seen.insert(name.clone()))
        .map(|name| CompletionCandidate::new(name).tag(Some("profile".into())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn set_target_completion_includes_virtual_matches() {
        assert!(!complete_set_targets(OsStr::new("ver")).is_empty());
    }

    #[test]
    fn saved_profile_completion_ignores_non_utf_input() {
        let invalid = OsStr::from_bytes(&[0xff]);

        assert!(complete_saved_profiles(invalid).is_empty());
    }
}
