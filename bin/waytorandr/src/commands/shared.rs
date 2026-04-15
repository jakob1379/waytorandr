use anyhow::Result;
use serde::Serialize;

use waytorandr_backend_loader::connect_backend;
use waytorandr_core::model::{OutputState, Topology};
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::state::StateStore;
use waytorandr_core::workflow;

#[derive(Serialize)]
pub(super) struct JsonOutputEntry {
    name: String,
    state: OutputState,
}

pub(super) fn plan_outputs(plan: &LayoutPlan) -> Vec<JsonOutputEntry> {
    sorted_outputs(plan.outputs.iter())
}

pub(super) fn topology_outputs(topology: &Topology) -> Vec<JsonOutputEntry> {
    sorted_outputs(topology.outputs.iter())
}

pub(super) fn load_current_topology(state_store: &StateStore) -> Result<Topology> {
    let backend = connect_backend()?;
    Ok(workflow::normalized_topology_from_backend(
        backend.as_ref(),
        state_store,
    )?)
}

fn sorted_outputs<'a, I>(iter: I) -> Vec<JsonOutputEntry>
where
    I: IntoIterator<Item = (&'a String, &'a OutputState)>,
{
    let mut outputs: Vec<JsonOutputEntry> = iter
        .into_iter()
        .map(|(name, state)| JsonOutputEntry {
            name: name.clone(),
            state: state.clone(),
        })
        .collect();
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    outputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;

    #[test]
    fn plan_outputs_are_sorted_by_name() {
        let plan = LayoutPlan::new(HashMap::from([
            ("zeta".to_string(), OutputState::new("zeta")),
            ("alpha".to_string(), OutputState::new("alpha")),
        ]));

        let json = serde_json::to_value(plan_outputs(&plan)).unwrap();

        assert_eq!(
            json,
            Value::Array(vec![
                serde_json::json!({"name": "alpha", "state": OutputState::new("alpha")}),
                serde_json::json!({"name": "zeta", "state": OutputState::new("zeta")}),
            ])
        );
    }

    #[test]
    fn topology_outputs_are_sorted_by_name() {
        let topology = Topology {
            outputs: HashMap::from([
                ("b".to_string(), OutputState::new("b")),
                ("a".to_string(), OutputState::new("a")),
            ]),
        };

        let json = serde_json::to_value(topology_outputs(&topology)).unwrap();

        assert_eq!(
            json,
            Value::Array(vec![
                serde_json::json!({"name": "a", "state": OutputState::new("a")}),
                serde_json::json!({"name": "b", "state": OutputState::new("b")}),
            ])
        );
    }
}
