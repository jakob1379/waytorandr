mod cli;
mod commands;
mod completion;
mod output;
mod preset;

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::env::CompleteEnv;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> Result<()> {
    CompleteEnv::with_factory(cli::Cli::command).complete();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    commands::run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn main_exposes_a_valid_cli_definition() {
        cli::Cli::command().debug_assert();
    }
}
