use crate::engine::{ApplyResult, Backend, Engine, TestResult};
use crate::error::{CoreError, CoreResult};
use crate::matcher::Matcher;
use crate::model::Topology;
use crate::planner::{LayoutPlan, Planner};
use crate::profile::{Hooks, OutputMatcher, Profile};
use crate::store::{State, StateStore};

pub struct ExecutionCycle {
    pub validation_plan: LayoutPlan,
    pub validation: TestResult,
    pub apply_plan: Option<LayoutPlan>,
    pub apply_result: Option<ApplyResult>,
    pub apply_topology: Option<Topology>,
}

pub fn default_profile_for_setup<'a>(state: &'a State, setup_fingerprint: &str) -> Option<&'a str> {
    state
        .default_profiles
        .get(setup_fingerprint)
        .map(String::as_str)
        .or_else(|| state.global_default_profile())
}

pub fn select_profile_for_topology(
    topology: &Topology,
    profiles: &[Profile],
    state: &State,
) -> Option<Profile> {
    if let Some(matched) = Matcher::match_profile(topology, profiles) {
        return Some(matched.profile);
    }

    let setup_fingerprint = topology.setup_fingerprint();
    let default_name = default_profile_for_setup(state, &setup_fingerprint)?;
    profiles
        .iter()
        .find(|profile| profile.name == default_name)
        .cloned()
}

pub fn current_profile_name(
    topology: &Topology,
    profiles: &[Profile],
    state: &State,
) -> Option<String> {
    state
        .last_profile
        .clone()
        .or_else(|| Matcher::match_profile(topology, profiles).map(|matched| matched.profile.name))
}

pub fn profile_from_topology(name: &str, topology: &Topology) -> Profile {
    Profile {
        name: name.to_string(),
        priority: 0,
        match_rules: topology
            .outputs
            .values()
            .filter(|output| !output.identity.is_ignored && !output.identity.is_virtual)
            .map(|output| OutputMatcher {
                identity: output.identity.clone(),
                required: output.enabled,
                position_hint: Some(output.position),
            })
            .collect(),
        layout: topology
            .outputs
            .iter()
            .map(|(output_name, output)| (output_name.clone(), output.clone().into()))
            .collect(),
        hooks: Default::default(),
        options: Default::default(),
    }
}

pub fn plan_profile_for_topology(profile: &Profile, topology: &Topology) -> CoreResult<LayoutPlan> {
    let matched = Matcher::match_profile(topology, std::slice::from_ref(profile))
        .ok_or(CoreError::ProfileMismatch)?;
    Planner::plan_from_profile(&matched.profile, &matched.matched_outputs, topology)
        .map_err(Into::into)
}

pub fn normalized_topology_from_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
) -> CoreResult<Topology> {
    let topology = backend.enumerate_outputs()?;
    state_store.normalize_topology_and_persist(&topology)
}

pub fn plan_profile_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    profile: &Profile,
) -> CoreResult<(Topology, LayoutPlan)> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = plan_profile_for_topology(profile, &topology)?;
    Ok((topology, plan))
}

pub fn plan_preset_with_backend<B: Backend + ?Sized>(
    backend: &B,
    state_store: &StateStore,
    preset: &str,
) -> CoreResult<(Topology, LayoutPlan)> {
    let topology = normalized_topology_from_backend(backend, state_store)?;
    let plan = Planner::plan_from_preset(preset, &topology, None)?;
    Ok((topology, plan))
}

