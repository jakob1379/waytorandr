// The CLI binary pulls transitive platform crates through independent upstream stacks.
#![allow(clippy::multiple_crate_versions)]

mod cli;
mod commands;
mod completion;
mod preset;
#[cfg(test)]
mod test_support;

use clap::CommandFactory;
use clap_complete::env::CompleteEnv;
use std::process::ExitCode;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> ExitCode {
    CompleteEnv::with_factory(cli::Cli::command).complete();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    match commands::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::FAILURE
        }
    }
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
