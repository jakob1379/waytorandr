use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, OnceLock};

use tempfile::TempDir;
use waytorandr_core::engine::{ApplyResult, Backend, ConfigFailureKind, OutputWatcher, TestResult};
use waytorandr_core::error::CoreError;
use waytorandr_core::model::{
    BackendKind, Capabilities, OutputIdentity, OutputState, Position, Topology, VirtualPreset,
};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::profile::{Hook, Hooks, OutputMatcher, Profile};
use waytorandr_core::state::{State, StateStore};
use waytorandr_core::store::{DefaultTarget, ProfileStore, ProfilesSettings};
use waytorandr_core::workflow;

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

fn config_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path()
        .join("config")
        .join("waytorandr")
        .join("waytorandr.json")
}

fn legacy_config_path(temp: &TempDir) -> std::path::PathBuf {
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
        let state_store = StateStore::bootstrap()?;

        store.save(&profile, &state_store)?;

        assert!(config_path(temp).exists());
        assert!(!temp
            .path()
            .join("config")
            .join("waytorandr")
            .join("profiles")
            .exists());

        let loaded = store
            .get_for_setup("desk", &setup_fingerprint, &state_store)?
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

        let state_store = StateStore::bootstrap()?;
        state_store.save_state(&state)?;
        store.save(&profile, &state_store)?;

        let setup_fingerprint = store
            .list(&state_store)?
            .into_iter()
            .find(|stored| stored.profile.name == "desk")
            .ok_or_else(|| std::io::Error::other("desk profile should be listed"))?
            .setup_fingerprint;
        let loaded = store
            .get_for_setup("desk", &setup_fingerprint, &state_store)?
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
        let config_path = config_path(temp);
        let state_store = StateStore::bootstrap()?;

        assert!(!legacy_path.exists());
        assert!(config_path.exists());
        assert!(store
            .get_for_setup("desk", &setup_fingerprint, &state_store)?
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
fn profile_store_migrates_legacy_defaults_into_profiles_json() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|temp| {
        let state_store = StateStore::bootstrap()?;
        let legacy_state = [
            "default_profile = \"desk\"",
            "daemon_enabled = false",
            "[default_profiles]",
            "\"conn:DP-1\" = \"office\"",
            "[known_outputs]",
        ]
        .join("\n");
        std::fs::write(state_store.dir().join("state.toml"), legacy_state)?;

        let store = ProfileStore::bootstrap()?;
        let settings = store.settings()?;
        let persisted = std::fs::read_to_string(state_store.dir().join("state.toml"))?;
        let profiles_json = std::fs::read_to_string(config_path(temp))?;

        assert_eq!(
            settings.new_setup_default,
            Some(DefaultTarget::Profile {
                name: "desk".to_string()
            })
        );
        assert_eq!(settings.setup_default_profile("conn:DP-1"), Some("office"));
        assert!(!persisted.contains("default_profile = \"desk\""));
        assert!(!persisted.contains("conn:DP-1"));
        assert!(profiles_json.contains("new_setup_default"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_migrates_legacy_json_filename() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|temp| {
        let legacy_path = legacy_config_path(temp);
        std::fs::create_dir_all(
            legacy_path
                .parent()
                .ok_or_else(|| std::io::Error::other("legacy config parent should exist"))?,
        )?;
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "profiles": [profile("desk", "DP-1")]
            }))?,
        )?;

        let store = ProfileStore::bootstrap()?;
        let state_store = StateStore::bootstrap()?;
        let fingerprint = profile("desk", "DP-1").setup_fingerprint();

        assert!(!legacy_path.exists());
        assert!(config_path(temp).exists());
        assert!(store
            .get_for_setup("desk", &fingerprint, &state_store)?
            .is_some());
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
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology: topology.clone(),
            test_result: TestResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };
        let profiles = vec![profile("desk", "DP-1"), profile("fallback", "HDMI-A-1")];
        let settings = ProfilesSettings {
            setup_defaults: HashMap::new(),
            new_setup_default: Some(DefaultTarget::Profile {
                name: "fallback".to_string(),
            }),
        };
        let mut state = State::default();

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &settings)
            .ok_or_else(|| std::io::Error::other("matching profile should be selected"))?;
        let cycle = workflow::apply_profile_workflow(&backend, &state_store, &selected)?;
        assert!(
            matches!(cycle, workflow::ApplyExecution::Applied { apply_result, .. } if apply_result.success)
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
        let mut settings = ProfilesSettings::default();
        settings.set_setup_default_profile(&topology.setup_fingerprint(), "external-only");

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &settings)
            .ok_or_else(|| std::io::Error::other("setup default should be selected"))?;

        assert_eq!(selected.name, "external-only");
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_prefers_global_default_profile_over_other_matching_profiles(
) -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1"), profile("laptop", "DP-1")];
        let settings = ProfilesSettings {
            setup_defaults: HashMap::new(),
            new_setup_default: Some(DefaultTarget::Profile {
                name: "laptop".to_string(),
            }),
        };

        let selected = workflow::select_profile_for_topology(&topology, &profiles, &settings)
            .ok_or_else(|| std::io::Error::other("global default profile should be selected"))?;

        assert_eq!(selected.name, "laptop");
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_returns_virtual_default_for_new_setup() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let settings = ProfilesSettings {
            setup_defaults: HashMap::new(),
            new_setup_default: Some(DefaultTarget::Virtual {
                preset: VirtualPreset::Vertical,
            }),
        };

        let selected = workflow::select_target_for_topology(&topology, &[], &settings)
            .ok_or_else(|| std::io::Error::other("virtual default should be selected"))?;

        assert!(matches!(
            selected,
            workflow::SelectedTarget::Virtual(VirtualPreset::Vertical)
        ));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_returns_builtin_default_when_internal_output_exists() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("eDP-1".to_string(), output("eDP-1"))]),
        };
        let settings = ProfilesSettings {
            setup_defaults: HashMap::new(),
            new_setup_default: Some(DefaultTarget::Virtual {
                preset: VirtualPreset::Builtin,
            }),
        };

        let selected = workflow::select_target_for_topology(&topology, &[], &settings)
            .ok_or_else(|| std::io::Error::other("builtin default should be selected"))?;

        assert!(matches!(
            selected,
            workflow::SelectedTarget::Virtual(VirtualPreset::Builtin)
        ));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn runtime_skips_builtin_default_when_no_internal_output_exists() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let settings = ProfilesSettings {
            setup_defaults: HashMap::new(),
            new_setup_default: Some(DefaultTarget::Virtual {
                preset: VirtualPreset::Builtin,
            }),
        };

        assert!(workflow::select_target_for_topology(&topology, &[], &settings).is_none());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn setup_names_persist_per_setup_fingerprint() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let state_store = StateStore::bootstrap()?;
        workflow::set_setup_name_for_setup_in_store(&state_store, "conn:DP-1", "office")?;

        let state = state_store
            .load_state()?
            .ok_or_else(|| std::io::Error::other("state should exist"))?;

        assert_eq!(state.setup_name_for_setup("conn:DP-1"), Some("office"));
        Ok(())
    })?;
    Ok(())
}

