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
