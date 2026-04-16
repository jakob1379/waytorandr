use clap::{Args, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;

use crate::completion::{complete_saved_profiles, complete_set_targets};

#[derive(Parser)]
#[command(name = "waytorandr")]
#[command(about = "Wayland-native display profile manager")]
#[command(long_about = "Save, set, and switch Wayland display layouts.")]
#[command(version)]
#[command(subcommand_required = true)]
#[command(arg_required_else_help = true)]
#[command(
    after_long_help = "Run `waytorandr set --help`, `waytorandr save --help`, or `waytorandr status --help` for command-specific examples."
)]
pub struct Cli {
    #[arg(
        long = "json",
        global = true,
        help = "Emit command output as JSON on stdout"
    )]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Set a saved profile, virtual configuration, or default/matching profile")]
    #[command(after_long_help = "Virtual configurations:
  off        Disable external outputs and keep built-in panels on when present
  external   Disable built-in panels and keep external outputs on when present
  common     Clone all connected outputs at the largest common resolution (not native mirroring)
  largest    Clone all connected outputs at the same origin using each output's largest mode
  mirror     Mirror all connected outputs on backends with native mirroring support
  horizontal Extend all connected outputs horizontally
  vertical   Extend all connected outputs vertically

When [target] is omitted, `set` first applies the configured default for the current hardware setup.
If no setup default is configured, it applies the best matching saved profile.
If there is no match, it applies the configured default for new setups.

Examples:
  waytorandr set
  waytorandr set docked
  waytorandr set docked --default
  waytorandr set external --default
  waytorandr set common --dry-run
  waytorandr set largest --dry-run
  waytorandr set mirror --dry-run
  waytorandr set vertical --reverse --dry-run

When a backend cannot support `mirror`, waytorandr prints backend-specific guidance instead of sending an invalid layout.")]
    Set(SetArgs),

    #[command(about = "Save the current compositor layout as a profile")]
    #[command(after_long_help = "Examples:
  waytorandr save
  waytorandr save docked
  waytorandr save docked --setup-name office
  waytorandr save --default
  waytorandr save docked --default
  waytorandr save docked --dry-run

If the profile name is omitted, `default` is used.
Use `--setup-name` to assign a friendly name to the current setup while keeping fingerprint-based matching.
Use `--default` together with `save` when the current screen setup may match multiple saved profiles and you want this saved layout to become the default profile for this setup.")]
    Save(SaveArgs),

    #[command(about = "Remove a saved profile")]
    Remove(RemoveArgs),

    #[command(about = "Set the next saved profile")]
    Cycle(MutatingArgs),

    #[command(about = "Show the current layout state and related saved profiles")]
    #[command(after_long_help = "Examples:
  waytorandr status
  waytorandr status --all

By default, `status` shows the current matched profile, the detected outputs,
and saved profiles related to the current detected topology.
Use `--all` to include every saved profile across all setups, grouped by setup fingerprint and optional setup name.")]
    Status(StatusArgs),

    #[command(about = "Show the waytorandr version")]
    Version,

    #[command(about = "Manage the waytorandrd user service")]
    #[command(after_long_help = "Examples:
  waytorandr service install
  waytorandr service start
  waytorandr service status
  waytorandr service uninstall")]
    Service(ServiceArgs),
}

#[derive(Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub(crate) command: ServiceCommands,
}

#[derive(Subcommand, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceCommands {
    #[command(about = "Install the waytorandrd user service")]
    Install,

    #[command(about = "Uninstall the waytorandrd user service")]
    Uninstall,

    #[command(about = "Start the waytorandrd user service")]
    Start,

    #[command(about = "Stop the waytorandrd user service")]
    Stop,

    #[command(about = "Restart the waytorandrd user service")]
    Restart,

    #[command(about = "Show the waytorandrd user service status")]
    Status,

    #[command(about = "Run waytorandrd in the foreground")]
    #[command(
        help_template = "Run waytorandrd in the foreground\n\nUsage: {usage}\n\nThis command does not support --json.\n\nOptions:\n  -h, --help  Print help\n"
    )]
    Run,
}

