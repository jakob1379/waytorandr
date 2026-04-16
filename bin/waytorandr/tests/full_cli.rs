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
fn full_cli_status_across_simulated_backends() -> Result<(), Box<dyn Error>> {
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

#[test]
fn status_defaults_to_current_setup_and_expands_with_all() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;

    env.write_backend_topology(&fixture_topology())?;
    env.run_json(["save", "desk", "--json"])?;

    env.write_backend_topology(&alternate_topology())?;
    env.run_json(["save", "travel", "--json"])?;

    let status = env.run_json(["status", "--json"])?;
    let setups = status["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?;
    assert_eq!(setups.len(), 1);
    assert_eq!(
        setups[0]["fingerprint"],
        alternate_topology().setup_fingerprint()
    );
    assert_eq!(setups[0]["profiles"][0]["name"], "travel");

    let all_status = env.run_json(["status", "--all", "--json"])?;
    assert_eq!(all_status["show_all"], true);
    assert_eq!(all_status["setups"].as_array().map(Vec::len), Some(2));

    Ok(())
}

#[test]
fn json_output_stays_plain_when_color_is_forced() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;

    env.write_backend_topology(&fixture_topology())?;
    let output = env.run_with_env(["status", "--json"], &[("CLICOLOR_FORCE", "1")])?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(!stdout.contains("\u{1b}["));

    let status: Value = serde_json::from_str(&stdout)?;
    assert_eq!(status["command"], "status");
    Ok(())
}

fn assert_initial_state(env: &TestEnvironment) -> Result<(), Box<dyn Error>> {
    let status_text = env.run_text(["status"])?;
    assert!(status_text.contains("Current profile: none"));
    assert!(status_text.contains("Setup fingerprint:"));
    assert!(status_text.contains("Detected outputs:"));
    assert!(status_text.contains("DP-1"));
    assert!(status_text.contains("eDP-1"));
    assert!(status_text.contains("Profiles: none saved"));

    let status = env.run_json(["status", "--json"])?;
    assert_eq!(status["command"], "status");
    assert_eq!(status["show_all"], false);
    assert_eq!(status["has_saved_profiles"], false);
    assert_eq!(
        status["setup_fingerprint"],
        fixture_topology().setup_fingerprint()
    );
    assert!(status["setup_name"].is_null());
    assert_eq!(status["fingerprint"], fixture_topology().fingerprint());
    assert_eq!(status["outputs"].as_array().map(Vec::len), Some(2));
    assert!(status["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?
        .is_empty());
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

    let save_desk = env.run_json([
        "save",
        "desk",
        "--default",
        "--setup-name",
        "office",
        "--json",
    ])?;
    assert_eq!(save_desk["saved"], true);
    assert_eq!(save_desk["default_set"], true);
    assert_eq!(save_desk["setup_name"], "office");

    let save_alt = env.run_json(["save", "desk-alt", "--json"])?;
    assert_eq!(save_alt["saved"], true);
    assert_saved_profiles(env, &["desk", "desk-alt"])?;
    assert_default_and_active(env, Some("desk"), None)?;
    assert_setup_name(
        env,
        fixture_topology().setup_fingerprint().as_str(),
        Some("office"),
    )?;

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
    assert_eq!(env.run_json(["status", "--json"])?["profile"], "desk-alt");

    let cycle_dry_run = env.run_json(["cycle", "--dry-run", "--json"])?;
    assert_eq!(cycle_dry_run["command"], "cycle");
    assert_eq!(cycle_dry_run["target"], "desk");
    assert_eq!(cycle_dry_run["dry_run"], true);
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;

    let cycled = env.run_json(["cycle", "--json"])?;
    assert_eq!(cycled["command"], "cycle");
    assert_eq!(cycled["target"], "desk");
    assert_default_and_active(env, Some("desk-alt"), Some("desk"))?;
    assert_eq!(env.run_json(["status", "--json"])?["profile"], "desk");

    let auto_set = env.run_json(["set", "--json"])?;
    assert_eq!(auto_set["command"], "set");
    assert_eq!(auto_set["selection"], "auto");
    assert_eq!(auto_set["target"], "desk-alt");
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;
    let status = env.run_json(["status", "--json"])?;
    assert_eq!(status["profile"], "desk-alt");
    assert_eq!(status["setup_name"], "office");
    assert_eq!(status["show_all"], false);
    assert_eq!(status["setups"].as_array().map(Vec::len), Some(1));
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

    let invalid_reverse = env.run_json_failure(["set", "external", "--reverse", "--json"])?;
    assert!(invalid_reverse.stderr.contains(
        "--reverse can only be used with virtual 'horizontal' or 'vertical' set targets"
    ));

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

    let set_default = env.run_json(["set", "external", "--default", "--json"])?;
    assert_eq!(set_default["target"], "external");
    assert_eq!(set_default["default_set"], true);
    assert_eq!(set_default["default_scope"], "new_setups");

    let topology = env.backend_topology()?;
    assert!(topology.outputs["DP-1"].enabled);
    assert!(!topology.outputs["eDP-1"].enabled);

    let status = env.run_json(["status", "--json"])?;
    assert_eq!(status["new_setup_default"]["kind"], "virtual");
    assert_eq!(status["new_setup_default"]["preset"], "external");

    env.write_backend_topology(&alternate_topology())?;
    let auto_set = env.run_json(["set", "--json"])?;
    assert_eq!(auto_set["selection"], "auto");
    assert_eq!(auto_set["target"], "external");
    assert_eq!(auto_set["target_type"], "virtual");

    Ok(())
}

#[test]
fn cli_help_renders_correctly() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;
    env.write_backend_topology(&fixture_topology())?;

    let top_level_help = env.run_text(["--help"])?;
    assert!(top_level_help.contains("USAGE") || top_level_help.contains("Usage"));
    assert!(top_level_help.contains("waytorandr"));

    let save_help = env.run_text(["save", "--help"])?;
    assert!(save_help.contains("USAGE") || save_help.contains("Usage"));
    assert!(save_help.contains("save"));

    let set_help = env.run_text(["set", "--help"])?;
    assert!(set_help.contains("USAGE") || set_help.contains("Usage"));
    assert!(set_help.contains("set"));

    let service_run_help = env.run_text(["service", "run", "--help"])?;
    assert!(service_run_help.contains("Usage") || service_run_help.contains("USAGE"));
    assert!(service_run_help.contains("run"));

    Ok(())
}

