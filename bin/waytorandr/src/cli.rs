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
    #[command(about = "Set a saved target, virtual configuration, or `auto` selection")]
    #[command(after_long_help = "Virtual configurations:
  auto       Apply the setup default, best matching saved profile, or new-setup default
  off        Disable external outputs and keep built-in panels on when present
  external   Prefer external outputs; if none are present, keep built-in panels enabled
  common     Clone all connected outputs at the largest common resolution (not native mirroring)
  largest    Clone all connected outputs at the same origin using each output's largest mode
  mirror     Mirror all connected outputs on backends with native mirroring support
  horizontal Extend all connected outputs horizontally
  vertical   Extend all connected outputs vertically

`waytorandr set auto` first applies the configured default for the current hardware setup.
If no setup default is configured, it applies the best matching saved profile.
If there is no match, it applies the configured default for new setups.

Examples:
  waytorandr set auto
  waytorandr set docked
  waytorandr set docked --default
  waytorandr set --profile auto
  waytorandr set vertical --save
  waytorandr set vertical --default
  waytorandr set external --global-default
  waytorandr set common --dry-run
  waytorandr set largest --dry-run
  waytorandr set mirror --dry-run
  waytorandr set vertical --reverse --dry-run

`--default` only affects the current setup fingerprint.
With virtual configurations, `--default` saves the resulting layout as profile `default`
and makes that saved profile the default for the current setup.
Use `--global-default` with a virtual configuration to set the fallback target for new setups.
Use `--save` with a virtual configuration as a shortcut for saving the resulting layout
as profile `default` and making it the default for the current setup.
Use `--profile` when a saved profile name collides with `auto` or a virtual target.

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
Use `--default` together with `save` when you want this saved layout to become the default profile for the current setup fingerprint.")]
    Save(SaveArgs),

    #[command(about = "Remove a saved profile for the current setup")]
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
        required_unless_present = "profile",
        conflicts_with = "profile",
        help = "Saved target, virtual configuration, or `auto`",
        add = ArgValueCompleter::new(complete_set_targets)
    )]
    pub(crate) target: Option<String>,

    #[arg(
        long = "profile",
        value_name = "profile",
        required_unless_present = "target",
        conflicts_with = "target",
        help = "Force a saved profile by name when it collides with `auto` or a virtual target",
        add = ArgValueCompleter::new(complete_saved_profiles)
    )]
    pub(crate) profile: Option<String>,

    #[arg(
        short = 'n',
        long = "dry-run",
        help = "Preview without applying the layout"
    )]
    pub(crate) dry_run: bool,

    #[arg(
        short = 'd',
        long = "default",
        help = "Make the resulting layout the default for the current setup fingerprint"
    )]
    pub(crate) make_default: bool,

    #[arg(
        long = "global-default",
        help = "With virtual configurations: set the fallback target for new setups"
    )]
    pub(crate) global_default: bool,

    #[arg(
        short = 's',
        long = "save",
        help = "With virtual configurations: save the resulting layout as profile `default` and make it the default for the current setup"
    )]
    pub(crate) save: bool,

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
        help = "Also set the saved profile as the default profile for the current setup fingerprint"
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
        help = "Profile name to remove from the current setup",
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
    fn remove_help_mentions_current_setup_scope() {
        let mut command = Cli::command();
        let remove = command
            .find_subcommand_mut("remove")
            .expect("remove subcommand should exist");

        assert_eq!(
            remove.get_about().map(|value| value.to_string()),
            Some("Remove a saved profile for the current setup".to_string())
        );

        let profile_arg = remove
            .get_arguments()
            .find(|arg| arg.get_id().as_str() == "name")
            .expect("remove profile arg should exist");
        assert_eq!(
            profile_arg.get_help().map(|value| value.to_string()),
            Some("Profile name to remove from the current setup".to_string())
        );
    }

    #[test]
    fn set_requires_explicit_target() {
        let error = match Cli::try_parse_from(["waytorandr", "set"]) {
            Ok(_) => panic!("set without target should be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn set_help_mentions_auto_target() {
        let mut command = Cli::command();
        let set = command
            .find_subcommand_mut("set")
            .expect("set subcommand should exist");

        assert_eq!(
            set.get_about().map(|value| value.to_string()),
            Some("Set a saved target, virtual configuration, or `auto` selection".to_string())
        );

        let target_arg = set
            .get_arguments()
            .find(|arg| arg.get_id().as_str() == "target")
            .expect("set target arg should exist");
        assert_eq!(
            target_arg.get_help().map(|value| value.to_string()),
            Some("Saved target, virtual configuration, or `auto`".to_string())
        );

        let profile_arg = set
            .get_arguments()
            .find(|arg| arg.get_id().as_str() == "profile")
            .expect("set profile arg should exist");
        assert_eq!(
            profile_arg.get_help().map(|value| value.to_string()),
            Some(
                "Force a saved profile by name when it collides with `auto` or a virtual target"
                    .to_string()
            )
        );
    }
}
