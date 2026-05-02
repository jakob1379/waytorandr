use std::collections::HashMap;

use super::matcher::MatchedOutputBinding;
pub use crate::error::PlanError;
use crate::model::{OutputIdentity, OutputState, Topology, VirtualPreset};
use crate::profile::Profile;

mod presets;

pub use presets::detect_preset;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LayoutPlan {
    pub outputs: HashMap<String, OutputState>,
    pub preset_used: Option<VirtualPreset>,
}

pub struct Planner;

impl LayoutPlan {
    #[must_use]
    pub fn new(outputs: HashMap<String, OutputState>) -> Self {
        Self {
            outputs,
            preset_used: None,
        }
    }

    #[must_use]
    pub fn with_preset_used(mut self, preset_used: VirtualPreset) -> Self {
        self.preset_used = Some(preset_used);
        self
    }
}

impl Planner {
    /// Build a layout plan from a matched profile.
    ///
    /// # Errors
    /// Returns `MissingOutput` when a matched topology name cannot be resolved.
    pub fn plan_from_profile(
        profile: &Profile,
        matched_outputs: &[MatchedOutputBinding],
        topology: &Topology,
    ) -> Result<LayoutPlan, PlanError> {
        let mut planned: HashMap<String, OutputState> = HashMap::new();

        for binding in matched_outputs {
            let output_state = topology.outputs.get(&binding.topology_name);
            let config = profile.layout.get(&binding.layout_name).cloned();

            let state = match (config, output_state) {
                (Some(mut cfg), Some(output)) => {
                    cfg.state.identity = output.identity.clone();
                    cfg.state
                }
                (Some(cfg), None) => cfg.state,
                (None, Some(state)) => state.clone(),
                (None, None) => {
                    return Err(PlanError::MissingOutput(binding.topology_name.clone()))
                }
            };

            planned.insert(binding.topology_name.clone(), state);
        }

        Ok(LayoutPlan {
            outputs: planned,
            preset_used: None,
        })
    }

    /// Build a layout plan from a named preset.
    ///
    /// # Errors
    /// Returns `InvalidConfiguration` when the topology cannot satisfy the preset.
    pub fn plan_from_preset(
        preset: VirtualPreset,
        topology: &Topology,
        builtin_output: Option<&OutputIdentity>,
        primary_hint: Option<&str>,
    ) -> Result<LayoutPlan, PlanError> {
        presets::plan_from_preset(preset, topology, builtin_output, primary_hint)
    }
}

#[cfg(test)]
mod tests;
