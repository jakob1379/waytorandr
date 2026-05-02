use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};

use super::output::{heading, success, value, warning, write_json};
use super::OutputMode;
use crate::cli::ServiceCommands;

mod output;
mod systemctl;
mod unit;

use output::{print_status_summary, JsonServiceActionResponse, JsonServiceStatusResponse};
use systemctl::{ensure_success, read_status, run_systemctl, ServiceStatus};
use unit::{daemon_binary_path, render_unit, unit_path};

const UNIT_NAME: &str = "waytorandrd.service";
const DOCS_URL: &str = env!("CARGO_PKG_REPOSITORY");
const INSTALL_TARGET: &str = "default.target";

pub(super) fn run(command: ServiceCommands, output_mode: OutputMode) -> Result<()> {
    match command {
        ServiceCommands::Install => cmd_install(output_mode),
        ServiceCommands::Uninstall => cmd_uninstall(output_mode),
        ServiceCommands::Start => cmd_systemctl("start", output_mode),
        ServiceCommands::Stop => cmd_systemctl("stop", output_mode),
        ServiceCommands::Restart => cmd_systemctl("restart", output_mode),
        ServiceCommands::Status => cmd_status(output_mode),
        ServiceCommands::Run(args) => cmd_run(output_mode, args.no_hooks),
    }
}

fn cmd_install(output_mode: OutputMode) -> Result<()> {
    let unit_path = unit_path()?;
    let daemon_path = daemon_binary_path()?;
    cmd_install_with(output_mode, &unit_path, &daemon_path, run_systemctl)
}

fn cmd_install_with<F>(
    output_mode: OutputMode,
    unit_path: &Path,
    daemon_path: &Path,
    run_systemctl: F,
) -> Result<()>
where
    F: Fn(&[&str]) -> Result<Output>,
{
    let unit_dir = unit_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent directory for user service unit"))?;
    fs::create_dir_all(unit_dir)
        .with_context(|| format!("failed to create {}", unit_dir.display()))?;

    fs::write(unit_path, render_unit(daemon_path))
        .with_context(|| format!("failed to write {}", unit_path.display()))?;

    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", UNIT_NAME])?;

    if output_mode.is_json() {
        return write_json(&JsonServiceActionResponse {
            command: "service-install",
            unit: UNIT_NAME,
            installed: true,
            path: Some(unit_path.display().to_string()),
            unit_file_state: Some("enabled".to_string()),
            active_state: None,
            sub_state: None,
        });
    }

    println!(
        "{} {}",
        success("Installed"),
        value(format!("user service '{UNIT_NAME}'"))
    );
    println!(
        "{} {}",
        super::output::key("Unit file"),
        unit_path.display()
    );
    println!(
        "{} {}",
        success("Enabled for"),
        value(format!("'{INSTALL_TARGET}'"))
    );
    Ok(())
}

fn cmd_uninstall(output_mode: OutputMode) -> Result<()> {
    let path = unit_path()?;
    cmd_uninstall_with(output_mode, &path, run_systemctl)
}

fn cmd_uninstall_with<F>(output_mode: OutputMode, path: &Path, run_systemctl: F) -> Result<()>
where
    F: Fn(&[&str]) -> Result<Output>,
{
    let was_installed = path.exists();
    if was_installed {
        run_systemctl(&["disable", "--now", UNIT_NAME])?;
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
        run_systemctl(&["daemon-reload"])?;
    }

    if output_mode.is_json() {
        return write_json(&JsonServiceActionResponse {
            command: "service-uninstall",
            unit: UNIT_NAME,
            installed: false,
            path: Some(path.display().to_string()),
            unit_file_state: Some("disabled".to_string()),
            active_state: None,
            sub_state: None,
        });
    }

    if was_installed {
        println!(
            "{} {}",
            success("Uninstalled"),
            value(format!("user service '{UNIT_NAME}'"))
        );
    } else {
        println!("{}", warning("User service is not installed"));
    }
    Ok(())
}

fn cmd_systemctl(action: &'static str, output_mode: OutputMode) -> Result<()> {
    cmd_systemctl_with(action, output_mode, run_systemctl, read_status)
}

fn cmd_systemctl_with<F, R>(
    action: &'static str,
    output_mode: OutputMode,
    run_systemctl: F,
    read_status: R,
) -> Result<()>
where
    F: Fn(&[&str]) -> Result<Output>,
    R: Fn() -> Result<ServiceStatus>,
{
    run_systemctl(&[action, UNIT_NAME])?;
    let status = read_status()?;

    if output_mode.is_json() {
        return write_json(&JsonServiceActionResponse {
            command: json_command_name(action),
            unit: UNIT_NAME,
            installed: status.installed,
            path: status.fragment_path.clone(),
            unit_file_state: status.unit_file_state.clone(),
            active_state: status.active_state.clone(),
            sub_state: status.sub_state,
        });
    }

    println!(
        "{} {}",
        heading(capitalize(action)),
        value(format!("user service '{UNIT_NAME}'"))
    );
    print_status_summary(&status);
    Ok(())
}

fn cmd_status(output_mode: OutputMode) -> Result<()> {
    cmd_status_with(output_mode, read_status)
}

fn cmd_status_with<R>(output_mode: OutputMode, read_status: R) -> Result<()>
where
    R: Fn() -> Result<ServiceStatus>,
{
    let status = read_status()?;
    if output_mode.is_json() {
        return write_json(&JsonServiceStatusResponse {
            command: "service-status",
            unit: status.unit,
            installed: status.installed,
            unit_file_state: status.unit_file_state,
            active_state: status.active_state,
            sub_state: status.sub_state,
            fragment_path: status.fragment_path,
            load_state: status.load_state,
        });
    }

    print_status_summary(&status);
    Ok(())
}

fn cmd_run(output_mode: OutputMode, no_hooks: bool) -> Result<()> {
    if output_mode.is_json() {
        bail!("--json is not supported with `waytorandr service run`");
    }

    let daemon_path = daemon_binary_path()?;
    cmd_run_with(&daemon_path, no_hooks, run_daemon)
}

fn cmd_run_with<F>(daemon_path: &Path, no_hooks: bool, run_daemon: F) -> Result<()>
where
    F: Fn(&Path, bool) -> Result<ExitStatus>,
{
    let status = run_daemon(daemon_path, no_hooks)?;

    ensure_success(status, "waytorandrd")?;
    Ok(())
}

fn run_daemon(daemon_path: &Path, no_hooks: bool) -> Result<ExitStatus> {
    let mut command = Command::new(daemon_path);
    if no_hooks {
        command.arg("--no-hooks");
    }
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch {}", daemon_path.display()))
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

fn json_command_name(action: &str) -> &'static str {
    match action {
        "start" => "service-start",
        "stop" => "service-stop",
        "restart" => "service-restart",
        _ => "service",
    }
}

#[cfg(test)]
mod tests;
