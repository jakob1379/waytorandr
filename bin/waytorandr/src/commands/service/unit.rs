use anyhow::{anyhow, bail, Result};
use directories::BaseDirs;
use std::path::{Path, PathBuf};

use super::{DOCS_URL, INSTALL_TARGET, UNIT_NAME};

pub(super) fn unit_path() -> Result<PathBuf> {
    let config_dir = BaseDirs::new()
        .ok_or_else(|| anyhow!("unable to resolve XDG config directory"))?
        .config_dir()
        .to_path_buf();
    Ok(config_dir.join("systemd").join("user").join(UNIT_NAME))
}

pub(super) fn daemon_binary_path() -> Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    resolve_daemon_binary_from_current_exe(current_exe.as_path())
}

pub(super) fn resolve_daemon_binary_from_current_exe(current_exe: &Path) -> Result<PathBuf> {
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

pub(super) fn render_unit(daemon_path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_unit_uses_expected_defaults() {
        let unit = render_unit(Path::new("/tmp/waytorandrd"));

        assert!(unit.contains("ConditionEnvironment=WAYLAND_DISPLAY"));
        assert!(unit.contains(&format!("Documentation={}", env!("CARGO_PKG_REPOSITORY"))));
        assert!(unit.contains("ExecStart=\"/tmp/waytorandrd\""));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn render_unit_quotes_systemd_values() {
        let unit = render_unit(Path::new("/tmp/bin/waytorandrd \"dev\""));

        assert!(unit.contains("ExecStart=\"/tmp/bin/waytorandrd \\\"dev\\\"\""));
    }

    #[test]
    fn resolve_daemon_binary_requires_sibling_binary() {
        let err = resolve_daemon_binary_from_current_exe(&PathBuf::from("/tmp/waytorandr"))
            .expect_err("missing sibling should fail");

        assert!(err
            .to_string()
            .contains("could not find a sibling 'waytorandrd' binary"));
    }
}