fn exercise_remove_workflows(env: &TestEnvironment) -> Result<(), Box<dyn Error>> {
    let remove_dry_run = env.run_json(["remove", "desk-alt", "--dry-run", "--json"])?;
    assert_eq!(remove_dry_run["would_remove"], true);
    assert_eq!(remove_dry_run.get("removed"), None);
    assert_saved_profiles(env, &["desk", "desk-alt"])?;

    let remove = env.run_json(["remove", "desk-alt", "--json"])?;
    assert_eq!(remove["removed"], true);
    assert_eq!(remove.get("would_remove"), None);
    assert_saved_profiles(env, &["desk"])?;

    let status_text = env.run_text(["status", "--all"])?;
    assert!(status_text.contains("Profiles:"));
    assert!(status_text.contains("desk"));
    Ok(())
}

#[test]
fn save_updates_setup_name_for_current_setup() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;
    env.write_backend_topology(&fixture_topology())?;

    env.run_json(["save", "desk", "--setup-name", "office", "--json"])?;
    assert_setup_name(
        &env,
        fixture_topology().setup_fingerprint().as_str(),
        Some("office"),
    )?;

    env.run_json(["save", "desk-alt", "-s", "office-basement", "--json"])?;
    assert_setup_name(
        &env,
        fixture_topology().setup_fingerprint().as_str(),
        Some("office-basement"),
    )?;

    let status_text = env.run_text(["status", "--all"])?;
    assert!(status_text.contains("setup: office-basement"));
    assert!(status_text.contains("fingerprint:"));

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

    fn run_with_env<const N: usize>(
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

fn assert_saved_profiles(
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

fn assert_default_and_active(
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

fn assert_setup_name(
    env: &TestEnvironment,
    setup_fingerprint: &str,
    expected_setup_name: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let listed = env.run_json(["status", "--all", "--json"])?;
    let setup = listed["setups"]
        .as_array()
        .ok_or_else(|| IoError::other("setups array"))?
        .iter()
        .find(|setup| setup["fingerprint"].as_str() == Some(setup_fingerprint))
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

fn alternate_topology() -> Topology {
    Topology {
        outputs: [(
            "eDP-1".to_string(),
            output(
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
