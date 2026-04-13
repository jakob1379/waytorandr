use std::error::Error;
use std::io::Error as IoError;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;
use waytorandr_core::model::{Mode, OutputState, Position, Topology};

const BIN_NAME: &str = "waytorandr";
const TEST_BACKEND_STATE_ENV: &str = "WAYTORANDR_TEST_BACKEND_STATE";
const TEST_BACKEND_NAME_ENV: &str = "WAYTORANDR_TEST_BACKEND_NAME";
const SIMULATED_BACKENDS: [&str; 3] = ["wlroots", "kscreen", "gnome"];

#[test]
fn full_cli_detect_list_current_across_simulated_backends() -> Result<(), Box<dyn Error>> {
    println!(
        "simulated backends under test: {}",
        SIMULATED_BACKENDS.join(", ")
    );

    for platform in SIMULATED_BACKENDS {
        let env = TestEnvironment::new(platform)?;
        env.write_backend_topology(&fixture_topology())?;
        assert_initial_state(&env)?;
    }

    Ok(())
}

#[test]
fn full_cli_save_set_across_simulated_backends() -> Result<(), Box<dyn Error>> {
    println!(
        "simulated backends under test: {}",
        SIMULATED_BACKENDS.join(", ")
    );

    for platform in SIMULATED_BACKENDS {
        let env = TestEnvironment::new(platform)?;
        env.write_backend_topology(&fixture_topology())?;
        exercise_save_and_set_workflows(&env)?;
    }

    Ok(())
}

#[test]
fn full_cli_virtual_across_simulated_backends() -> Result<(), Box<dyn Error>> {
    println!(
        "simulated backends under test: {}",
        SIMULATED_BACKENDS.join(", ")
    );

    for platform in SIMULATED_BACKENDS {
        let env = TestEnvironment::new(platform)?;
        let supports_native_mirror = platform != "wlroots";
        env.write_backend_topology(&fixture_topology())?;
        exercise_virtual_workflows(&env, supports_native_mirror)?;
    }

    Ok(())
}

#[test]
fn full_cli_remove_across_simulated_backends() -> Result<(), Box<dyn Error>> {
    println!(
        "simulated backends under test: {}",
        SIMULATED_BACKENDS.join(", ")
    );

    for platform in SIMULATED_BACKENDS {
        let env = TestEnvironment::new(platform)?;
        env.write_backend_topology(&fixture_topology())?;
        exercise_save_and_set_workflows(&env)?;
        exercise_remove_workflows(&env)?;
    }

    Ok(())
}

