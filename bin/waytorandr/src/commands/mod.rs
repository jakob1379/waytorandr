use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

mod apply;
mod output;
mod read;
mod service;
mod shared;
mod write;

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy)]
pub(super) enum OutputMode {
    Text,
    Json,
}

impl OutputMode {
    fn from_json(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Text
        }
    }

    fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

pub(crate) fn run() -> Result<()> {
    let cli = Cli::parse();
    let output_mode = OutputMode::from_json(cli.json);

    match cli.command {
        Commands::Set(args) => write::cmd_set(
            args.target.as_deref(),
            args.dry_run,
            args.make_default,
            args.make_global_default,
            args.reverse,
            args.largest,
            output_mode,
        ),
        Commands::Save(args) => write::cmd_save(
            &args.name,
            args.setup_name.as_deref(),
            args.dry_run,
            args.make_default,
            args.make_global_default,
            output_mode,
        ),
        Commands::Remove(args) => write::cmd_remove(&args.name, args.dry_run, output_mode),
        Commands::Cycle(args) => write::cmd_cycle(args.dry_run, output_mode),
        Commands::Status(args) => read::cmd_status(args.all, output_mode),
        Commands::Version => read::cmd_version(output_mode),
        Commands::Service(args) => service::run(args.command, cli.json),
    }
}

pub(super) fn version_text() -> String {
    format!("{APP_NAME} {APP_VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text_includes_name_and_version() {
        assert_eq!(version_text(), format!("{APP_NAME} {APP_VERSION}"));
    }
}
