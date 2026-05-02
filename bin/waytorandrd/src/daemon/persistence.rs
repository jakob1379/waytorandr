use anyhow::Result;
use waytorandr_core::{workflow, BackendKind, ConfigFailureKind, Profile, StateStore, Topology};

use super::{topology_outputs_summary, DaemonOutcome};

pub(super) struct DaemonApplyFailureContext<'a> {
    pub(super) profile: &'a Profile,
    pub(super) backend_kind: BackendKind,
    pub(super) topology: &'a Topology,
    pub(super) planned_outputs: &'a str,
    pub(super) outcome: &'a str,
    pub(super) failure_kind: Option<ConfigFailureKind>,
    pub(super) failure_message: Option<&'a str>,
    pub(super) fallback_message: &'a str,
}

pub(super) fn daemon_apply_failure_context(context: &DaemonApplyFailureContext<'_>) -> String {
    let message = context.failure_message.unwrap_or(context.fallback_message);
    let failure_kind = context
        .failure_kind
        .map_or("unknown", ConfigFailureKind::as_label);
    format!(
        "daemon failed to apply profile '{}' with {} backend ({outcome}, failure_kind={failure_kind}): {message}; current_outputs={}; planned_outputs={planned_outputs}",
        context.profile.name,
        context.backend_kind,
        topology_outputs_summary(context.topology),
        outcome = context.outcome,
        planned_outputs = context.planned_outputs,
    )
}

pub(super) fn record_daemon_apply_outcome(
    state_store: &StateStore,
    recorded_profile_name: Option<&str>,
    backend: BackendKind,
    topology: &Topology,
    already_matching: bool,
) -> Result<()> {
    if let Some(profile_name) = recorded_profile_name {
        persist_daemon_applied_runtime_state(state_store, profile_name, backend, topology)?;
        log_profile_outcome(profile_name, topology, already_matching);
    } else {
        persist_daemon_observed_runtime_state(state_store, backend, topology)?;
        log_remembered_layout_outcome(topology, already_matching);
    }

    Ok(())
}

fn log_profile_outcome(profile_name: &str, topology: &Topology, already_matching: bool) {
    if already_matching {
        tracing::info!(
            profile = %profile_name,
            topology = %topology_outputs_summary(topology),
            "profile already matches current topology"
        );
    } else {
        tracing::info!(
            profile = %profile_name,
            topology = %topology_outputs_summary(topology),
            "applied profile"
        );
    }
}

fn log_remembered_layout_outcome(topology: &Topology, already_matching: bool) {
    if already_matching {
        tracing::info!(
            topology = %topology_outputs_summary(topology),
            "remembered layout already matches current topology"
        );
    } else {
        tracing::info!(
            topology = %topology_outputs_summary(topology),
            "applied remembered layout"
        );
    }
}

pub(super) fn remember_current_topology(
    state_store: &StateStore,
    backend: BackendKind,
    topology: &Topology,
) -> Result<DaemonOutcome> {
    if !topology.has_enabled_real_outputs() {
        tracing::warn!(
            fingerprint = %topology.setup_fingerprint(),
            "skipping remembered layout update because current topology has no enabled real outputs"
        );
        record_daemon_started(state_store, backend)?;
        return Ok(DaemonOutcome::NoMatch);
    }

    persist_daemon_observed_runtime_state(state_store, backend, topology)?;
    Ok(DaemonOutcome::NoMatch)
}

pub(crate) fn record_daemon_started(state_store: &StateStore, backend: BackendKind) -> Result<()> {
    state_store.update_state(|state| {
        state.record_daemon_started(backend);
        Ok(())
    })?;
    Ok(())
}

fn persist_daemon_applied_runtime_state(
    state_store: &StateStore,
    profile_name: &str,
    backend: BackendKind,
    topology: &Topology,
) -> Result<()> {
    workflow::persist_applied_runtime_state(state_store, profile_name, Some(backend), topology)?;
    record_daemon_started(state_store, backend)
}

fn persist_daemon_observed_runtime_state(
    state_store: &StateStore,
    backend: BackendKind,
    topology: &Topology,
) -> Result<()> {
    workflow::persist_observed_runtime_state(state_store, Some(backend), topology)?;
    record_daemon_started(state_store, backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use waytorandr_core::{OutputState, Profile};

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

    fn with_test_state_dir<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _guard = super::super::xdg_test_guard();
        let unique = format!(
            "waytorandrd-persistence-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let config_home = root.join("config");
        let state_home = root.join("state");
        std::fs::create_dir_all(&config_home)?;
        std::fs::create_dir_all(&state_home)?;
        let _config_home = scoped_env_var("XDG_CONFIG_HOME", &config_home);
        let _state_home = scoped_env_var("XDG_STATE_HOME", &state_home);

        let result = f();
        let _ = std::fs::remove_dir_all(root);
        result
    }

    fn daemon_persistence_topology(connector: &str) -> Topology {
        let mut topology = Topology::new();
        let mut output = OutputState::new(connector);
        output.enabled = true;
        topology.outputs.insert(connector.to_string(), output);
        topology
    }

    #[test]
    fn daemon_apply_failure_context_includes_profile_backend_and_outputs() {
        let profile = Profile::new("desk", 0, Vec::new(), Default::default());
        let topology = Topology {
            outputs: std::collections::HashMap::from([(
                "DP-1".to_string(),
                OutputState::new("DP-1"),
            )]),
        };

        let context = DaemonApplyFailureContext {
            profile: &profile,
            backend_kind: BackendKind::Test,
            topology: &topology,
            planned_outputs: "DP-1:on",
            outcome: "apply failed",
            failure_kind: None,
            failure_message: Some("backend rejected"),
            fallback_message: "fallback",
        };
        let message = daemon_apply_failure_context(&context);

        assert!(message.contains("profile 'desk'"));
        assert!(message.contains("test backend"));
        assert!(message.contains("current_outputs="));
        assert!(message.contains("planned_outputs=DP-1:on"));
    }

    #[test]
    fn persist_daemon_applied_runtime_state_records_lifecycle() -> anyhow::Result<()> {
        with_test_state_dir(|| {
            let store = StateStore::bootstrap()?;
            let topology = daemon_persistence_topology("DP-2");

            persist_daemon_applied_runtime_state(&store, "desk", BackendKind::Wlroots, &topology)?;

            let state = store
                .load_state()?
                .ok_or_else(|| anyhow::anyhow!("persisted state"))?;
            assert_eq!(state.last_profile.as_deref(), Some("desk"));
            assert_eq!(state.backend, Some(BackendKind::Wlroots));
            assert!(state.daemon_enabled);
            Ok(())
        })
    }

    #[test]
    fn persist_daemon_observed_runtime_state_records_lifecycle() -> anyhow::Result<()> {
        with_test_state_dir(|| {
            let store = StateStore::bootstrap()?;
            let topology = daemon_persistence_topology("DP-3");

            persist_daemon_observed_runtime_state(&store, BackendKind::Gnome, &topology)?;

            let state = store
                .load_state()?
                .ok_or_else(|| anyhow::anyhow!("persisted state"))?;
            assert_eq!(state.last_profile, None);
            assert_eq!(state.backend, Some(BackendKind::Gnome));
            assert!(state.daemon_enabled);
            Ok(())
        })
    }
}