fn assert_initial_state(env: &TestEnvironment) -> Result<(), Box<dyn Error>> {
    let detected_text = env.run_text(["detected"])?;
    assert!(detected_text.contains("Detected outputs:"));
    assert!(detected_text.contains("DP-1"));
    assert!(detected_text.contains("eDP-1"));

    let detected = env.run_json(["detected", "--json"])?;
    assert_eq!(detected["command"], "detected");
    assert_eq!(
        detected["setup_fingerprint"],
        fixture_topology().setup_fingerprint()
    );
    assert_eq!(detected["outputs"].as_array().map(Vec::len), Some(2));

    let initial_list = env.run_json(["list", "--json"])?;
    assert_eq!(initial_list["command"], "list");
    assert!(initial_list["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?
        .is_empty());

    let initial_current = env.run_text(["current"])?;
    assert!(initial_current.contains("Current profile: none"));
    assert!(!env.state_file_path().exists());
    Ok(())
}

fn exercise_save_and_set_workflows(env: &TestEnvironment) -> Result<(), Box<dyn Error>> {
    let save_default_dry_run = env.run_json(["save", "--dry-run", "--json"])?;
    assert_eq!(save_default_dry_run["command"], "save");
    assert_eq!(save_default_dry_run["profile"], "default");
    assert_eq!(save_default_dry_run["dry_run"], true);
    assert_eq!(save_default_dry_run["saved"], false);
    assert_eq!(
        save_default_dry_run["plan"].as_array().map(Vec::len),
        Some(2)
    );
    assert_saved_profiles(env, &[])?;
    assert!(!env.state_file_path().exists());

    let save_desk = env.run_json(["save", "desk", "--default", "--json"])?;
    assert_eq!(save_desk["saved"], true);
    assert_eq!(save_desk["default_set"], true);

    let save_alt = env.run_json(["save", "desk-alt", "--json"])?;
    assert_eq!(save_alt["saved"], true);
    assert_saved_profiles(env, &["desk", "desk-alt"])?;
    assert_default_and_active(env, Some("desk"), None)?;

    let set_dry_run = env.run_json(["set", "desk-alt", "--dry-run", "--json"])?;
    assert_eq!(set_dry_run["command"], "set");
    assert_eq!(set_dry_run["target"], "desk-alt");
    assert_eq!(set_dry_run["target_type"], "profile");
    assert_eq!(set_dry_run["dry_run"], true);
    assert_eq!(set_dry_run["validation"]["success"], true);
    assert_default_and_active(env, Some("desk"), None)?;

    let set_make_default = env.run_json(["set", "desk-alt", "--default", "--json"])?;
    assert_eq!(set_make_default["target"], "desk-alt");
    assert_eq!(set_make_default["default_set"], true);
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;
    assert_eq!(env.run_json(["current", "--json"])?["profile"], "desk-alt");

    let cycle_dry_run = env.run_json(["cycle", "--dry-run", "--json"])?;
    assert_eq!(cycle_dry_run["command"], "cycle");
    assert_eq!(cycle_dry_run["target"], "desk");
    assert_eq!(cycle_dry_run["dry_run"], true);
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;

    let cycled = env.run_json(["cycle", "--json"])?;
    assert_eq!(cycled["command"], "cycle");
    assert_eq!(cycled["target"], "desk");
    assert_default_and_active(env, Some("desk-alt"), Some("desk"))?;
    assert_eq!(env.run_json(["current", "--json"])?["profile"], "desk");

    let auto_set = env.run_json(["set", "--json"])?;
    assert_eq!(auto_set["command"], "set");
    assert_eq!(auto_set["selection"], "auto");
    assert_eq!(auto_set["target"], "desk-alt");
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;
    assert_eq!(env.run_json(["current", "--json"])?["profile"], "desk-alt");
    Ok(())
}

fn exercise_virtual_workflows(
    env: &TestEnvironment,
    supports_native_mirror: bool,
) -> Result<(), Box<dyn Error>> {
    let off = env.run_json(["set", "off", "--json"])?;
    assert_eq!(off["target"], "off");
    assert_eq!(off["target_type"], "virtual");
    let topology = env.backend_topology()?;
    assert!(!topology.outputs["DP-1"].enabled);
    assert!(topology.outputs["eDP-1"].enabled);
    assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(0, 0));

    let horizontal = env.run_json(["set", "horizontal", "--json"])?;
    assert_eq!(horizontal["target"], "horizontal");
    let topology = env.backend_topology()?;
    assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(2560, 0));
    assert!(topology.outputs.values().all(|state| state.enabled));
    assert!(topology
        .outputs
        .values()
        .all(|state| state.mirror_target.is_none()));

    let vertical_reverse = env.run_json(["set", "vertical", "--reverse", "--json"])?;
    assert_eq!(vertical_reverse["target"], "vertical-reverse");
    let topology = env.backend_topology()?;
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(320, 0));
    assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 1080));

    let common = env.run_json(["set", "common", "--json"])?;
    assert_eq!(common["target"], "common");
    let topology = env.backend_topology()?;
    assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(0, 0));
    assert_eq!(
        topology.outputs["DP-1"].mode,
        Some(Mode::new(1920, 1080, 60))
    );
    assert_eq!(
        topology.outputs["eDP-1"].mode,
        Some(Mode::new(1920, 1080, 60))
    );
    assert!(topology
        .outputs
        .values()
        .all(|state| state.mirror_target.is_none()));

    let largest = env.run_json(["set", "largest", "--json"])?;
    assert_eq!(largest["target"], "largest");
    let topology = env.backend_topology()?;
    assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(0, 0));
    assert_eq!(
        topology.outputs["DP-1"].mode,
        Some(Mode::new(2560, 1440, 60))
    );
    assert_eq!(
        topology.outputs["eDP-1"].mode,
        Some(Mode::new(1920, 1080, 60))
    );
    assert!(topology
        .outputs
        .values()
        .all(|state| state.mirror_target.is_none()));

    let largest_via_common_flag = env.run_json(["set", "common", "-l", "--json"])?;
    assert_eq!(largest_via_common_flag["target"], "largest");
    let topology = env.backend_topology()?;
    assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 0));
    assert_eq!(topology.outputs["eDP-1"].position, Position::new(0, 0));
    assert_eq!(
        topology.outputs["DP-1"].mode,
        Some(Mode::new(2560, 1440, 60))
    );
    assert_eq!(
        topology.outputs["eDP-1"].mode,
        Some(Mode::new(1920, 1080, 60))
    );
    assert!(topology
        .outputs
        .values()
        .all(|state| state.mirror_target.is_none()));

    if supports_native_mirror {
        let mirror = env.run_json(["set", "mirror", "--json"])?;
        assert_eq!(mirror["target"], "mirror");
        let topology = env.backend_topology()?;
        assert_eq!(topology.outputs["DP-1"].position, Position::new(0, 0));
        assert_eq!(topology.outputs["eDP-1"].position, Position::new(0, 0));
        assert_eq!(
            topology.outputs["DP-1"].mode,
            Some(Mode::new(1920, 1080, 60))
        );
        assert_eq!(
            topology.outputs["eDP-1"].mode,
            Some(Mode::new(1920, 1080, 60))
        );
        assert_eq!(topology.outputs["DP-1"].mirror_target, None);
        assert_eq!(
            topology.outputs["eDP-1"].mirror_target.as_deref(),
            Some("DP-1")
        );
    } else {
        let mirror = env.run_json_failure(["set", "mirror", "--json"])?;
        assert!(mirror
            .stderr
            .contains("native display mirroring is not available"));
        assert!(mirror.stderr.contains("wl-mirror"));
    }

    Ok(())
}