#[derive(Args)]
pub struct SetArgs {
    #[arg(
        value_name = "target",
        help = "Saved target or virtual preset; omit to apply an auto-selected target or use a default/virtual preset",
        add = ArgValueCompleter::new(complete_set_targets)
    )]
    pub(crate) target: Option<String>,

    #[arg(
        short = 'n',
        long = "dry-run",
        help = "Preview without applying the layout"
    )]
    pub(crate) dry_run: bool,

    #[arg(
        short = 'd',
        long = "default",
        help = "With saved profiles: set the default for this setup. With virtual configurations: set the default for new setups"
    )]
    pub(crate) make_default: bool,

    #[arg(
        short = 'l',
        long = "largest",
        hide = true,
        help = "Deprecated compatibility alias for `waytorandr set largest`"
    )]
    pub(crate) largest: bool,

    #[arg(
        short = 'r',
        long = "reverse",
        help = "Only with `horizontal` or `vertical`: reverse ordering"
    )]
    pub(crate) reverse: bool,
}

#[derive(Args)]
pub struct SaveArgs {
    #[arg(
        value_name = "profile",
        default_value = "default",
        help = "Profile name to save; defaults to `default`"
    )]
    pub(crate) name: String,

    #[arg(
        short = 's',
        long = "setup-name",
        value_name = "name",
        help = "Optional friendly name for the current setup"
    )]
    pub(crate) setup_name: Option<String>,

    #[arg(
        short = 'd',
        long = "default",
        help = "Also set the saved profile as the default profile for this setup"
    )]
    pub(crate) make_default: bool,

    #[arg(
        short = 'n',
        long = "dry-run",
        help = "Preview the profile that would be saved"
    )]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub struct RemoveArgs {
    #[arg(
        value_name = "profile",
        help = "Profile name to remove",
        add = ArgValueCompleter::new(complete_saved_profiles)
    )]
    pub(crate) name: String,

    #[arg(
        short = 'n',
        long = "dry-run",
        help = "Preview without removing the profile"
    )]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub struct MutatingArgs {
    #[arg(
        short = 'n',
        long = "dry-run",
        help = "Preview without applying changes"
    )]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub struct StatusArgs {
    #[arg(
        short = 'a',
        long = "all",
        help = "Show all saved profiles, not just profiles matching the current topology"
    )]
    pub(crate) all: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use clap::CommandFactory;
    use clap::Parser;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_json_flag_parses_before_subcommand() {
        let cli = Cli::parse_from(["waytorandr", "--json", "status"]);
        assert!(cli.json);
    }

    #[test]
    fn global_json_flag_parses_after_subcommand() {
        let cli = Cli::parse_from(["waytorandr", "status", "--json"]);
        assert!(cli.json);
    }

    #[test]
    fn version_subcommand_parses() {
        let cli = Cli::parse_from(["waytorandr", "version"]);
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn removed_read_subcommands_are_rejected() {
        for command in ["list", "current", "detected"] {
            let error = match Cli::try_parse_from(["waytorandr", command]) {
                Ok(_) => panic!("{command} should be rejected"),
                Err(error) => error,
            };
            assert_eq!(error.kind(), ErrorKind::InvalidSubcommand);
        }
    }

    #[test]
    fn save_parses_setup_name_short_flag() {
        let cli = Cli::parse_from(["waytorandr", "save", "desk", "-s", "office"]);

        match cli.command {
            Commands::Save(args) => assert_eq!(args.setup_name.as_deref(), Some("office")),
            _ => panic!("expected save command"),
        }
    }

    #[test]
    fn version_flag_is_supported() {
        let err = Cli::try_parse_from(["waytorandr", "--version"])
            .err()
            .expect("--version should trigger clap version output");

        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn service_run_help_hides_json_flag() {
        let mut cli = Cli::command();
        let service = cli
            .find_subcommand_mut("service")
            .expect("service subcommand");
        let mut run = service
            .find_subcommand_mut("run")
            .expect("service run subcommand")
            .clone();
        let mut help = Vec::new();
        run.write_long_help(&mut help).expect("write help");
        let help = String::from_utf8(help).expect("utf8 help");

        assert!(help.contains("does not support --json"));
        assert!(!help.contains("Emit command output as JSON on stdout"));
    }
}
