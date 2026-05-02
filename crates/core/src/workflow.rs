mod execution;
mod persistence;
mod selection;
mod topology;

pub use execution::{
    apply_plan, apply_prepared_profile_workflow, apply_preset_workflow,
    apply_preset_workflow_with_policy, apply_profile_workflow, apply_profile_workflow_with_policy,
    prepare_profile_application, validate_plan, validate_preset_workflow,
    validate_profile_workflow, ApplyExecution, ApplyPolicy, PreparedPlanApplication,
    ValidationExecution,
};
pub use persistence::{
    persist_applied_runtime_state, persist_observed_runtime_state,
    set_setup_name_for_setup_in_store,
};
pub use selection::{
    current_profile_name, plan_profile_for_topology, profile_from_topology,
    select_profile_application_target, select_profile_for_topology, ProfileSelectionDecision,
};
pub use topology::{normalized_topology_from_backend, observed_topology_from_backend};

#[cfg(test)]
mod tests {
    use crate::model::{BackendKind, OutputState, Topology};
    use crate::state::State;
    use std::collections::HashMap;

    fn workflow_root_output_state(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state
    }

    #[test]
    fn record_applied_profile_updates_runtime_state() {
        let mut state = State::default();
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), workflow_root_output_state("DP-1"))]),
        };

        state.record_applied_profile("desk", Some(BackendKind::Wlroots), &topology);

        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert!(state.last_topology_fingerprint.is_some());
        assert_eq!(
            state
                .remembered_setups
                .get(&topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
    }

    #[test]
    fn record_observed_topology_clears_last_profile_and_remembers_setup() {
        let mut state = State::default();
        state.last_profile = Some("desk".to_string());
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), workflow_root_output_state("DP-1"))]),
        };

        state.record_observed_topology(Some(BackendKind::Wlroots), &topology);

        assert_eq!(state.last_profile, None);
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
        assert_eq!(
            state
                .remembered_topology_for_setup(&topology.setup_fingerprint())
                .map(Topology::fingerprint),
            Some(topology.fingerprint())
        );
    }
}
