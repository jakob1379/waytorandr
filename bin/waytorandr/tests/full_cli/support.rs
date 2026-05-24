use std::error::Error;
use std::io::Error as IoError;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;
use waytorandr_core::{Mode, OutputState, Position, Topology};

const BIN_NAME: &str = "waytorandr";
const TEST_BACKEND_STATE_ENV: &str = "WAYTORANDR_TEST_BACKEND_STATE";
const TEST_BACKEND_NAME_ENV: &str = "WAYTORANDR_TEST_BACKEND_NAME";

pub(crate) struct TestEnvironment {
    root: PathBuf,
    config_home: PathBuf,
    state_home: PathBuf,
    backend_state_path: PathBuf,
    backend_name: String,
}

#[derive(Deserialize)]
struct PersistedBackendState {
    topology: Topology,
}

pub(crate) struct FailureOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl TestEnvironment {
    pub(crate) fn new(backend_name: &str) -> Result<Self, Box<dyn Error>> {
        let root = unique_test_dir(backend_name)?;
        let config_home = root.join("config");
        let state_home = root.join("state");
        std::fs::create_dir_all(&config_home)?;
        std::fs::create_dir_all(&state_home)?;

        Ok(Self {
            root,
            config_home,
            backend_state_path: state_home.join("test-backend.json"),
            state_home,
            backend_name: backend_name.to_string(),
        })
    }

    pub(crate) fn write_backend_topology(&self, topology: &Topology) -> Result<(), Box<dyn Error>> {
        let content = serde_json::json!({ "topology": topology });
        std::fs::write(
            &self.backend_state_path,
            format!("{}\n", serde_json::to_string_pretty(&content)?),
        )?;
        Ok(())
    }

    pub(crate) fn backend_topology(&self) -> Result<Topology, Box<dyn Error>> {
        let content = std::fs::read_to_string(&self.backend_state_path)?;
        Ok(serde_json::from_str::<PersistedBackendState>(&content)?.topology)
    }

    pub(crate) fn state_file_path(&self) -> PathBuf {
        self.state_home.join("waytorandr").join("state.toml")
    }

