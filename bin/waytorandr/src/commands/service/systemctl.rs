use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::{Command, ExitStatus, Output};

use super::unit::unit_path;
use super::UNIT_NAME;

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub(super) struct ServiceStatus {
    pub(super) installed: bool,
    pub(super) unit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unit_file_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) active_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sub_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) fragment_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) load_state: Option<String>,
}

pub(super) fn run_systemctl(args: &[&str]) -> Result<Output> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .with_context(|| format!("failed to run systemctl --user {}", args.join(" ")))?;
    ensure_output_success(&output, &format!("systemctl --user {}", args.join(" ")))?;
    Ok(output)
}

pub(super) fn ensure_output_success(output: &Output, command: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if stderr.is_empty() { stdout } else { stderr };
    if message.is_empty() {
        bail!("{command} failed with status {}", output.status);
    }
    bail!("{command} failed with status {}: {message}", output.status)
}

pub(super) fn ensure_success(status: ExitStatus, command: &str) -> Result<()> {
    if status.success() {
        return Ok(());
    }

    match status.code() {
        Some(code) => bail!("{command} exited with status {code}"),
        None => bail!("{command} terminated by signal"),
    }
}

pub(super) fn read_status() -> Result<ServiceStatus> {
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

pub(super) fn failed_show_means_not_installed(status: &ServiceStatus) -> bool {
    !status.installed && status.load_state.as_deref() == Some("not-found")
}

pub(super) fn parse_systemctl_show(text: &str) -> ServiceStatus {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn failed_output(stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(7 << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
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
    fn systemctl_failure_reports_command_status_and_stderr() {
        let output = failed_output("ignored stdout", "unit failed");

        let err = ensure_output_success(&output, "systemctl --user restart waytorandrd.service")
            .expect_err("nonzero status should fail");

        assert_eq!(
            err.to_string(),
            "systemctl --user restart waytorandrd.service failed with status exit status: 7: unit failed"
        );
    }

    #[test]
    fn systemctl_failure_reports_command_status_and_stdout_when_stderr_is_empty() {
        let output = failed_output("stdout failure", "");

        let err = ensure_output_success(&output, "systemctl --user status waytorandrd.service")
            .expect_err("nonzero status should fail");

        assert_eq!(
            err.to_string(),
            "systemctl --user status waytorandrd.service failed with status exit status: 7: stdout failure"
        );
    }

    #[test]
    fn systemctl_failure_reports_command_status_without_output() {
        let output = failed_output("", "");

        let err = ensure_output_success(&output, "systemctl --user stop waytorandrd.service")
            .expect_err("nonzero status should fail");

        assert_eq!(
            err.to_string(),
            "systemctl --user stop waytorandrd.service failed with status exit status: 7"
        );
    }
}
