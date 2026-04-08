use std::error::Error;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;
use waytorandr_core::model::{Mode, OutputState, Position, Topology};

const BIN_NAME: &str = "waytorandr";
const TEST_BACKEND_STATE_ENV: &str = "WAYTORANDR_TEST_BACKEND_STATE";
const TEST_BACKEND_NAME_ENV: &str = "WAYTORANDR_TEST_BACKEND_NAME";

#[test]
fn full_cli_command_matrix_across_platforms() -> Result<(), Box<dyn Error>> {
    let platforms = ["wlroots", "kscreen", "gnome"];
    println!("platforms under test: {}", platforms.join(", "));

    for platform in platforms {
        let env = TestEnvironment::new(platform)?;
        let supports_native_mirror = platform != "wlroots";
        let supports_largest = platform == "kscreen";
        let mirror_note = if supports_native_mirror && supports_largest {
            "mirror, largest, common -l"
        } else if supports_native_mirror {
            "mirror, largest(expected failure), common -l(expected failure)"
        } else {
            "mirror(expected failure), largest(expected failure), common -l(expected failure)"
        };

        println!(
            "\nplatform {platform}: running detected, list, current, save --dry-run, save --default, save, set --dry-run, set --default, cycle --dry-run, cycle, set(auto), off, horizontal, vertical --reverse, common, {mirror_note}, remove --dry-run, remove"
        );

        env.write_backend_topology(&fixture_topology());

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
            .expect("setups array")
            .is_empty());

        let initial_current = env.run_text(["current"])?;
        assert!(initial_current.contains("Current profile: none"));

        let save_default_dry_run = env.run_json(["save", "--dry-run", "--json"])?;
        assert_eq!(save_default_dry_run["command"], "save");
        assert_eq!(save_default_dry_run["profile"], "default");
        assert_eq!(save_default_dry_run["dry_run"], true);
        assert_eq!(save_default_dry_run["saved"], false);
        assert_eq!(
            save_default_dry_run["plan"].as_array().map(Vec::len),
            Some(2)
        );
        assert_saved_profiles(&env, &[]);

        let save_desk = env.run_json(["save", "desk", "--default", "--json"])?;
        assert_eq!(save_desk["saved"], true);
        assert_eq!(save_desk["default_set"], true);

        let save_alt = env.run_json(["save", "desk-alt", "--json"])?;
        assert_eq!(save_alt["saved"], true);
        assert_saved_profiles(&env, &["desk", "desk-alt"]);
        assert_default_and_active(&env, Some("desk"), None);

        let set_dry_run = env.run_json(["set", "desk-alt", "--dry-run", "--json"])?;
        assert_eq!(set_dry_run["command"], "set");
        assert_eq!(set_dry_run["target"], "desk-alt");
        assert_eq!(set_dry_run["target_type"], "profile");
        assert_eq!(set_dry_run["dry_run"], true);
        assert_eq!(set_dry_run["validation"]["success"], true);
        assert_default_and_active(&env, Some("desk"), None);

        let set_make_default = env.run_json(["set", "desk-alt", "--default", "--json"])?;
        assert_eq!(set_make_default["target"], "desk-alt");
        assert_eq!(set_make_default["default_set"], true);
        assert_default_and_active(&env, Some("desk-alt"), Some("desk-alt"));
        assert_eq!(env.run_json(["current", "--json"])?["profile"], "desk-alt");

        let cycle_dry_run = env.run_json(["cycle", "--dry-run", "--json"])?;
        assert_eq!(cycle_dry_run["command"], "cycle");
        assert_eq!(cycle_dry_run["target"], "desk");
        assert_eq!(cycle_dry_run["dry_run"], true);
        assert_default_and_active(&env, Some("desk-alt"), Some("desk-alt"));

        let cycled = env.run_json(["cycle", "--json"])?;
        assert_eq!(cycled["command"], "cycle");
        assert_eq!(cycled["target"], "desk");
        assert_default_and_active(&env, Some("desk-alt"), Some("desk"));
        assert_eq!(env.run_json(["current", "--json"])?["profile"], "desk");

        let auto_set = env.run_json(["set", "--json"])?;
        assert_eq!(auto_set["command"], "set");
        assert_eq!(auto_set["selection"], "auto");
        assert_eq!(auto_set["target"], "desk-alt");
        assert_default_and_active(&env, Some("desk-alt"), Some("desk-alt"));
        assert_eq!(env.run_json(["current", "--json"])?["profile"], "desk-alt");

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
            if supports_largest {
                let largest = env.run_json(["set", "largest", "--json"])?;
                assert_eq!(largest["target"], "largest");
                let topology = env.backend_topology()?;
                assert_eq!(
                    topology.outputs["DP-1"].mode,
                    Some(Mode::new(2560, 1440, 60))
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

                let legacy_largest = env.run_json(["set", "common", "-l", "--json"])?;
                assert_eq!(legacy_largest["target"], "largest");
                let topology = env.backend_topology()?;
                assert_eq!(
                    topology.outputs["DP-1"].mode,
                    Some(Mode::new(2560, 1440, 60))
                );
                assert_eq!(
                    topology.outputs["eDP-1"].mode,
                    Some(Mode::new(1920, 1080, 60))
                );
                assert_eq!(
                    topology.outputs["eDP-1"].mirror_target.as_deref(),
                    Some("DP-1")
                );
            } else {
                let largest = env.run_json_failure(["set", "largest", "--json"])?;
                assert!(largest
                    .stderr
                    .contains("the `largest` layout is not available"));
                assert!(largest.stderr.contains("mirror"));
                assert!(largest.stderr.contains("common"));

                let legacy_largest = env.run_json_failure(["set", "common", "-l", "--json"])?;
                assert!(legacy_largest
                    .stderr
                    .contains("the `largest` layout is not available"));
            }
        } else {
            let mirror = env.run_json_failure(["set", "mirror", "--json"])?;
            assert!(mirror
                .stderr
                .contains("native display mirroring is not available"));
            assert!(mirror.stderr.contains("wl-mirror"));

            let largest = env.run_json_failure(["set", "largest", "--json"])?;
            assert!(largest
                .stderr
                .contains("native display mirroring is not available"));

            let legacy_largest = env.run_json_failure(["set", "common", "-l", "--json"])?;
            assert!(legacy_largest
                .stderr
                .contains("native display mirroring is not available"));
        }

        let remove_dry_run = env.run_json(["remove", "desk-alt", "--dry-run", "--json"])?;
        assert_eq!(remove_dry_run["removed"], true);
        assert_saved_profiles(&env, &["desk", "desk-alt"]);

        let remove = env.run_json(["remove", "desk-alt", "--json"])?;
        assert_eq!(remove["removed"], true);
        assert_saved_profiles(&env, &["desk"]);

        let list_text = env.run_text(["list", "--all"])?;
        assert!(list_text.contains("Profiles:"));
        assert!(list_text.contains("desk"));
    }

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
        let root = unique_test_dir(backend_name);
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

    fn write_backend_topology(&self, topology: &Topology) {
        let content = serde_json::json!({ "topology": topology });
        std::fs::write(
            &self.backend_state_path,
            format!("{}\n", serde_json::to_string_pretty(&content).unwrap()),
        )
        .unwrap();
    }

    fn backend_topology(&self) -> Result<Topology, Box<dyn Error>> {
        let content = std::fs::read_to_string(&self.backend_state_path)?;
        Ok(serde_json::from_str::<PersistedBackendState>(&content)?.topology)
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
        Ok(Command::new(cli_bin())
            .args(args)
            .env("RUST_LOG", "error")
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env(TEST_BACKEND_STATE_ENV, &self.backend_state_path)
            .env(TEST_BACKEND_NAME_ENV, &self.backend_name)
            .output()?)
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn assert_saved_profiles(env: &TestEnvironment, expected_names: &[&str]) {
    let listed = env
        .run_json(["list", "--all", "--json"])
        .expect("list command should succeed");
    let actual_names: Vec<&str> = listed["setups"]
        .as_array()
        .expect("setups array")
        .iter()
        .flat_map(|setup| {
            setup["profiles"]
                .as_array()
                .expect("profiles array")
                .iter()
                .map(|profile| {
                    profile["name"]
                        .as_str()
                        .expect("profile name should be a string")
                })
        })
        .collect();

    assert_eq!(actual_names, expected_names);
}

fn assert_default_and_active(
    env: &TestEnvironment,
    expected_default: Option<&str>,
    expected_active: Option<&str>,
) {
    let listed = env
        .run_json(["list", "--all", "--json"])
        .expect("list command should succeed");
    let profiles: Vec<&Value> = listed["setups"]
        .as_array()
        .expect("setups array")
        .iter()
        .flat_map(|setup| setup["profiles"].as_array().expect("profiles array").iter())
        .collect();

    let actual_default = profiles
        .iter()
        .find(|profile| profile["is_default"] == true)
        .map(|profile| profile["name"].as_str().expect("default name"));
    let actual_active = profiles
        .iter()
        .find(|profile| profile["is_active"] == true)
        .map(|profile| profile["name"].as_str().expect("active name"));

    assert_eq!(actual_default, expected_default);
    assert_eq!(actual_active, expected_active);
}

fn cli_bin() -> PathBuf {
    PathBuf::from(
        std::env::var_os("CARGO_BIN_EXE_waytorandr")
            .expect("cargo should provide the compiled waytorandr binary path"),
    )
}

fn unique_test_dir(backend_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "{BIN_NAME}-{backend_name}-{}-{nanos}",
        std::process::id()
    ))
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
