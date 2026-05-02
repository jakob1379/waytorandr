use crate::error::CoreResult;
use crate::model::Topology;
use crate::planning::normalize_topology_with_known_outputs;
use crate::state::{StateReader, StateStore};

/// Load the backend topology and normalize it using stored known outputs.
///
/// # Errors
/// Returns an error if the backend cannot enumerate outputs, or if the state store
/// cannot be read.
pub fn normalized_topology_from_backend<B: crate::engine::Backend + ?Sized>(
    backend: &B,
    state_store: &impl StateReader,
) -> CoreResult<Topology> {
    let topology = backend.enumerate_outputs()?;
    let state = state_store.load_state()?.unwrap_or_default();
    Ok(normalize_topology_with_known_outputs(
        &topology,
        &state.known_outputs,
    ))
}

/// Load the backend topology and persist it as the latest observed state.
///
/// # Errors
/// Returns an error if the backend cannot enumerate outputs, or if the observed
/// topology cannot be persisted.
pub fn observed_topology_from_backend<B: crate::engine::Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
) -> CoreResult<Topology> {
    let topology = backend.enumerate_outputs()?;
    let backend_kind = backend.capabilities().backend;
    state_store.update_observed_topology(&topology, |state, normalized| {
        state.record_observed_topology(Some(backend_kind), normalized);
        Ok(normalized.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{ApplyResult, OutputWatcher, ValidationResult};
    use crate::error::{CoreError, CoreResult};
    use crate::model::{BackendKind, Capabilities, OutputIdentity, OutputState};
    use crate::planning::LayoutPlan;
    use crate::test_support::{xdg_lock, XdgTestEnv};
    use std::collections::HashMap;

    struct TopologyBackend {
        topology: Topology,
    }

    impl crate::engine::Backend for TopologyBackend {
        fn capabilities(&self) -> Capabilities {
            Capabilities::new(BackendKind::Test)
        }

        fn enumerate_outputs(&self) -> CoreResult<Topology> {
            Ok(self.topology.clone())
        }

        fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used"),
            })
        }

        fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
            let _ = plan;
            Ok(ValidationResult::unsupported(None))
        }

        fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            let _ = plan;
            Ok(ApplyResult::failed(None, None))
        }
    }

    fn workflow_topology(connector: &str) -> Topology {
        let mut output = OutputState::new(connector);
        output.enabled = true;
        Topology {
            outputs: HashMap::from([(connector.to_string(), output)]),
        }
    }

    #[test]
    fn normalized_topology_from_backend_uses_cached_identity() -> anyhow::Result<()> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = XdgTestEnv::new()?;
        let state_store = StateStore::bootstrap()?;
        state_store.update_state(|state| {
            state.known_outputs.insert(
                "DP-1".to_string(),
                OutputIdentity {
                    make: Some("Dell".to_string()),
                    connector: Some("DP-1".to_string()),
                    ..OutputIdentity::default()
                },
            );
            Ok(())
        })?;
        let backend = TopologyBackend {
            topology: workflow_topology("DP-1"),
        };

        let normalized = normalized_topology_from_backend(&backend, &state_store)?;

        assert_eq!(
            normalized.outputs["DP-1"].identity.make.as_deref(),
            Some("Dell")
        );
        Ok(())
    }

    #[test]
    fn observed_topology_from_backend_persists_state() -> anyhow::Result<()> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = XdgTestEnv::new()?;
        let state_store = StateStore::bootstrap()?;
        let topology = workflow_topology("eDP-1");
        let backend = TopologyBackend {
            topology: topology.clone(),
        };

        let observed = observed_topology_from_backend(&backend, &state_store)?;

        assert_eq!(observed.outputs, topology.outputs);
        let state = state_store
            .load_state()?
            .ok_or_else(|| anyhow::anyhow!("persisted state"))?;
        assert_eq!(
            state
                .remembered_topology_for_setup(&topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
        assert!(env.state_file().exists());
        Ok(())
    }
}
