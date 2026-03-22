use anyhow::{anyhow, bail, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_core::store::{ProfileStore, StateStore};
use waytorandr_core::{Backend, Matcher, Planner, Profile, Topology};

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let backend = waytorandr_wlroots::WlrootsBackend::connect()
        .map_err(|err| anyhow!("failed to connect to wlroots backend: {err}"))?;
    let capabilities = backend.capabilities();
    let store = ProfileStore::new()?;
    let state_store = StateStore::new()?;
    let mut watcher = backend.watch_outputs()?;

    let mut state = state_store.load_state()?.unwrap_or_default();
    state.daemon_enabled = true;
    state.backend = Some(capabilities.backend_name.clone());
    state_store.save_state(&state)?;

    let initial = backend.enumerate_outputs()?;
    maybe_apply_matching_profile(&backend, &store, &state_store, &initial)?;

    tracing::info!(backend = %capabilities.backend_name, "daemon ready, watching outputs");

    loop {
        if let Some(topology) = watcher.poll_changed()? {
            tracing::info!(fingerprint = %topology.fingerprint(), "topology changed");
            if let Err(err) = maybe_apply_matching_profile(&backend, &store, &state_store, &topology) {
                tracing::error!(error = %err, "failed to apply matching profile");
            }
        }
    }
}

fn maybe_apply_matching_profile(
    backend: &waytorandr_wlroots::WlrootsBackend,
    store: &ProfileStore,
    state_store: &StateStore,
    topology: &Topology,
) -> Result<()> {
    let profiles = store.list()?;
    let selected = if let Some(matched) = Matcher::match_profile(topology, &profiles) {
        matched.profile
    } else {
        let state = state_store.load_state()?.unwrap_or_default();
        match state.default_profile {
            Some(default_name) => store
                .get(&default_name)?
                .ok_or_else(|| anyhow!("default profile '{}' is missing", default_name))?,
            None => {
                tracing::info!("no matching profile and no default configured");
                return Ok(());
            }
        }
    };

    apply_profile(backend, state_store, &selected, topology)
}

fn apply_profile(
    backend: &waytorandr_wlroots::WlrootsBackend,
    state_store: &StateStore,
    profile: &Profile,
    topology: &Topology,
) -> Result<()> {
    let matched = Matcher::match_profile(topology, std::slice::from_ref(profile))
        .ok_or_else(|| anyhow!("profile '{}' does not match the current topology", profile.name))?;
    let plan = Planner::plan_from_profile(&matched, topology)?;
    let test = backend.test(&plan)?;

    if !test.success {
        bail!(test.message.unwrap_or_else(|| "backend rejected configuration".to_string()));
    }

    let result = backend.apply(&plan)?;
    if !result.success {
        bail!(result.message.unwrap_or_else(|| "backend failed to apply configuration".to_string()));
    }

    let applied = result.applied_state.unwrap_or_else(|| topology.clone());
    let mut state = state_store.load_state()?.unwrap_or_default();
    state.last_profile = Some(profile.name.clone());
    state.last_topology_fingerprint = Some(applied.fingerprint());
    state.backend = Some("wlroots".to_string());
    state.daemon_enabled = true;
    state_store.save_state(&state)?;

    tracing::info!(profile = %profile.name, "applied profile");
    Ok(())
}
