use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, OnceLock};

use tempfile::TempDir;
use waytorandr_core::engine::{ApplyResult, Backend, OutputWatcher, TestResult};
use waytorandr_core::error::CoreError;
use waytorandr_core::model::{
    BackendKind, Capabilities, OutputIdentity, OutputState, Position, Topology,
};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::{Hook, Hooks, OutputMatcher, Profile};
use waytorandr_core::state::{State, StateStore};
use waytorandr_core::store::ProfileStore;
use waytorandr_core::workflow::{self, PlanSnapshot};

fn xdg_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn scoped_env_var(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> ScopedEnvVar {
    let previous = std::env::var_os(key);
    std::env::set_var(key, value);
    ScopedEnvVar { key, previous }
}

fn with_test_dirs<T>(
    f: impl FnOnce(&TempDir) -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    let _guard = xdg_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir()?;
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    std::fs::create_dir_all(&config_home)?;
    std::fs::create_dir_all(&state_home)?;

    let _config_home = scoped_env_var("XDG_CONFIG_HOME", &config_home);
    let _state_home = scoped_env_var("XDG_STATE_HOME", &state_home);

    f(&temp)
}

fn output(connector: &str) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = true;
    state.position = Position::new(0, 0);
    state
}

fn profile(name: &str, connector: &str) -> Profile {
    Profile::new(
        name,
        0,
        vec![OutputMatcher::new(
            OutputIdentity::new(connector),
            true,
            Some(Position::new(0, 0)),
        )],
        HashMap::from([(connector.to_string(), output(connector).into())]),
    )
}

fn profiles_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path()
        .join("config")
        .join("waytorandr")
        .join("profiles.json")
}

#[test]
fn profile_store_roundtrips_saved_profiles_per_setup() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|temp| {
        let store = ProfileStore::bootstrap()?;
        let profile = profile("desk", "DP-1");
        let setup_fingerprint = profile.setup_fingerprint();
        let state = State::default();

        store.save_with_known_outputs(&profile, &state.known_outputs)?;

        assert!(profiles_path(temp).exists());
        assert!(!temp
            .path()
            .join("config")
            .join("waytorandr")
            .join("profiles")
            .exists());

        let loaded = store
            .get_for_setup_with_known_outputs("desk", &setup_fingerprint, &state.known_outputs)?
            .ok_or_else(|| std::io::Error::other("desk profile should exist"))?;
        assert_eq!(loaded.profile.name, "desk");
        assert_eq!(loaded.setup_fingerprint, setup_fingerprint);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_returns_canonical_match_ready_profiles() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let store = ProfileStore::bootstrap()?;
        let mut state = State::default();
        state.known_outputs.insert("DP-1".to_string(), {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Dell".to_string());
            identity.model = Some("U2720Q".to_string());
            identity
        });
        let profile = profile("desk", "DP-1");

        store.save_with_known_outputs(&profile, &state.known_outputs)?;

        let setup_fingerprint = store
            .list_with_known_outputs(&state.known_outputs)?
            .into_iter()
            .find(|stored| stored.profile.name == "desk")
            .ok_or_else(|| std::io::Error::other("desk profile should be listed"))?
            .setup_fingerprint;
        let loaded = store
            .get_for_setup_with_known_outputs("desk", &setup_fingerprint, &state.known_outputs)?
            .ok_or_else(|| std::io::Error::other("desk profile should exist"))?;
        assert_eq!(loaded.profile.match_rules.len(), 1);
        assert_eq!(
            loaded.profile.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_migrates_legacy_profiles_to_json_file() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|temp| {
        let legacy_profile = profile("desk", "DP-1");
        let profiles_dir = temp
            .path()
            .join("config")
            .join("waytorandr")
            .join("profiles");
        std::fs::create_dir_all(&profiles_dir)?;
        let legacy_path = profiles_dir.join("desk.toml");
        std::fs::write(&legacy_path, toml::to_string_pretty(&legacy_profile)?)?;

        let store = ProfileStore::bootstrap()?;
        let setup_fingerprint = legacy_profile.setup_fingerprint();
        let profiles_path = profiles_path(temp);
        let state = State::default();

        assert!(!legacy_path.exists());
        assert!(profiles_path.exists());
        assert!(store
            .get_for_setup_with_known_outputs("desk", &setup_fingerprint, &state.known_outputs)?
            .is_some());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn state_store_normalizes_profile_using_cached_outputs() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let state_store = StateStore::bootstrap()?;
        let mut state = State::default();
        state.known_outputs.insert("DP-1".to_string(), {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Dell".to_string());
            identity.model = Some("U2720Q".to_string());
            identity
        });
        state_store.save_state(&state)?;

        let loaded = state_store
            .load_state()?
            .ok_or_else(|| std::io::Error::other("state should exist"))?;
        let normalized = waytorandr_core::normalize::normalize_profile_with_known_outputs(
            &profile("desk", "DP-1"),
            &loaded.known_outputs,
        );
        let identity = &normalized.layout["DP-1"].state.identity;

        assert_eq!(identity.make.as_deref(), Some("Dell"));
        assert_eq!(identity.model.as_deref(), Some("U2720Q"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn state_store_drops_legacy_default_profile_on_write() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let state_store = StateStore::bootstrap()?;
        let legacy_state = [
            "default_profile = \"desk\"",
            "daemon_enabled = false",
            "[default_profiles]",
            "[known_outputs]",
        ]
        .join("\n");
        std::fs::write(state_store.dir().join("state.toml"), legacy_state)?;

        let loaded = state_store
            .load_state()?
            .ok_or_else(|| std::io::Error::other("state should exist"))?;

        let persisted = std::fs::read_to_string(state_store.dir().join("state.toml"))?;
        assert_eq!(
            loaded
                .default_profiles
                .get(State::GLOBAL_DEFAULT_PROFILE_KEY)
                .map(String::as_str),
            Some("desk")
        );
        assert!(persisted.contains("default_profile = \"desk\""));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn state_store_persists_known_outputs_only_on_explicit_observation() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let state_store = StateStore::bootstrap()?;
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };

        let before = std::fs::read_to_string(state_store.dir().join("state.toml"));
        assert!(before.is_err());

        let normalized = state_store.observe_topology_and_persist_known_outputs(&topology)?;

        assert_eq!(normalized.fingerprint(), topology.fingerprint());
        let persisted = std::fs::read_to_string(state_store.dir().join("state.toml"))?;
        assert!(persisted.contains("known_outputs"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_selects_applies_and_records_matching_profile() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let backend = TestBackend {
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1"), profile("fallback", "HDMI-A-1")];
        let mut state = State::default();
        state.default_profiles.insert(
            State::GLOBAL_DEFAULT_PROFILE_KEY.to_string(),
            "fallback".to_string(),
        );

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &state)
            .ok_or_else(|| std::io::Error::other("matching profile should be selected"))?;
        let plan = workflow::plan_profile_for_topology(&selected, &topology)?;
        let cycle = workflow::apply_plan_cycle(
            &backend,
            &selected.hooks,
            PlanSnapshot {
                topology: topology.clone(),
                plan: plan.clone(),
            },
            PlanSnapshot {
                topology: topology.clone(),
                plan,
            },
        )?;
        assert!(
            matches!(cycle, workflow::ExecutionCycle::Applied { apply_result, .. } if apply_result.success)
        );
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );

        state.record_applied_profile(&selected.name, Some(BackendKind::Test), &topology);
        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_prefers_setup_default_over_matching_profile() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("both", "DP-1"), profile("external-only", "DP-1")];
        let mut state = State::default();
        state
            .default_profiles
            .insert(topology.setup_fingerprint(), "external-only".to_string());

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &state)
            .ok_or_else(|| std::io::Error::other("setup default should be selected"))?;

        assert_eq!(selected.name, "external-only");
        Ok(())
    })?;
    Ok(())
}

