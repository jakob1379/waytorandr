use clap_complete::engine::CompletionCandidate;
use std::collections::BTreeSet;

use crate::preset::{is_builtin_set_target, virtual_completion_candidates};
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::workflow;
use waytorandr_core::{ProfileQueryContext, ProfileStore, ReadOnlyStateStore};

pub(crate) fn complete_set_targets(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let mut candidates: Vec<_> = virtual_completion_candidates(current)
        .into_iter()
        .chain(saved_profile_set_target_completion_candidates(current))
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
    let mut seen = BTreeSet::new();
    load_saved_profile_names()
        .into_iter()
        .filter(|name| name.starts_with(current))
        .filter(|name| seen.insert(name.clone()))
        .map(|name| CompletionCandidate::new(name).tag(Some("profile".into())))
        .collect()
}

fn saved_profile_set_target_completion_candidates(current: &str) -> Vec<CompletionCandidate> {
    let mut seen = BTreeSet::new();
    load_saved_profile_names()
        .into_iter()
        .filter(|name| !is_builtin_set_target(name))
        .filter(|name| name.starts_with(current))
        .filter(|name| seen.insert(name.clone()))
        .map(|name| CompletionCandidate::new(name).tag(Some("profile".into())))
        .collect()
}

fn load_saved_profile_names() -> Vec<String> {
    let Ok(setup_fingerprint) = current_setup_fingerprint() else {
        return Vec::new();
    };

    ProfileStore::open_read_only()
        .and_then(|store| {
            let state_store = ReadOnlyStateStore::open()?;
            let query_context = ProfileQueryContext::load_read_only(&state_store)?;
            store.list_for_setup(&setup_fingerprint, &query_context)
        })
        .unwrap_or_default()
        .into_iter()
        .map(|stored| stored.profile.name)
        .collect()
}

fn current_setup_fingerprint() -> anyhow::Result<String> {
    #[cfg(test)]
    if let Some(fingerprint) = test_backend_setup_fingerprint()? {
        return Ok(fingerprint);
    }

    let backend = connect_backend().map_err(anyhow::Error::from)?;
    let state_store = ReadOnlyStateStore::open()?;
    Ok(
        workflow::normalized_topology_from_backend(backend.as_ref(), &state_store)?
            .setup_fingerprint(),
    )
}

