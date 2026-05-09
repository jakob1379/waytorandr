mod cli;
mod commands;
mod completion;
mod preset;

use clap::CommandFactory;
use clap_complete::env::CompleteEnv;
use std::process::ExitCode;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use waytorandr_core::terminal::escape_terminal_text;

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
            eprintln!("Error: {}", escape_terminal_text(format!("{err:#}")));
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