pub fn execute_plan_cycle<B, F>(
    engine: &Engine<B>,
    hooks: &Hooks,
    dry_run: bool,
    mut plan_factory: F,
) -> CoreResult<ExecutionCycle>
where
    B: Backend,
    F: FnMut() -> CoreResult<(Topology, LayoutPlan)>,
{
    let (validation_topology, validation_plan) = plan_factory()?;
    let validation = engine.test_plan(&validation_plan)?;

    if dry_run || !validation.success {
        return Ok(ExecutionCycle {
            validation_plan,
            validation,
            apply_plan: None,
            apply_result: None,
            apply_topology: Some(validation_topology),
        });
    }

    let (apply_topology, apply_plan) = plan_factory()?;
    let apply_result = engine.apply_plan(&apply_plan, hooks)?;

    Ok(ExecutionCycle {
        validation_plan,
        validation,
        apply_plan: Some(apply_plan),
        apply_result: Some(apply_result),
        apply_topology: Some(apply_topology),
    })
}

pub fn execute_plan_cycle_with_backend<B, F>(
    backend: &B,
    hooks: &Hooks,
    dry_run: bool,
    plan_factory: F,
) -> CoreResult<ExecutionCycle>
where
    B: Backend + ?Sized,
    F: FnMut() -> CoreResult<(Topology, LayoutPlan)>,
{
    let engine = Engine::new(backend);
    execute_plan_cycle(&engine, hooks, dry_run, plan_factory)
}

pub fn record_applied_profile(
    state: &mut State,
    profile_name: &str,
    backend: Option<&str>,
    topology: &Topology,
) {
    state.last_profile = Some(profile_name.to_string());
    state.last_topology_fingerprint = Some(topology.fingerprint());
    state.backend = backend.map(str::to_string);
}

pub fn set_default_profile_for_setup(
    state: &mut State,
    setup_fingerprint: &str,
    profile_name: &str,
) {
    state
        .default_profiles
        .insert(setup_fingerprint.to_string(), profile_name.to_string());
}

