use clap::{Args, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;

use crate::completion::{complete_saved_profiles, complete_set_targets};

#[derive(Parser)]
#[command(name = "waytorandr")]
#[command(about = "Wayland-native display profile manager")]
#[command(long_about = "Save, set, and switch Wayland display layouts.")]
#[command(subcommand_required = true)]
#[command(arg_required_else_help = true)]
#[command(
    after_long_help = "Run `waytorandr set --help` or `waytorandr save --help` for command-specific examples."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    #[command(about = "Set a saved profile, virtual configuration, or default/matching profile")]
    #[command(after_long_help = "Virtual configurations:
  off        Disable all outputs
  common     Place all connected outputs at a common resolution on the same origin
  mirror     Reserved name; prints guidance to use wl-mirror for real mirroring
  horizontal Extend all connected outputs horizontally
  vertical   Extend all connected outputs vertically

When [profile] is omitted, `set` first applies the configured default for the current hardware setup.
If no setup default is configured, it applies the best matching saved profile.

Examples:
  waytorandr set
  waytorandr set docked
  waytorandr set docked --default
  waytorandr set common --dry-run
  waytorandr set common --largest --dry-run
  waytorandr set vertical --reverse --dry-run

For true mirroring, use `wl-mirror` until output-management protocols grow real mirroring support.")]
    Set(SetArgs),

    #[command(about = "Save the current compositor layout as a profile")]
    #[command(after_long_help = "Examples:
  waytorandr save
  waytorandr save docked
  waytorandr save --default
  waytorandr save docked --default
  waytorandr save docked --dry-run

If the profile name is omitted, `default` is used.
Use `--default` together with `save` when the current screen setup may match multiple saved profiles and you want this saved layout to become the preferred default.")]
    Save(SaveArgs),

    #[command(about = "Remove a saved profile")]
    Remove(RemoveArgs),

    #[command(about = "Set the next saved profile")]
    Cycle(MutatingArgs),

    #[command(about = "List profiles matching the current topology by default")]
    #[command(after_long_help = "Examples:
  waytorandr list
  waytorandr list --all

By default, `list` only shows profiles matching the current detected topology.
Use `--all` to show every saved profile across all setups, grouped by setup fingerprint.")]
    List(ListArgs),

    #[command(about = "Show the active or currently matched profile")]
    Current,

    #[command(about = "Show detected outputs and current geometry")]
    Detected,
}

#[derive(Args)]
pub(crate) struct SetArgs {
    #[arg(
        value_name = "profile",
        help = "Saved profile or virtual configuration; omit to set setup default or best match",
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
        help = "Only with saved profiles: also set the profile as the default for this hardware setup"
    )]
    pub(crate) make_default: bool,

    #[arg(
        short = 'l',
        long = "largest",
        help = "Only with `common`: use the largest available shared target mode"
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
pub(crate) struct SaveArgs {
    #[arg(
        value_name = "profile",
        default_value = "default",
        help = "Profile name to save; defaults to `default`"
    )]
    pub(crate) name: String,

    #[arg(
        short = 'd',
        long = "default",
        help = "Also set the saved profile as the default profile"
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
pub(crate) struct RemoveArgs {
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
pub(crate) struct MutatingArgs {
    #[arg(
        short = 'n',
        long = "dry-run",
        help = "Preview without applying changes"
    )]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct ListArgs {
    #[arg(
        short = 'a',
        long = "all",
        help = "List all saved profiles, not just profiles matching the current topology"
    )]
    pub(crate) all: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