#[derive(Clone)]
struct TestBackend {
    topology: Topology,
    test_result: TestResult,
    apply_calls: Arc<Mutex<usize>>,
    apply_success: bool,
    apply_failure: Option<ConfigFailureKind>,
    apply_message: Option<String>,
}

impl Backend for TestBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Test);
        capabilities.can_test = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> waytorandr_core::error::CoreResult<Topology> {
        Ok(self.topology.clone())
    }

    fn watch_outputs(&self) -> waytorandr_core::error::CoreResult<Box<dyn OutputWatcher>> {
        Err(CoreError::Backend {
            source: anyhow::anyhow!("not used in tests"),
        })
    }

    fn test(&self, _plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<TestResult> {
        Ok(self.test_result.clone())
    }

    fn apply(&self, plan: &LayoutPlan) -> waytorandr_core::error::CoreResult<ApplyResult> {
        *self
            .apply_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        let mut result = ApplyResult::default();
        result.success = self.apply_success;
        result.failure = self.apply_failure;
        result.message = self.apply_message.clone();
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
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology,
            test_result: TestResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
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
        let mut profile = profile("desk", "DP-1");
        profile.hooks = hooks;

        let cycle = workflow::apply_profile_workflow(&backend, &state_store, &profile)?;
        assert!(
            matches!(cycle, workflow::ApplyExecution::Applied { apply_result, .. } if apply_result.success)
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

#[test]
fn validate_profile_workflow_returns_accepted_plan() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology: topology.clone(),
            test_result: TestResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: true,
            apply_failure: None,
            apply_message: None,
        };

        let execution =
            workflow::validate_profile_workflow(&backend, &state_store, &profile("desk", "DP-1"))?;

        assert!(matches!(
            execution,
            workflow::ValidationExecution::Accepted {
                ref plan,
                ref validation,
            } if validation.success && plan.outputs.contains_key("DP-1")
        ));
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            0
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn apply_profile_workflow_returns_structured_apply_failures() -> Result<(), Box<dyn Error>> {
    with_test_dirs(|_| {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let state_store = StateStore::bootstrap()?;
        let backend = TestBackend {
            topology,
            test_result: TestResult::supported(None),
            apply_calls: Arc::new(Mutex::new(0)),
            apply_success: false,
            apply_failure: Some(ConfigFailureKind::TopologyChanged),
            apply_message: Some("changed".to_string()),
        };

        let execution =
            workflow::apply_profile_workflow(&backend, &state_store, &profile("desk", "DP-1"))?;

        assert!(matches!(
            execution,
            workflow::ApplyExecution::ApplyFailed { ref apply_result, .. }
                if apply_result.failure == Some(ConfigFailureKind::TopologyChanged)
                    && apply_result.message.as_deref() == Some("changed")
        ));
        assert_eq!(
            *backend
                .apply_calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            1
        );
        Ok(())
    })?;
    Ok(())
}
