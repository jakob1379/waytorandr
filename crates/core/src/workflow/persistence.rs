//! Runtime-state persistence helpers for completed workflow outcomes.

use crate::error::CoreResult;
use crate::model::{BackendKind, Topology};
use crate::state::StateStore;

pub fn persist_applied_runtime_state(
    state_store: &StateStore,
    profile_name: &str,
    backend: Option<BackendKind>,
    topology: &Topology,
) -> CoreResult<()> {
    state_store.update_observed_topology(topology, |state, topology| {
        state.record_applied_profile(profile_name, backend, topology);
        Ok(())
    })
}

pub fn persist_observed_runtime_state(
    state_store: &StateStore,
    backend: Option<BackendKind>,
    topology: &Topology,
) -> CoreResult<()> {
    state_store.update_observed_topology(topology, |state, topology| {
        state.record_observed_topology(backend, topology);
        Ok(())
    })
}

pub fn set_setup_name_for_setup_in_store(
    state_store: &StateStore,
    setup_fingerprint: &str,
    setup_name: &str,
) -> CoreResult<()> {
    state_store.update_state(|state| {
        state.set_setup_name_for_setup(setup_fingerprint, setup_name);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OutputState, Topology};
    use crate::test_support::{xdg_lock, XdgTestEnv};

    fn workflow_persistence_topology(connector: &str) -> Topology {
        let mut topology = Topology::new();
        let mut output = OutputState::new(connector);
        output.enabled = true;
        topology.outputs.insert(connector.to_string(), output);
        topology
    }

    #[test]
    fn persist_applied_runtime_state_records_profile_and_topology() -> anyhow::Result<()> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = XdgTestEnv::new()?;
        let store = StateStore::bootstrap()?;
        let topology = workflow_persistence_topology("DP-1");

        persist_applied_runtime_state(&store, "desk", Some(BackendKind::Wlroots), &topology)?;

        let state = store
            .load_state()?
            .ok_or_else(|| anyhow::anyhow!("persisted state"))?;
        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert_eq!(
            state
                .remembered_topology_for_setup(&topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
        assert!(env.state_file().exists());
        Ok(())
    }

    #[test]
    fn persist_observed_runtime_state_clears_profile_and_remembers_setup() -> anyhow::Result<()> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = XdgTestEnv::new()?;
        let store = StateStore::bootstrap()?;
        let topology = workflow_persistence_topology("eDP-1");

        persist_applied_runtime_state(&store, "desk", Some(BackendKind::Wlroots), &topology)?;
        persist_observed_runtime_state(&store, Some(BackendKind::KScreen), &topology)?;

        let state = store
            .load_state()?
            .ok_or_else(|| anyhow::anyhow!("persisted state"))?;
        assert_eq!(state.last_profile, None);
        assert_eq!(state.backend, Some(BackendKind::KScreen));
        assert_eq!(
            state
                .remembered_topology_for_setup(&topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
        Ok(())
    }

    #[test]
    fn setup_name_update_preserves_observed_runtime_state() -> anyhow::Result<()> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = XdgTestEnv::new()?;
        let store = StateStore::bootstrap()?;
        let topology = workflow_persistence_topology("HDMI-A-1");

        set_setup_name_for_setup_in_store(&store, &topology.setup_fingerprint(), "office")?;
        persist_observed_runtime_state(&store, Some(BackendKind::Gnome), &topology)?;

        let state = store
            .load_state()?
            .ok_or_else(|| anyhow::anyhow!("persisted state"))?;
        assert_eq!(
            state.setup_name_for_setup(&topology.setup_fingerprint()),
            Some("office")
        );
        assert_eq!(state.backend, Some(BackendKind::Gnome));
        assert!(!state.daemon_enabled);
        Ok(())
    }
}
