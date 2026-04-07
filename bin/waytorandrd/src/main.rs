use anyhow::{anyhow, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_core::engine::Backend;
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

fn connect_backend() -> Result<Box<dyn Backend>> {
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".to_string());
    let xdg_runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "<unset>".to_string());
    let display_hint = if wayland_display.contains('/') {
        "; WAYLAND_DISPLAY should be a socket name like 'wayland-0', not a path"
    } else {
        ""
    };
    let prefer_kscreen = is_kde_session();
    let mut errors = Vec::new();

    for (label, attempt) in backend_attempts(prefer_kscreen) {
        match attempt() {
            Ok(backend) => return Ok(backend),
            Err(err) => errors.push(format!("{label}: {err}")),
        }
    }

    Err(anyhow!(
        "failed to connect to a supported backend (WAYLAND_DISPLAY={wayland_display}, XDG_RUNTIME_DIR={xdg_runtime_dir}{display_hint}); attempts: {}",
        errors.join("; ")
    ))
}

fn backend_attempts(prefer_kscreen: bool) -> Vec<(&'static str, fn() -> Result<Box<dyn Backend>>)> {
    let kscreen = (
        "kscreen",
        connect_kscreen as fn() -> Result<Box<dyn Backend>>,
    );
    let wlroots = (
        "wlroots",
        connect_wlroots as fn() -> Result<Box<dyn Backend>>,
    );
    if prefer_kscreen {
        vec![kscreen, wlroots]
    } else {
        vec![wlroots, kscreen]
    }
}

fn connect_kscreen() -> Result<Box<dyn Backend>> {
    waytorandr_kscreen::backend::KScreenBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

fn connect_wlroots() -> Result<Box<dyn Backend>> {
    waytorandr_wlroots::backend::WlrootsBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

fn is_kde_session() -> bool {
    [
        std::env::var("XDG_CURRENT_DESKTOP").ok(),
        std::env::var("XDG_SESSION_DESKTOP").ok(),
        std::env::var("DESKTOP_SESSION").ok(),
    ]
    .into_iter()
    .flatten()
    .any(|value| {
        let value = value.to_ascii_lowercase();
        value.contains("kde") || value.contains("plasma")
    })
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
