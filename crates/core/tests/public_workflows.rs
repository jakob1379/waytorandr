use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, OnceLock};

use tempfile::TempDir;
use waytorandr_core::engine::{ApplyResult, Backend, OutputWatcher, TestResult};
use waytorandr_core::error::CoreError;
use waytorandr_core::model::{Capabilities, OutputIdentity, OutputState, Position, Topology};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::{Hook, Hooks, OutputConfig, OutputMatcher, Profile, ProfileOptions};
use waytorandr_core::runtime;
use waytorandr_core::store::{ProfileStore, State, StateStore};

fn xdg_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn with_test_dirs<T>(f: impl FnOnce(&TempDir) -> T) -> T {
    let _guard = xdg_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    std::fs::create_dir_all(&config_home).unwrap();
    std::fs::create_dir_all(&state_home).unwrap();

    let previous_config = std::env::var_os("XDG_CONFIG_HOME");
    let previous_state = std::env::var_os("XDG_STATE_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &config_home);
    std::env::set_var("XDG_STATE_HOME", &state_home);

    let result = f(&temp);

    restore_env("XDG_CONFIG_HOME", previous_config);
    restore_env("XDG_STATE_HOME", previous_state);

    result
}

fn restore_env(key: &str, value: Option<OsString>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

fn output(connector: &str) -> OutputState {
    let mut state = OutputState::new(connector);
    state.enabled = true;
    state.position = Position::new(0, 0);
    state
}

fn profile(name: &str, connector: &str) -> Profile {
    Profile {
        name: name.to_string(),
        priority: 0,
        match_rules: vec![OutputMatcher {
            identity: OutputIdentity::new(connector),
            required: true,
            position_hint: Some(Position::new(0, 0)),
        }],
        layout: HashMap::from([(
            connector.to_string(),
            OutputConfig {
                state: output(connector),
                preset: None,
            },
        )]),
        hooks: Hooks::default(),
        options: ProfileOptions::default(),
    }
}

fn profiles_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path()
        .join("config")
        .join("waytorandr")
        .join("profiles.json")
}

#[test]
fn profile_store_roundtrips_saved_profiles_per_setup() {
    with_test_dirs(|temp| {
        let store = ProfileStore::new().unwrap();
        let profile = profile("desk", "DP-1");
        let setup_fingerprint = profile.setup_fingerprint();

        store.save(&profile, "setup-1").unwrap();

        assert!(profiles_path(temp).exists());
        assert!(!temp
            .path()
            .join("config")
            .join("waytorandr")
            .join("profiles")
            .exists());

        let loaded = store
            .get_in_setup("desk", &setup_fingerprint)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.profile.name, "desk");
        assert_eq!(loaded.setup_fingerprint, setup_fingerprint);
    });
}