fn exercise_remove_workflows(env: &TestEnvironment) -> Result<(), Box<dyn Error>> {
    let remove_dry_run = env.run_json(["remove", "desk-alt", "--dry-run", "--json"])?;
    assert_eq!(remove_dry_run["removed"], true);
    assert_saved_profiles(env, &["desk", "desk-alt"])?;

    let remove = env.run_json(["remove", "desk-alt", "--json"])?;
    assert_eq!(remove["removed"], true);
    assert_saved_profiles(env, &["desk"])?;

    let list_text = env.run_text(["list", "--all"])?;
    assert!(list_text.contains("Profiles:"));
    assert!(list_text.contains("desk"));
    Ok(())
}

struct TestEnvironment {
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

struct FailureOutput {
    stderr: String,
}

impl TestEnvironment {
    fn new(backend_name: &str) -> Result<Self, Box<dyn Error>> {
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

    fn write_backend_topology(&self, topology: &Topology) -> Result<(), Box<dyn Error>> {
        let content = serde_json::json!({ "topology": topology });
        std::fs::write(
            &self.backend_state_path,
            format!("{}\n", serde_json::to_string_pretty(&content)?),
        )?;
        Ok(())
    }

    fn backend_topology(&self) -> Result<Topology, Box<dyn Error>> {
        let content = std::fs::read_to_string(&self.backend_state_path)?;
        Ok(serde_json::from_str::<PersistedBackendState>(&content)?.topology)
    }

    fn state_file_path(&self) -> PathBuf {
        self.state_home.join("waytorandr").join("state.toml")
    }

    fn run_json<const N: usize>(&self, args: [&str; N]) -> Result<Value, Box<dyn Error>> {
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

    fn run_json_failure<const N: usize>(
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
            stderr: String::from_utf8(output.stderr)?,
        })
    }

    fn run_text<const N: usize>(&self, args: [&str; N]) -> Result<String, Box<dyn Error>> {
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

    fn run<const N: usize>(&self, args: [&str; N]) -> Result<Output, Box<dyn Error>> {
        let mut command = Command::new(cli_bin()?);
        command
            .args(args)
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

        Ok(command.output()?)
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

fn assert_saved_profiles(
    env: &TestEnvironment,
    expected_names: &[&str],
) -> Result<(), Box<dyn Error>> {
    let listed = env.run_json(["list", "--all", "--json"])?;
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

fn assert_default_and_active(
    env: &TestEnvironment,
    expected_default: Option<&str>,
    expected_active: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let listed = env.run_json(["list", "--all", "--json"])?;
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

fn fixture_topology() -> Topology {
    Topology {
        outputs: [
            (
                "DP-1".to_string(),
                output(
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
                output(
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

fn output(
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
