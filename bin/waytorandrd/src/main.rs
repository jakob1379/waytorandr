use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_backend_loader::connect_backend;
use waytorandr_core::engine::HookPolicy;
use waytorandr_core::state::StateStore;
use waytorandr_core::store::ProfileStore;
use waytorandr_core::terminal::escape_terminal_text;
use waytorandr_core::workflow;

mod daemon;

const WATCHER_RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Parser)]
#[command(name = "waytorandrd")]
#[command(about = "Daemon for automatically applying waytorandr display profiles")]
#[command(version)]
struct Cli {
    #[arg(
        long = "log-level",
        value_enum,
        value_name = "level",
        conflicts_with = "verbose",
        help = "Set daemon log level"
    )]
    log_level: Option<LogLevel>,

    #[arg(
        short = 'v',
        long = "verbose",
        conflicts_with = "log_level",
        help = "Enable debug logging"
    )]
    verbose: bool,

    #[arg(long = "no-hooks", help = "Disable profile hook execution")]
    no_hooks: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum LogLevel {
    Info,
    Debug,
}

impl Cli {
    fn log_filter(&self) -> &'static str {
        if self.verbose {
            return "debug";
        }

        match self.log_level.unwrap_or(LogLevel::Info) {
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }

    fn explicit_log_filter(&self) -> Option<&'static str> {
        if self.verbose || self.log_level.is_some() {
            Some(self.log_filter())
        } else {
            None
        }
    }

    const fn hook_policy(&self) -> HookPolicy {
        if self.no_hooks {
            HookPolicy::Disabled
        } else {
            HookPolicy::Enabled
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::registry()
        .with(
            cli.explicit_log_filter()
                .map(tracing_subscriber::EnvFilter::new)
                .unwrap_or_else(|| {
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
                }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    std::env::set_var("WAYTORANDR_DAEMON_MODE", "1");

    let mut backend = connect_backend()?;
    let mut capabilities = backend.capabilities();
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;
    let mut watcher = backend.watch_outputs()?;

    workflow::record_daemon_started_in_store(&state_store, capabilities.backend)?;

    if let Err(err) =
        daemon::enforce_topology_policy(backend.as_ref(), &store, &state_store, cli.hook_policy())
    {
        tracing::error!(error = %escape_terminal_text(err.to_string()), "failed to apply matching profile");
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
                if let Err(err) = daemon::enforce_topology_policy(
                    backend.as_ref(),
                    &store,
                    &state_store,
                    cli.hook_policy(),
                ) {
                    tracing::error!(error = %escape_terminal_text(err.to_string()), "failed to apply matching profile");
                }
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(error = %escape_terminal_text(err.to_string()), "output watcher failed; reconnecting backend");
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
                                cli.hook_policy(),
                            ) {
                                tracing::error!(error = %escape_terminal_text(err.to_string()), "failed to apply matching profile after reconnect");
                            }
                            break;
                        }
                        Err(err) => {
                            tracing::warn!(error = %escape_terminal_text(err.to_string()), "backend reconnect failed");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
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

    #[test]
    fn daemon_cli_defaults_to_info_logging() {
        let cli = Cli::parse_from(["waytorandrd"]);

        assert_eq!(cli.log_filter(), "info");
    }

    #[test]
    fn daemon_cli_accepts_debug_log_level() {
        let cli = Cli::parse_from(["waytorandrd", "--log-level", "debug"]);

        assert_eq!(cli.log_filter(), "debug");
    }

    #[test]
    fn daemon_cli_verbose_enables_debug_logging() {
        let cli = Cli::parse_from(["waytorandrd", "--verbose"]);

        assert_eq!(cli.log_filter(), "debug");
    }

    #[test]
    fn daemon_cli_accepts_no_hooks() {
        let cli = Cli::parse_from(["waytorandrd", "--no-hooks"]);

        assert_eq!(cli.hook_policy(), HookPolicy::Disabled);
    }
}