    pub(crate) fn run_json<const N: usize>(
        &self,
        args: [&str; N],
    ) -> Result<Value, Box<dyn Error>> {
        println!("[{}] $ {} {}", self.backend_name, BIN_NAME, args.join(" "));
        let output = self.run(args)?;
        if !output.status.success() {
            return Err(format!(
                "command {:?} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
            .into());
        }

        serde_json::from_slice(&output.stdout).map_err(|source| {
            format!(
                "failed to parse JSON for {:?}: {source}\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
            .into()
        })
    }

    pub(crate) fn run_json_failure<const N: usize>(
        &self,
        args: [&str; N],
    ) -> Result<FailureOutput, Box<dyn Error>> {
        println!("[{}] $ {} {}", self.backend_name, BIN_NAME, args.join(" "));
        let output = self.run(args)?;
        if output.status.success() {
            return Err(format!(
                "command {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
            .into());
        }

        Ok(FailureOutput {
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        })
    }

    pub(crate) fn run_failure<const N: usize>(
        &self,
        args: [&str; N],
    ) -> Result<FailureOutput, Box<dyn Error>> {
        println!("[{}] $ {} {}", self.backend_name, BIN_NAME, args.join(" "));
        let output = self.run(args)?;
        if output.status.success() {
            return Err(format!(
                "command {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
            .into());
        }

        Ok(FailureOutput {
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        })
    }

    pub(crate) fn run_text<const N: usize>(
        &self,
        args: [&str; N],
    ) -> Result<String, Box<dyn Error>> {
        println!("[{}] $ {} {}", self.backend_name, BIN_NAME, args.join(" "));
        let output = self.run(args)?;
        if !output.status.success() {
            return Err(format!(
                "command {:?} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
            .into());
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    pub(crate) fn run_with_env<const N: usize>(
        &self,
        args: [&str; N],
        extra_env: &[(&str, &str)],
    ) -> Result<Output, Box<dyn Error>> {
        let mut command = self.base_command()?;
        command.args(args);
        for (key, value) in extra_env {
            command.env(key, value);
        }
        Ok(command.output()?)
    }

    fn run<const N: usize>(&self, args: [&str; N]) -> Result<Output, Box<dyn Error>> {
        let mut command = self.base_command()?;
        command.args(args);
        Ok(command.output()?)
    }

    fn base_command(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = Command::new(cli_bin()?);
        command
            .env("RUST_LOG", "error")
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env(TEST_BACKEND_STATE_ENV, &self.backend_state_path)
            .env(TEST_BACKEND_NAME_ENV, &self.backend_name)
            .env_remove("XDG_CURRENT_DESKTOP")
            .env_remove("XDG_SESSION_DESKTOP")
            .env_remove("DESKTOP_SESSION");

        let session_desktop = self.session_desktop();
        command
            .env("XDG_CURRENT_DESKTOP", session_desktop)
            .env("XDG_SESSION_DESKTOP", session_desktop)
            .env("DESKTOP_SESSION", session_desktop);

        Ok(command)
    }

    fn session_desktop(&self) -> &'static str {
        match self.backend_name.as_str() {
            "gnome" => "GNOME",
            "kscreen" => "plasma",
            _ => "sway",
        }
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn assert_saved_profiles(
    env: &TestEnvironment,
    expected_names: &[&str],
) -> Result<(), Box<dyn Error>> {
    let listed = env.run_json(["status", "--all", "--json"])?;
    let mut actual_names = Vec::new();
    for setup in listed["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?
    {
        for profile in setup["profiles"]
            .as_array()
            .ok_or_else(|| IoError::other("profiles array"))?
        {
            actual_names.push(
                profile["name"]
                    .as_str()
                    .ok_or_else(|| IoError::other("profile name should be a string"))?,
            );
        }
    }

    assert_eq!(actual_names, expected_names);
    Ok(())
}

pub(crate) fn assert_default_and_active(
    env: &TestEnvironment,
    expected_default: Option<&str>,
    expected_active: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let listed = env.run_json(["status", "--all", "--json"])?;
    let profiles: Vec<&Value> = listed["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?
        .iter()
        .flat_map(|setup| setup["profiles"].as_array().into_iter().flatten())
        .collect();

    let actual_default = profiles
        .iter()
        .find(|profile| profile["is_default"] == true)
        .and_then(|profile| profile["name"].as_str());
    let actual_active = profiles
        .iter()
        .find(|profile| profile["is_active"] == true)
        .and_then(|profile| profile["name"].as_str());

    assert_eq!(actual_default, expected_default);
    assert_eq!(actual_active, expected_active);
    Ok(())
}

pub(crate) fn assert_setup_name(
    env: &TestEnvironment,
    setup_fingerprint: &str,
    expected_setup_name: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let listed = env.run_json(["status", "--all", "--json"])?;
    let setup = listed["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?
        .iter()
        .find(|setup| setup["setup_fingerprint"].as_str() == Some(setup_fingerprint))
        .ok_or_else(|| IoError::other("matching setup should exist"))?;

    let actual_setup_name = setup["setup_name"].as_str();
    assert_eq!(actual_setup_name, expected_setup_name);
    Ok(())
}

fn cli_bin() -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(
        std::env::var_os("CARGO_BIN_EXE_waytorandr").ok_or_else(|| {
            IoError::other("cargo should provide the compiled waytorandr binary path")
        })?,
    ))
}

fn unique_test_dir(backend_name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "{BIN_NAME}-{backend_name}-{}-{nanos}",
        std::process::id()
    )))
}

pub(crate) fn fixture_topology() -> Topology {
    Topology {
        outputs: [
            (
                "DP-1".to_string(),
                full_cli_output_state(
                    "DP-1",
                    Some("Dell U2720Q"),
                    2560,
                    1440,
                    &[
                        Mode::new(2560, 1440, 60),
                        Mode::new(1920, 1080, 60),
                        Mode::new(1280, 720, 60),
                    ],
                    0,
                    0,
                ),
            ),
            (
                "eDP-1".to_string(),
                full_cli_output_state(
                    "eDP-1",
                    Some("Built-in Panel"),
                    1920,
                    1080,
                    &[Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)],
                    2560,
                    0,
                ),
            ),
        ]
        .into_iter()
        .collect(),
    }
}

pub(crate) fn alternate_topology() -> Topology {
    Topology {
        outputs: [(
            "eDP-1".to_string(),
            full_cli_output_state(
                "eDP-1",
                Some("Built-in Panel"),
                1920,
                1080,
                &[Mode::new(1920, 1080, 60), Mode::new(1280, 720, 60)],
                0,
                0,
            ),
        )]
        .into_iter()
        .collect(),
    }
}

fn full_cli_output_state(
    connector: &str,
    description: Option<&str>,
    width: u32,
    height: u32,
    available_modes: &[Mode],
    x: i32,
    y: i32,
) -> OutputState {
    let mut state = OutputState::new(connector);
    state.identity.description = description.map(str::to_string);
    state.enabled = true;
    state.mode = Some(Mode::new(width, height, 60));
    state.available_modes = available_modes.to_vec();
    state.position = Position::new(x, y);
    state
}
