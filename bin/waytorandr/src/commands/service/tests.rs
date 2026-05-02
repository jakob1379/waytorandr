use super::*;
use anyhow::bail;
use anyhow::Result;
use std::cell::RefCell;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Output};
use std::rc::Rc;

fn successful_output() -> Output {
    Output {
        status: ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

fn status_fixture() -> ServiceStatus {
    ServiceStatus {
        installed: true,
        unit: UNIT_NAME,
        unit_file_state: Some("enabled".to_string()),
        active_state: Some("active".to_string()),
        sub_state: Some("running".to_string()),
        fragment_path: Some("/tmp/waytorandrd.service".to_string()),
        load_state: Some("loaded".to_string()),
    }
}

#[test]
fn run_rejects_json_output() -> Result<()> {
    let Err(err) = cmd_run(OutputMode::Json, false) else {
        bail!("json should be rejected for service run");
    };

    assert!(err
        .to_string()
        .contains("--json is not supported with `waytorandr service run`"));
    Ok(())
}

#[test]
fn install_writes_unit_and_enables_systemd() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let unit_path = temp_dir.path().join("systemd/user").join(UNIT_NAME);
    let daemon_path = PathBuf::from("/tmp/bin/waytorandrd");
    let calls = Rc::new(RefCell::new(Vec::<Vec<String>>::new()));
    let seen_calls = Rc::clone(&calls);

    cmd_install_with(OutputMode::Text, &unit_path, &daemon_path, |args| {
        seen_calls
            .borrow_mut()
            .push(args.iter().map(ToString::to_string).collect());
        Ok(successful_output())
    })?;

    assert_eq!(
        calls.borrow().as_slice(),
        [
            vec!["daemon-reload".to_string()],
            vec!["enable".to_string(), UNIT_NAME.to_string()]
        ]
    );
    let unit = fs::read_to_string(unit_path)?;
    assert!(unit.contains("ExecStart=\"/tmp/bin/waytorandrd\""));
    assert!(unit.contains("WantedBy=default.target"));
    Ok(())
}

#[test]
fn systemctl_action_uses_expected_args_and_status() -> Result<()> {
    let calls = Rc::new(RefCell::new(Vec::<Vec<String>>::new()));
    let seen_calls = Rc::clone(&calls);

    cmd_systemctl_with(
        "restart",
        OutputMode::Text,
        |args| {
            seen_calls
                .borrow_mut()
                .push(args.iter().map(ToString::to_string).collect());
            Ok(successful_output())
        },
        || Ok(status_fixture()),
    )?;

    assert_eq!(
        calls.borrow().as_slice(),
        [vec!["restart".to_string(), UNIT_NAME.to_string()]]
    );
    Ok(())
}

#[test]
fn run_command_reports_daemon_exit_status() -> Result<()> {
    let daemon_path = PathBuf::from("/tmp/waytorandrd");
    cmd_run_with(&daemon_path, false, |_, _| Ok(ExitStatus::from_raw(0)))?;

    let Err(err) = cmd_run_with(&daemon_path, false, |_, _| Ok(ExitStatus::from_raw(2 << 8)))
    else {
        bail!("failed daemon exit should fail");
    };

    assert_eq!(err.to_string(), "waytorandrd exited with status 2");
    Ok(())
}

#[test]
fn run_command_forwards_no_hooks() -> Result<()> {
    let daemon_path = PathBuf::from("/tmp/waytorandrd");
    let seen_no_hooks = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&seen_no_hooks);

    cmd_run_with(&daemon_path, true, |_, no_hooks| {
        *seen.borrow_mut() = Some(no_hooks);
        Ok(ExitStatus::from_raw(0))
    })?;

    assert_eq!(*seen_no_hooks.borrow(), Some(true));
    Ok(())
}

#[test]
fn uninstall_returns_error_when_disable_fails() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let unit_path = temp_dir.path().join(UNIT_NAME);
    fs::write(&unit_path, "[Unit]\nDescription=test\n")?;

    let Err(err) = cmd_uninstall_with(OutputMode::Text, &unit_path, |args| {
        assert_eq!(args, ["disable", "--now", UNIT_NAME]);
        bail!("disable failed")
    }) else {
        bail!("disable failure should be returned");
    };

    assert_eq!(err.to_string(), "disable failed");
    assert!(unit_path.exists(), "unit file should remain on failure");
    Ok(())
}