#[derive(Clone)]
struct TestBackend {
    apply_calls: Arc<Mutex<usize>>,
}

impl Backend for TestBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Test);
        capabilities.can_test = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> waytorandr_core::error::CoreResult<Topology> {
        Ok(Topology::default())
    }

    fn watch_outputs(&self) -> waytorandr_core::error::CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn test(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<TestResult> {
        Ok(TestResult::supported(None))
    }

    fn apply(&self, plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<ApplyResult> {
        *self
            .apply_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        let mut result = ApplyResult::default();
        result.success = true;
        result.applied_state = Some(Topology {
            outputs: plan.outputs.clone(),
        });
        Ok(result)
    }
}

#[test]
fn runtime_cycle_applies_plan_once_through_public_api() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|temp| {
        let log_path = temp.path().join("hooks.log");
        let backend = TestBackend {
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let mut pre_hook = Hook::new("sh");
        pre_hook.args = vec![
            "-c".to_string(),
            format!("printf '%s\\n' pre >> {}", log_path.display()),
        ];
        pre_hook.timeout_secs = 5;
        let mut post_hook = Hook::new("sh");
        post_hook.args = vec![
            "-c".to_string(),
            format!("printf '%s\\n' post >> {}", log_path.display()),
        ];
        post_hook.timeout_secs = 5;
        let mut hooks = Hooks::default();
        hooks.pre_apply = vec![pre_hook];
        hooks.post_apply = vec![post_hook];
        let plan = LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))]));

        let cycle = workflow::apply_plan_cycle(
            &backend,
            &hooks,
            PlanSnapshot {
                topology: Topology::default(),
                plan: plan.clone(),
            },
            PlanSnapshot {
                topology: Topology::default(),
                plan,
            },
        )?;
        assert!(
            matches!(cycle, workflow::ExecutionCycle::Applied { apply_result, .. } if apply_result.success)
        );
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );

        let log = std::fs::read_to_string(log_path)?;
        assert!(log.contains("pre"));
        assert!(log.contains("post"));
        Ok(())
    })?;
    Ok(())
}
