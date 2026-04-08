use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_backend_loader::connect_backend;
use waytorandr_core::runtime;
use waytorandr_core::store::{ProfileStore, StateStore};

mod daemon;

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let backend = connect_backend()?;
    let capabilities = backend.capabilities();
    let store = ProfileStore::new()?;
    let state_store = StateStore::new()?;
    let mut watcher = backend.watch_outputs()?;

    let mut state = state_store.load_state()?.unwrap_or_default();
    runtime::record_daemon_started(&mut state, &capabilities.backend_name);
    state_store.save_state(&state)?;

    daemon::handle_topology_change(backend.as_ref(), &store, &state_store)?;

    tracing::info!(backend = %capabilities.backend_name, "daemon ready, watching outputs");

    loop {
        if let Some(topology) = watcher.poll_changed()? {
            let topology = state_store.normalize_topology_and_persist(&topology)?;
            tracing::info!(fingerprint = %topology.fingerprint(), "topology changed");
            if let Err(err) = daemon::handle_topology_change(backend.as_ref(), &store, &state_store)
            {
                tracing::error!(error = %err, "failed to apply matching profile");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waytorandr_core::store::State;

    #[test]
    fn record_daemon_start_marks_backend_and_enablement() {
        let mut state = State::default();

        runtime::record_daemon_started(&mut state, "wlroots");

        assert!(state.daemon_enabled);
        assert_eq!(state.backend.as_deref(), Some("wlroots"));
    }
}
