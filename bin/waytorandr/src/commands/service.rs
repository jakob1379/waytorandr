use anyhow::{anyhow, bail, Context, Result};
use directories::BaseDirs;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};

use super::output::write_json;
use crate::cli::ServiceCommands;

const UNIT_NAME: &str = "waytorandrd.service";
const DOCS_URL: &str = "https://github.com/jsg/waytorandr";
const INSTALL_TARGET: &str = "default.target";

#[derive(Debug, Serialize, Default, PartialEq, Eq)]
struct ServiceStatus {
    installed: bool,
    unit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit_file_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fragment_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_state: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsonServiceActionResponse {
    command: &'static str,
    unit: &'static str,
    installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit_file_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub_state: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsonServiceStatusResponse {
    command: &'static str,
    unit: &'static str,
    installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit_file_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fragment_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_state: Option<String>,
}

pub(super) fn run(command: ServiceCommands, json: bool) -> Result<()> {
    match command {
        ServiceCommands::Install => cmd_install(json),
        ServiceCommands::Uninstall => cmd_uninstall(json),
        ServiceCommands::Start => cmd_systemctl("start", json),
        ServiceCommands::Stop => cmd_systemctl("stop", json),
        ServiceCommands::Restart => cmd_systemctl("restart", json),
        ServiceCommands::Status => cmd_status(json),
        ServiceCommands::Run => cmd_run(json),
    }
}

fn cmd_install(json: bool) -> Result<()> {
    let unit_path = unit_path()?;
    let unit_dir = unit_path
        .parent()
        .ok_or_else(|| anyhow!("missing parent directory for user service unit"))?;
    fs::create_dir_all(unit_dir)
        .with_context(|| format!("failed to create {}", unit_dir.display()))?;

    let daemon_path = daemon_binary_path()?;
    fs::write(&unit_path, render_unit(&daemon_path))
        .with_context(|| format!("failed to write {}", unit_path.display()))?;

    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", UNIT_NAME])?;

    if json {
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

    println!("Installed user service '{UNIT_NAME}'");
    println!("Unit file: {}", unit_path.display());
    println!("Enabled for '{INSTALL_TARGET}'");
    Ok(())
}

fn cmd_uninstall(json: bool) -> Result<()> {
    let path = unit_path()?;
    let was_installed = path.exists();
    if was_installed {
        let _ = run_systemctl(&["disable", "--now", UNIT_NAME]);
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
        run_systemctl(&["daemon-reload"])?;
    }

    if json {
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
        println!("Uninstalled user service '{UNIT_NAME}'");
    } else {
        println!("User service '{UNIT_NAME}' is not installed");
    }
    Ok(())
}

fn cmd_systemctl(action: &'static str, json: bool) -> Result<()> {
    run_systemctl(&[action, UNIT_NAME])?;
    let status = read_status()?;

    if json {
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

    println!("{} user service '{}'", capitalize(action), UNIT_NAME);
    print_status_summary(&status);
    Ok(())
}

fn cmd_status(json: bool) -> Result<()> {
    let status = read_status()?;
    if json {
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

fn cmd_run(json: bool) -> Result<()> {
    if json {
        bail!("--json is not supported with `waytorandr service run`");
    }

    let daemon_path = daemon_binary_path()?;
    let status = Command::new(&daemon_path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch {}", daemon_path.display()))?;

    ensure_success(status, "waytorandrd")?;
    Ok(())
}

fn unit_path() -> Result<PathBuf> {
    let config_dir = BaseDirs::new()
        .ok_or_else(|| anyhow!("unable to resolve XDG config directory"))?
        .config_dir()
        .to_path_buf();
    Ok(config_dir.join("systemd").join("user").join(UNIT_NAME))
}

fn daemon_binary_path() -> Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    resolve_daemon_binary_from_current_exe(current_exe.as_path())
}

fn resolve_daemon_binary_from_current_exe(current_exe: &Path) -> Result<PathBuf> {
    let current_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow!("unable to determine current binary directory"))?;
    let candidate = current_dir.join("waytorandrd");
    if candidate.exists() {
        return Ok(candidate);
    }

    bail!(
        "could not find a sibling 'waytorandrd' binary next to '{}'",
        current_exe.display()
    )
}

fn render_unit(daemon_path: &Path) -> String {
    format!(
        "[Unit]\nDescription=Wayland display profile daemon\nDocumentation={DOCS_URL}\nConditionEnvironment=WAYLAND_DISPLAY\n\n[Service]\nType=simple\nExecStart={}\nRestart=always\nSlice=background.slice\n\n[Install]\nWantedBy={INSTALL_TARGET}\n",
        quote_systemd_value(daemon_path.as_os_str())
    )
}

fn quote_systemd_value(value: &std::ffi::OsStr) -> String {
    let value = value.to_string_lossy();
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn run_systemctl(args: &[&str]) -> Result<Output> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .with_context(|| format!("failed to run systemctl --user {}", args.join(" ")))?;
    ensure_output_success(&output, &format!("systemctl --user {}", args.join(" ")))?;
    Ok(output)
}

fn ensure_output_success(output: &Output, command: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if stderr.is_empty() { stdout } else { stderr };
    if message.is_empty() {
        bail!("{command} failed with status {}", output.status);
    }
    bail!("{message}")
}

fn ensure_success(status: ExitStatus, command: &str) -> Result<()> {
    if status.success() {
        return Ok(());
    }

    match status.code() {
        Some(code) => bail!("{command} exited with status {code}"),
        None => bail!("{command} terminated by signal"),
    }
}

fn read_status() -> Result<ServiceStatus> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args([
            "show",
            "--property=LoadState,UnitFileState,ActiveState,SubState,FragmentPath",
            UNIT_NAME,
        ])
        .output()
        .with_context(|| format!("failed to run systemctl --user show {UNIT_NAME}"))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut status = parse_systemctl_show(&text);
    status.unit = UNIT_NAME;
    status.installed = unit_path()?.exists()
        || status
            .load_state
            .as_deref()
            .is_some_and(|state| state != "not-found");

    if !output.status.success() && failed_show_means_not_installed(&status) {
        return Ok(status);
    }

    ensure_output_success(&output, &format!("systemctl --user show {UNIT_NAME}"))?;
    Ok(status)
}

fn failed_show_means_not_installed(status: &ServiceStatus) -> bool {
    !status.installed && status.load_state.as_deref() == Some("not-found")
}

fn parse_systemctl_show(text: &str) -> ServiceStatus {
    let mut status = ServiceStatus {
        installed: false,
        unit: UNIT_NAME,
        ..ServiceStatus::default()
    };

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        };
        match key {
            "LoadState" => status.load_state = value,
            "UnitFileState" => status.unit_file_state = value,
            "ActiveState" => status.active_state = value,
            "SubState" => status.sub_state = value,
            "FragmentPath" => status.fragment_path = value,
            _ => {}
        }
    }

    status
}

fn print_status_summary(status: &ServiceStatus) {
    println!("Service: {unit}", unit = status.unit);
    println!("Installed: {}", yes_no(status.installed));
    if let Some(state) = &status.unit_file_state {
        println!("Enabled: {state}");
    }
    if let Some(active) = &status.active_state {
        if let Some(sub) = &status.sub_state {
            println!("Active: {active} ({sub})");
        } else {
            println!("Active: {active}");
        }
    }
    if let Some(path) = &status.fragment_path {
        println!("Unit file: {path}");
    }
}

const fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
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
mod tests {
    use super::*;

    #[test]
    fn render_unit_uses_expected_defaults() {
        let unit = render_unit(Path::new("/tmp/waytorandrd"));

        assert!(unit.contains("ConditionEnvironment=WAYLAND_DISPLAY"));
        assert!(unit.contains("ExecStart=\"/tmp/waytorandrd\""));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn parse_systemctl_show_extracts_known_fields() {
        let status = parse_systemctl_show(
            "LoadState=loaded\nUnitFileState=enabled\nActiveState=active\nSubState=running\nFragmentPath=/tmp/waytorandrd.service\n",
        );

        assert_eq!(status.load_state.as_deref(), Some("loaded"));
        assert_eq!(status.unit_file_state.as_deref(), Some("enabled"));
        assert_eq!(status.active_state.as_deref(), Some("active"));
        assert_eq!(status.sub_state.as_deref(), Some("running"));
        assert_eq!(
            status.fragment_path.as_deref(),
            Some("/tmp/waytorandrd.service")
        );
    }

    #[test]
    fn failed_show_means_not_installed_only_for_not_found_units() {
        let not_found = ServiceStatus {
            installed: false,
            load_state: Some("not-found".to_string()),
            ..ServiceStatus::default()
        };
        let broken_systemd = ServiceStatus {
            installed: false,
            load_state: None,
            ..ServiceStatus::default()
        };

        assert!(failed_show_means_not_installed(&not_found));
        assert!(!failed_show_means_not_installed(&broken_systemd));
    }

    #[test]
    fn resolve_daemon_binary_requires_sibling_binary() {
        let err = resolve_daemon_binary_from_current_exe(&PathBuf::from("/tmp/waytorandr"))
            .expect_err("missing sibling should fail");

        assert!(err
            .to_string()
            .contains("could not find a sibling 'waytorandrd' binary"));
    }

    #[test]
    fn run_rejects_json_output() {
        let err = cmd_run(true).expect_err("json should be rejected for service run");

        assert!(err
            .to_string()
            .contains("--json is not supported with `waytorandr service run`"));
    }
}
