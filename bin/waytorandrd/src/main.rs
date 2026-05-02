// The daemon binary pulls transitive platform crates through independent upstream stacks.
#![allow(clippy::multiple_crate_versions)]

use anyhow::Result;
use clap::Parser;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use waytorandr_backend_loader::connect_backend;
use waytorandr_core::ProfileStore;
use waytorandr_core::StateStore;

mod daemon;

const WATCHER_RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Parser)]
#[command(name = "waytorandrd")]
#[command(about = "Daemon for automatically applying waytorandr display profiles")]
#[command(version)]
struct Cli {
    #[arg(short = 'v', long = "verbose", help = "Enable debug logging")]
    verbose: bool,

    #[arg(long = "no-hooks", help = "Disable profile hook execution")]
    no_hooks: bool,
}

impl Cli {
    fn log_filter(&self) -> &'static str {
        if self.verbose {
            "debug"
        } else {
            "info"
        }
    }

    fn explicit_log_filter(&self) -> Option<&'static str> {
        if self.verbose {
            Some(self.log_filter())
        } else {
            None
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::registry()
        .with(cli.explicit_log_filter().map_or_else(
            || {
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
            },
            tracing_subscriber::EnvFilter::new,
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut backend = connect_backend()?;
    let store = ProfileStore::bootstrap()?;
    let state_store = StateStore::bootstrap()?;

    daemon::run_watch_loop(
        &mut backend,
        &store,
        &state_store,
        WATCHER_RECONNECT_INTERVAL,
        cli.no_hooks,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use waytorandr_core::State;

    #[test]
    fn record_daemon_start_marks_backend_and_enablement() {
        let mut state = State::default();
        state.record_daemon_started(waytorandr_core::BackendKind::Wlroots);

        assert!(state.daemon_enabled);
        assert_eq!(state.backend, Some(waytorandr_core::BackendKind::Wlroots));
    }

    #[test]
    fn daemon_cli_defaults_to_info_logging() {
        let cli = Cli::parse_from(["waytorandrd"]);

        assert_eq!(cli.log_filter(), "info");
    }

    #[test]
    fn daemon_cli_rejects_log_level_option() {
        let error = match Cli::try_parse_from(["waytorandrd", "--log-level", "debug"]) {
            Ok(_) => panic!("--log-level should not be supported"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn daemon_cli_verbose_enables_debug_logging() {
        let cli = Cli::parse_from(["waytorandrd", "--verbose"]);

        assert_eq!(cli.log_filter(), "debug");
    }
}
