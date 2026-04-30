use anyhow::Result;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_backend_loader::connect_backend;
use waytorandr_core::state::StateStore;
use waytorandr_core::store::ProfileStore;
use waytorandr_core::workflow;

mod daemon;

const WATCHER_RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut backend = connect_backend()?;
    let mut capabilities = backend.capabilities();
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let mut watcher = backend.watch_outputs()?;

    workflow::record_daemon_started_in_store(&state_store, capabilities.backend)?;

    if let Err(err) = daemon::enforce_topology_policy(backend.as_ref(), &store, &state_store) {
        tracing::error!(error = %err, "failed to apply matching profile");
    }

    tracing::info!(backend = %capabilities.backend, "daemon ready, watching outputs");

    loop {
        match watcher.poll_changed() {
            Ok(Some(topology)) => {
                workflow::persist_observed_runtime_state(
                    &state_store,
                    Some(capabilities.backend),
                    &topology,
                )?;
                tracing::info!(fingerprint = %topology.fingerprint(), "topology changed");
                if let Err(err) =
                    daemon::enforce_topology_policy(backend.as_ref(), &store, &state_store)
                {
                    tracing::error!(error = %err, "failed to apply matching profile");
                }
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(error = %err, "output watcher failed; reconnecting backend");
                loop {
                    std::thread::sleep(WATCHER_RECONNECT_INTERVAL);
                    match connect_backend().and_then(|next_backend| {
                        let next_capabilities = next_backend.capabilities();
                        let next_watcher = next_backend.watch_outputs()?;
                        Ok((next_backend, next_capabilities, next_watcher))
                    }) {
                        Ok((next_backend, next_capabilities, next_watcher)) => {
                            backend = next_backend;
                            capabilities = next_capabilities;
                            watcher = next_watcher;
                            workflow::record_daemon_started_in_store(
                                &state_store,
                                capabilities.backend,
                            )?;
                            tracing::info!(backend = %capabilities.backend, "backend reconnected");
                            if let Err(err) = daemon::enforce_topology_policy(
                                backend.as_ref(),
                                &store,
                                &state_store,
                            ) {
                                tracing::error!(error = %err, "failed to apply matching profile after reconnect");
                            }
                            break;
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "backend reconnect failed");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use waytorandr_core::state::State;

    #[test]
    fn record_daemon_start_marks_backend_and_enablement() {
        let mut state = State::default();
        state.record_daemon_started(waytorandr_core::model::BackendKind::Wlroots);

        assert!(state.daemon_enabled);
        assert_eq!(
            state.backend,
            Some(waytorandr_core::model::BackendKind::Wlroots)
        );
    }
}