pub fn record_daemon_started(state: &mut State, backend_name: &str) {
    state.daemon_enabled = true;
    state.backend = Some(backend_name.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::OutputWatcher;
    use crate::model::{OutputIdentity, OutputState};
    use crate::profile::{Hooks, ProfileOptions};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn output(connector: &str) -> OutputState {
        let mut state = OutputState::new(connector);
        state.enabled = true;
        state
    }

    fn profile(name: &str, connector: &str) -> Profile {
        Profile {
            name: name.to_string(),
            priority: 0,
            match_rules: vec![crate::profile::OutputMatcher {
                identity: OutputIdentity::new(connector),
                required: true,
                position_hint: Some(crate::model::Position::default()),
            }],
            layout: HashMap::from([(
                connector.to_string(),
                crate::profile::OutputConfig {
                    state: output(connector),
                    preset: None,
                },
            )]),
            hooks: Hooks::default(),
            options: ProfileOptions::default(),
        }
    }

    #[test]
    fn select_profile_prefers_match_before_default() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let mut state = State::default();
        state.default_profiles.insert(
            State::GLOBAL_DEFAULT_PROFILE_KEY.to_string(),
            "fallback".to_string(),
        );
        let profiles = vec![profile("desk", "DP-1"), profile("fallback", "HDMI-A-1")];

        let selected = select_profile_for_topology(&topology, &profiles, &state).unwrap();

        assert_eq!(selected.name, "desk");
    }

    #[test]
    fn record_applied_profile_updates_runtime_state() {
        let mut state = State::default();
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };

        record_applied_profile(&mut state, "desk", Some("wlroots"), &topology);

        assert_eq!(state.last_profile.as_deref(), Some("desk"));
        assert_eq!(state.backend.as_deref(), Some("wlroots"));
        assert!(state.last_topology_fingerprint.is_some());
    }

    #[test]
    fn current_profile_name_prefers_recorded_profile() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profiles = vec![profile("desk", "DP-1")];
        let mut state = State::default();
        state.last_profile = Some("manual".to_string());

        let selected = current_profile_name(&topology, &profiles, &state);

        assert_eq!(selected.as_deref(), Some("manual"));
    }

    #[test]
    fn profile_from_topology_builds_matchers_from_real_outputs() {
        let mut virtual_output = output("HEADLESS-1");
        virtual_output.identity.is_virtual = true;
        let mut topology = Topology {
            outputs: HashMap::from([
                ("DP-1".to_string(), output("DP-1")),
                ("HEADLESS-1".to_string(), virtual_output),
            ]),
        };
        topology.outputs.get_mut("DP-1").unwrap().enabled = false;

        let profile = profile_from_topology("desk", &topology);

        assert_eq!(profile.name, "desk");
        assert_eq!(profile.layout.len(), 2);
        assert_eq!(profile.match_rules.len(), 1);
        assert_eq!(
            profile.match_rules[0].identity.connector.as_deref(),
            Some("DP-1")
        );
        assert!(!profile.match_rules[0].required);
    }

    #[test]
    fn plan_profile_for_topology_returns_mismatch_error_for_nonmatching_profile() {
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let profile = profile("desk", "HDMI-A-1");

        let result = plan_profile_for_topology(&profile, &topology);

        assert!(matches!(result, Err(CoreError::ProfileMismatch)));
    }

    #[derive(Clone)]
    struct CycleBackend {
        test_success: bool,
        test_calls: Arc<Mutex<usize>>,
        apply_calls: Arc<Mutex<usize>>,
    }

    impl Backend for CycleBackend {
        fn capabilities(&self) -> crate::model::Capabilities {
            let mut capabilities = crate::model::Capabilities::named("cycle-test");
            capabilities.can_enumerate = true;
            capabilities.can_test = true;
            capabilities.can_apply = true;
            capabilities
        }

        fn enumerate_outputs(&self) -> CoreResult<Topology> {
            Ok(Topology::default())
        }

        fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
            Err(CoreError::Backend {
                source: anyhow::anyhow!("not used"),
            })
        }

        fn current_state(&self) -> CoreResult<Topology> {
            Ok(Topology::default())
        }

        fn test(&self, _plan: &LayoutPlan) -> CoreResult<TestResult> {
            *self.test_calls.lock().unwrap() += 1;
            let mut result = TestResult::default();
            result.success = self.test_success;
            Ok(result)
        }

        fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
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
    fn execute_plan_cycle_skips_apply_on_dry_run() {
        let backend = CycleBackend {
            test_success: true,
            test_calls: Arc::new(Mutex::new(0)),
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let engine = Engine::new(backend.clone());
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let hooks = Hooks::default();

        let cycle = execute_plan_cycle(&engine, &hooks, true, || {
            Ok((
                topology.clone(),
                LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            ))
        })
        .unwrap();

        assert!(cycle.validation.success);
        assert!(cycle.apply_plan.is_none());
        assert!(cycle.apply_result.is_none());
        assert_eq!(*backend.test_calls.lock().unwrap(), 1);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 0);
    }

    #[test]
    fn execute_plan_cycle_applies_after_successful_validation() {
        let backend = CycleBackend {
            test_success: true,
            test_calls: Arc::new(Mutex::new(0)),
            apply_calls: Arc::new(Mutex::new(0)),
        };
        let engine = Engine::new(backend.clone());
        let topology = Topology {
            outputs: HashMap::from([("DP-1".to_string(), output("DP-1"))]),
        };
        let hooks = Hooks::default();
        let mut calls = 0usize;

        let cycle = execute_plan_cycle(&engine, &hooks, false, || {
            calls += 1;
            Ok((
                topology.clone(),
                LayoutPlan::new(HashMap::from([("DP-1".to_string(), output("DP-1"))])),
            ))
        })
        .unwrap();

        assert_eq!(calls, 2);
        assert!(cycle.validation.success);
        assert!(cycle.apply_plan.is_some());
        assert!(cycle.apply_result.as_ref().map(|result| result.success) == Some(true));
        assert_eq!(*backend.test_calls.lock().unwrap(), 1);
        assert_eq!(*backend.apply_calls.lock().unwrap(), 1);
    }
}
