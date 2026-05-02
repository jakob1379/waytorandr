use std::collections::HashMap;

use super::support::{
    config_path, legacy_config_path, public_workflow_output_state, public_workflow_profile,
    state_path, with_test_dirs, TestResult,
};
use waytorandr_core::{
    OutputIdentity, Profile, ProfileQueryContext, ProfileStore, State, StateStore,
};

fn query_context(
    state_store: &StateStore,
) -> Result<ProfileQueryContext, Box<dyn std::error::Error>> {
    Ok(ProfileQueryContext::load(state_store)?)
}

#[test]
fn profile_store_roundtrips_saved_profiles_per_setup() -> TestResult {
    with_test_dirs(|temp| {
        let store = ProfileStore::bootstrap()?;
        let profile = public_workflow_profile("desk", "DP-1");
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
            .get_for_setup("desk", &setup_fingerprint, &query_context(&state_store)?)?
            .ok_or_else(|| std::io::Error::other("desk profile should exist"))?;
        assert_eq!(loaded.profile.name, "desk");
        assert_eq!(loaded.setup_fingerprint, setup_fingerprint);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_returns_canonical_match_ready_profiles() -> TestResult {
    with_test_dirs(|_| {
        let store = ProfileStore::bootstrap()?;
        let mut state = State::default();
        state.known_outputs.insert("DP-1".to_string(), {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Dell".to_string());
            identity.model = Some("U2720Q".to_string());
            identity
        });
        let profile = public_workflow_profile("desk", "DP-1");

        let state_store = StateStore::bootstrap()?;
        state_store.save_state(&state)?;
        store.save(&profile, &state_store)?;

        let setup_fingerprint = store
            .list(&query_context(&state_store)?)?
            .into_iter()
            .find(|stored| stored.profile.name == "desk")
            .ok_or_else(|| std::io::Error::other("desk profile should be listed"))?
            .setup_fingerprint;
        let loaded = store
            .get_for_setup("desk", &setup_fingerprint, &query_context(&state_store)?)?
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
fn profile_store_migrates_legacy_profiles_to_json_file() -> TestResult {
    with_test_dirs(|temp| {
        let legacy_profile = public_workflow_profile("desk", "DP-1");
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
            .get_for_setup("desk", &setup_fingerprint, &query_context(&state_store)?)?
            .is_some());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn state_store_normalizes_profile_using_cached_outputs() -> TestResult {
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
        let normalized = waytorandr_core::normalize_profile_with_known_outputs(
            &public_workflow_profile("desk", "DP-1"),
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
fn profile_store_migrates_legacy_defaults_into_profiles_json() -> TestResult {
    with_test_dirs(|_| {
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

        assert_eq!(settings.setup_default_profile("conn:DP-1"), Some("office"));
        assert!(!persisted.contains("default_profile = \"desk\""));
        assert!(!persisted.contains("conn:DP-1"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_migrates_legacy_json_filename() -> TestResult {
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
                "profiles": [public_workflow_profile("desk", "DP-1")]
            }))?,
        )?;

        let store = ProfileStore::bootstrap()?;
        let state_store = StateStore::bootstrap()?;
        let fingerprint = public_workflow_profile("desk", "DP-1").setup_fingerprint();

        assert!(!legacy_path.exists());
        assert!(config_path(temp).exists());
        assert!(store
            .get_for_setup("desk", &fingerprint, &query_context(&state_store)?)?
            .is_some());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn remove_for_setup_clears_deleted_profiles_setup_default_only() -> TestResult {
    with_test_dirs(|_| {
        let store = ProfileStore::bootstrap()?;
        let state_store = StateStore::bootstrap()?;
        let desk = public_workflow_profile("desk", "DP-1");
        let travel = public_workflow_profile("desk", "eDP-1");
        let desk_setup = desk.setup_fingerprint();

        store.save(&desk, &state_store)?;
        store.save(&travel, &state_store)?;
        store.set_setup_default_profile(&desk_setup, "desk")?;
        assert!(store.remove_for_setup("desk", &desk_setup, &state_store)?);

        let settings = store.settings()?;
        assert_eq!(settings.setup_default_profile(&desk_setup), None);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn remove_unique_clears_all_defaults_for_deleted_profile() -> TestResult {
    with_test_dirs(|_| {
        let store = ProfileStore::bootstrap()?;
        let state_store = StateStore::bootstrap()?;
        let desk = public_workflow_profile("desk", "DP-1");
        let desk_setup = desk.setup_fingerprint();

        store.save(&desk, &state_store)?;
        store.set_setup_default_profile(&desk_setup, "desk")?;
        assert!(store.remove_unique("desk")?);

        let settings = store.settings()?;
        assert_eq!(settings.setup_default_profile(&desk_setup), None);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_open_honors_legacy_json_fallback() -> TestResult {
    with_test_dirs(|temp| {
        let legacy_path = legacy_config_path(temp);
        let legacy_profile = public_workflow_profile("desk", "DP-1");
        let setup_fingerprint = legacy_profile.setup_fingerprint();
        let setup_defaults = serde_json::Map::from_iter([(
            setup_fingerprint.clone(),
            serde_json::Value::String("desk".to_string()),
        )]);
        std::fs::create_dir_all(
            legacy_path
                .parent()
                .ok_or_else(|| std::io::Error::other("legacy config parent should exist"))?,
        )?;
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "profiles": [legacy_profile.clone()],
                "settings": {
                    "setup_defaults": setup_defaults
                }
            }))?,
        )?;

        let store = ProfileStore::open()?;
        let state_store = StateStore::bootstrap()?;
        let settings = store.settings()?;

        assert_eq!(
            settings.setup_default_profile(&setup_fingerprint),
            Some("desk")
        );
        assert_eq!(store.list(&query_context(&state_store)?)?.len(), 1);
        assert!(store
            .get_for_setup("desk", &setup_fingerprint, &query_context(&state_store)?)?
            .is_some());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn profile_store_save_via_open_preserves_legacy_json_contents() -> TestResult {
    with_test_dirs(|temp| {
        let legacy_path = legacy_config_path(temp);
        let legacy_profile = public_workflow_profile("desk", "DP-1");
        std::fs::create_dir_all(
            legacy_path
                .parent()
                .ok_or_else(|| std::io::Error::other("legacy config parent should exist"))?,
        )?;
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "profiles": [legacy_profile.clone()]
            }))?,
        )?;

        let store = ProfileStore::open()?;
        let state_store = StateStore::bootstrap()?;
        let new_profile = public_workflow_profile("office", "HDMI-A-1");

        store.save(&new_profile, &state_store)?;

        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config_path(temp))?)?;
        let profile_names: Vec<_> = saved["profiles"]
            .as_array()
            .ok_or_else(|| std::io::Error::other("profiles should be an array"))?
            .iter()
            .filter_map(|profile| profile["name"].as_str())
            .collect();

        assert_eq!(profile_names.len(), 2);
        assert!(profile_names.contains(&"desk"));
        assert!(profile_names.contains(&"office"));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn read_only_state_and_profile_access_do_not_create_or_migrate_files() -> TestResult {
    with_test_dirs(|temp| {
        let legacy_path = legacy_config_path(temp);
        let state_path = state_path(temp);
        std::fs::create_dir_all(
            legacy_path
                .parent()
                .ok_or_else(|| std::io::Error::other("legacy config parent should exist"))?,
        )?;
        std::fs::create_dir_all(
            state_path
                .parent()
                .ok_or_else(|| std::io::Error::other("state parent should exist"))?,
        )?;

        let desk = public_workflow_profile("desk", "DP-1");
        let duplicate_name = public_workflow_profile("desk", "HDMI-A-1");
        let office = public_workflow_profile("office", "eDP-1");
        let mut state = State::default();
        state.known_outputs.insert("DP-1".to_string(), {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Dell".to_string());
            identity.model = Some("U2720Q".to_string());
            identity
        });
        let normalized_desk =
            waytorandr_core::normalize_profile_with_known_outputs(&desk, &state.known_outputs);
        let legacy_profiles_json = serde_json::to_string_pretty(&serde_json::json!({
            "profiles": [desk.clone(), duplicate_name.clone(), office.clone()],
            "settings": {
                "setup_defaults": {
                    desk.setup_fingerprint(): "desk"
                }
            }
        }))?;
        std::fs::write(&legacy_path, &legacy_profiles_json)?;
        state.last_profile = Some("desk".to_string());
        let legacy_state = toml::to_string_pretty(&state)?;
        std::fs::write(&state_path, &legacy_state)?;

        let store = ProfileStore::open_read_only()?;
        let state_store = waytorandr_core::ReadOnlyStateStore::open()?;
        let query_context = ProfileQueryContext::load_read_only(&state_store)?;

        assert_eq!(
            store
                .settings()?
                .setup_default_profile(&desk.setup_fingerprint()),
            Some("desk")
        );
        assert_eq!(store.list_names()?, vec!["desk", "office"]);
        assert_eq!(
            store.list_names_for_setup(&normalized_desk.setup_fingerprint(), &query_context)?,
            vec!["desk"]
        );
        assert_eq!(store.list(&query_context)?.len(), 3);
        assert_eq!(
            store
                .list_for_setup(&normalized_desk.setup_fingerprint(), &query_context)?
                .len(),
            1
        );
        assert_eq!(store.profiles(&query_context)?.len(), 3);
        assert_eq!(
            store
                .profiles_for_setup(&normalized_desk.setup_fingerprint(), &query_context)?
                .len(),
            1
        );
        assert!(!config_path(temp).exists());
        assert_eq!(std::fs::read_to_string(&legacy_path)?, legacy_profiles_json);
        assert_eq!(std::fs::read_to_string(&state_path)?, legacy_state);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn state_store_persists_known_outputs_only_on_explicit_observation() -> TestResult {
    with_test_dirs(|_| {
        let state_store = StateStore::bootstrap()?;
        let topology = waytorandr_core::Topology {
            outputs: HashMap::from([("DP-1".to_string(), public_workflow_output_state("DP-1"))]),
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
fn layout_only_profile_save_is_canonical_and_idempotent() -> TestResult {
    with_test_dirs(|temp| {
        let store = ProfileStore::bootstrap()?;
        let state_store = StateStore::bootstrap()?;
        let mut state = State::default();
        state.known_outputs.insert("DP-1".to_string(), {
            let mut identity = OutputIdentity::new("DP-1");
            identity.make = Some("Dell".to_string());
            identity.model = Some("U2720Q".to_string());
            identity
        });
        state_store.save_state(&state)?;

        let profile = Profile::new(
            "desk",
            0,
            Vec::new(),
            HashMap::from([(
                "DP-1".to_string(),
                public_workflow_output_state("DP-1").into(),
            )]),
        );

        store.save(&profile, &state_store)?;
        store.save(&profile, &state_store)?;

        let listed = store.list(&query_context(&state_store)?)?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].profile.match_rules.len(), 1);
        assert_eq!(
            listed[0].profile.match_rules[0].identity.make.as_deref(),
            Some("Dell")
        );

        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config_path(temp))?)?;
        assert_eq!(persisted["profiles"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            persisted["profiles"][0]["match_rules"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        Ok(())
    })?;
    Ok(())
}