#[test]
fn profile_store_returns_canonical_match_ready_profiles() {
    with_test_dirs(|_| {
        let store = ProfileStore::new().unwrap();
        let profile = Profile {
            name: "desk".to_string(),
            priority: 0,
            match_rules: Vec::new(),
            layout: HashMap::from([(
                "DP-1".to_string(),
                OutputConfig {
                    state: output("DP-1"),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
            options: ProfileOptions::default(),
        };

        store.save(&profile, "setup-1").unwrap();

        let loaded = store.get_in_setup("desk", "conn:DP-1").unwrap().unwrap();
        assert_eq!(loaded.profile.match_rules.len(), 1);
        assert_eq!(
            loaded.profile.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
    });
}

#[test]
fn profile_store_migrates_legacy_profiles_to_json_file() {
    with_test_dirs(|temp| {
        let legacy_profile = profile("desk", "DP-1");
        let profiles_dir = temp
            .path()
            .join("config")
            .join("waytorandr")
            .join("profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let legacy_path = profiles_dir.join("desk.toml");
        std::fs::write(
            &legacy_path,
            toml::to_string_pretty(&legacy_profile).unwrap(),
        )
        .unwrap();

        let store = ProfileStore::new().unwrap();
        let setup_fingerprint = legacy_profile.setup_fingerprint();
        let profiles_path = profiles_path(temp);

        assert!(!legacy_path.exists());
        assert!(profiles_path.exists());
        assert!(store
            .get_in_setup("desk", &setup_fingerprint)
            .unwrap()
            .is_some());
    });
}

#[test]
fn state_store_normalizes_profile_using_cached_outputs() {
    with_test_dirs(|_| {
        let state_store = StateStore::new().unwrap();
        let mut state = State::default();
        state.known_outputs.insert("DP-1".to_string(), {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Dell".to_string());
            identity.model = Some("U2720Q".to_string());
            identity
        });
        state_store.save_state(&state).unwrap();

        let normalized = state_store
            .normalize_profile(&profile("desk", "DP-1"))
            .unwrap();
        let identity = &normalized.layout["DP-1"].state.identity;

        assert_eq!(identity.make.as_deref(), Some("Dell"));
        assert_eq!(identity.model.as_deref(), Some("U2720Q"));
    });
}

#[test]
fn state_store_drops_legacy_default_profile_on_write() {
    with_test_dirs(|_| {
        let state_store = StateStore::new().unwrap();
        let legacy_state = [
            "default_profile = \"desk\"",
            "daemon_enabled = false",
            "[default_profiles]",
            "[known_outputs]",
        ]
        .join("\n");
        std::fs::write(state_store.dir().join("state.toml"), legacy_state).unwrap();

        let loaded = state_store.load_state().unwrap().unwrap();

        let persisted = std::fs::read_to_string(state_store.dir().join("state.toml")).unwrap();
        assert_eq!(
            loaded
                .default_profiles
                .get(State::GLOBAL_DEFAULT_PROFILE_KEY)
                .map(String::as_str),
            Some("desk")
        );
        assert!(!persisted.contains("default_profile ="));
    });
}

#[test]
fn runtime_selects_applies_and_records_matching_profile() {
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

        let selected = runtime::select_profile_for_topology(&topology, &profiles, &state)
            .expect("matching profile should be selected");
        let cycle =
            runtime::execute_plan_cycle_with_backend(&backend, &selected.hooks, false, || {
                let plan = runtime::plan_profile_for_topology(&selected, &topology)?;
                Ok((topology.clone(), plan))
            })
            .unwrap();
        let applied = cycle.apply_result.unwrap();

        assert!(applied.success);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);

        runtime::record_applied_profile(&mut state, &selected.name, Some("test"), &topology);
        assert_eq!(state.last_profile.as_deref(), Some("desk"));
    });
}

#[test]
fn runtime_prefers_setup_default_over_matching_profile() {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("both", "DP-1"), profile("external-only", "DP-1")];
        let mut state = State::default();
        state
            .default_profiles
            .insert(topology.setup_fingerprint(), "external-only".to_string());

        let selected = runtime::select_profile_for_topology(&topology, &profiles, &state)
            .expect("setup default should be selected");

        assert_eq!(selected.name, "external-only");
    });
}

#[derive(Clone)]
struct TestBackend {
    apply_calls: Arc<Mutex<usize>>,
}

impl Backend for TestBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::named("test");
        capabilities.can_enumerate = true;
        capabilities.can_test = true;
        capabilities.can_apply = true;
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

    fn current_state(&self) -> waytorandr_core::error::CoreResult<Topology> {
        Ok(Topology::default())
    }

    fn test(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<TestResult> {
        let mut result = TestResult::default();
        result.success = true;
        Ok(result)
    }

    fn apply(&self, plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<ApplyResult> {
        *self.apply_calls.lock().unwrap() += 1;
        let mut result = ApplyResult::default();
        result.success = true;
        result.applied_state = Some(Topology {
            outputs: plan.outputs.clone(),
        });
        Ok(result)
    }
}

#[test]
fn runtime_cycle_applies_plan_once_through_public_api() {
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

        let cycle = runtime::execute_plan_cycle_with_backend(&backend, &hooks, false, || {
            Ok((Topology::default(), plan.clone()))
        })
        .unwrap();
        let result = cycle.apply_result.unwrap();

        assert!(result.success);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);

        let log = std::fs::read_to_string(log_path).unwrap();
        assert!(log.contains("pre"));
        assert!(log.contains("post"));
    });
}