#[cfg(test)]
fn test_backend_setup_fingerprint() -> anyhow::Result<Option<String>> {
    #[derive(serde::Deserialize)]
    struct TestBackendState {
        topology: waytorandr_core::Topology,
    }

    let Some(path) = std::env::var_os("WAYTORANDR_TEST_BACKEND_STATE") else {
        return Ok(None);
    };

    let content = std::fs::read_to_string(path)?;
    let state: TestBackendState = serde_json::from_str(&content)?;
    Ok(Some(state.topology.setup_fingerprint()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use waytorandr_core::ProfileStore;
    use waytorandr_core::StateStore;
    use waytorandr_core::{Mode, OutputIdentity, OutputState, Position, Topology};
    use waytorandr_core::{OutputMatcher, Profile};

    const TEST_BACKEND_STATE_ENV: &str = "WAYTORANDR_TEST_BACKEND_STATE";
    const TEST_BACKEND_NAME_ENV: &str = "WAYTORANDR_TEST_BACKEND_NAME";

    #[test]
    fn set_target_completion_includes_virtual_matches() {
        assert!(!complete_set_targets(OsStr::new("ver")).is_empty());
    }

    #[test]
    fn set_target_completion_includes_auto() {
        let names: Vec<_> = complete_set_targets(OsStr::new("au"))
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, vec!["auto"]);
    }

    #[test]
    fn set_target_completion_hides_colliding_saved_names() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = CompletionTestEnv::new();

        env.write_backend_topology(&completion_topology(["eDP-1"]))
            .expect("write topology");
        env.save_profile("auto", &["eDP-1"])
            .expect("save current profile");

        let names: Vec<_> = complete_set_targets(OsStr::new("au"))
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, vec!["auto"]);
    }

    #[test]
    fn saved_profile_completion_ignores_non_utf_input() {
        let invalid = OsStr::from_bytes(&[0xff]);

        assert!(complete_saved_profiles(invalid).is_empty());
    }

    #[test]
    fn saved_profile_completion_supports_plain_names() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = CompletionTestEnv::new();

        env.write_backend_topology(&completion_topology(["eDP-1"]))
            .expect("write topology");
        env.save_profile("ro-auto", &["eDP-1"])
            .expect("save current profile");

        let names: Vec<_> = complete_saved_profiles(OsStr::new("ro-"))
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, vec!["ro-auto"]);
    }

    #[test]
    fn set_completion_keeps_virtual_presets_when_setup_lookup_fails() {
        let names: Vec<_> = complete_set_targets(OsStr::new("ver"))
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect();

        assert!(names.iter().any(|name| name == "vertical"));
    }

    #[test]
    fn saved_profile_completion_does_not_create_store_files_when_nothing_is_saved() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = CompletionTestEnv::new();

        env.write_backend_topology(&completion_topology(["eDP-1"]))
            .expect("write topology");

        assert!(complete_saved_profiles(OsStr::new("")).is_empty());
        assert!(!env.config_store_dir().exists());
        assert!(!env.state_store_dir().exists());
        assert!(!env.profiles_file().exists());
        assert!(!env.state_file().exists());
    }

    struct CompletionTestEnv {
        _root: tempfile::TempDir,
        backend_state_path: PathBuf,
    }

    impl CompletionTestEnv {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("tempdir");
            let config_home = root.path().join("config");
            let state_home = root.path().join("state");
            std::fs::create_dir_all(&config_home).expect("config dir");
            std::fs::create_dir_all(&state_home).expect("state dir");

            std::env::set_var("XDG_CONFIG_HOME", &config_home);
            std::env::set_var("XDG_STATE_HOME", &state_home);
            std::env::set_var(TEST_BACKEND_STATE_ENV, state_home.join("test-backend.json"));
            std::env::set_var(TEST_BACKEND_NAME_ENV, "test");

            Self {
                backend_state_path: state_home.join("test-backend.json"),
                _root: root,
            }
        }

        fn write_backend_topology(&self, topology: &Topology) -> anyhow::Result<()> {
            let content = serde_json::json!({ "topology": topology });
            std::fs::write(&self.backend_state_path, format!("{content}\n"))?;
            Ok(())
        }

        fn save_profile(&self, name: &str, connectors: &[&str]) -> anyhow::Result<()> {
            let store = ProfileStore::bootstrap()?;
            let state_store = StateStore::bootstrap()?;
            store.save(&completion_profile(name, connectors), &state_store)?;
            Ok(())
        }

        fn config_store_dir(&self) -> PathBuf {
            self._root.path().join("config").join("waytorandr")
        }

        fn state_store_dir(&self) -> PathBuf {
            self._root.path().join("state").join("waytorandr")
        }

        fn profiles_file(&self) -> PathBuf {
            self.config_store_dir().join("waytorandr.json")
        }

        fn state_file(&self) -> PathBuf {
            self.state_store_dir().join("state.toml")
        }
    }

    impl Drop for CompletionTestEnv {
        fn drop(&mut self) {
            remove_env_var("XDG_CONFIG_HOME");
            remove_env_var("XDG_STATE_HOME");
            remove_env_var(TEST_BACKEND_STATE_ENV);
            remove_env_var(TEST_BACKEND_NAME_ENV);
        }
    }

    fn remove_env_var(name: &str) {
        if std::env::var_os(name).is_some() {
            std::env::remove_var(name);
        }
    }

    fn completion_topology(connectors: [&str; 1]) -> Topology {
        let mut topology = Topology::new();
        for connector in connectors {
            let mut output = OutputState::new(connector);
            output.identity = OutputIdentity::new(connector);
            output.enabled = true;
            output.mode = Some(Mode::new(1920, 1080, 60));
            output.position = Position::new(0, 0);
            topology.outputs.insert(connector.to_string(), output);
        }
        topology
    }

    fn completion_profile(name: &str, connectors: &[&str]) -> Profile {
        let match_rules = connectors
            .iter()
            .map(|connector| OutputMatcher::new(OutputIdentity::new(*connector), true, None))
            .collect();

        let layout = connectors
            .iter()
            .map(|connector| {
                let mut state = OutputState::new(*connector);
                state.identity = OutputIdentity::new(*connector);
                state.enabled = true;
                state.mode = Some(Mode::new(1920, 1080, 60));
                state.position = Position::new(0, 0);
                ((*connector).to_string(), state.into())
            })
            .collect();

        Profile::new(name, 0, match_rules, layout)
    }
}
