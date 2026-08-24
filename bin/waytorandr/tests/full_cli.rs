#![cfg(feature = "test-backend")]

use std::error::Error;
use std::io::Error as IoError;

use serde_json::Value;
use waytorandr_core::{Mode, Position};

#[path = "full_cli/support.rs"]
mod full_cli_support;

use full_cli_support::*;

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
        setups[0]["setup_fingerprint"],
        alternate_topology().setup_fingerprint()
    );
    assert_eq!(setups[0]["profiles"][0]["name"], "travel");

    let all_status = env.run_json(["status", "--all", "--json"])?;
    assert_eq!(all_status["show_all"], true);
    assert_eq!(all_status["setups"].as_array().map(Vec::len), Some(2));

    Ok(())
}

#[test]
fn cycle_rotates_virtual_layouts_without_saved_profiles() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;
    env.write_backend_topology(&fixture_topology())?;

    let first = env.run_json(["cycle", "--json"])?;
    assert_eq!(first["target"], "horizontal");
    assert_eq!(first["target_type"], "virtual");

    let second = env.run_json(["cycle", "--json"])?;
    assert_eq!(second["target"], "vertical");
    assert_eq!(second["target_type"], "virtual");

    let wrapped = env.run_json(["cycle", "--json"])?;
    assert_eq!(wrapped["target"], "horizontal");
    assert_eq!(wrapped["target_type"], "virtual");
    Ok(())
}

#[test]
fn cycle_skips_presets_shadowed_by_saved_profile_names() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;
    env.write_backend_topology(&fixture_topology())?;
    env.run_json(["save", "horizontal", "--json"])?;

    let first = env.run_json(["cycle", "--json"])?;
    assert_eq!(first["target"], "horizontal");
    assert_eq!(first["target_type"], "profile");

    let second = env.run_json(["cycle", "--json"])?;
    assert_eq!(second["target"], "vertical");
    assert_eq!(second["target_type"], "virtual");

    let wrapped = env.run_json(["cycle", "--json"])?;
    assert_eq!(wrapped["target"], "horizontal");
    assert_eq!(wrapped["target_type"], "profile");
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
    assert_eq!(
        status["topology_fingerprint"],
        fixture_topology().fingerprint()
    );
    assert!(status.get("fingerprint").is_none());
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
    assert_eq!(cycle_dry_run["target"], "horizontal");
    assert_eq!(cycle_dry_run["dry_run"], true);
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;

    let cycled_horizontal = env.run_json(["cycle", "--json"])?;
    assert_eq!(cycled_horizontal["command"], "cycle");
    assert_eq!(cycled_horizontal["target"], "horizontal");
    assert_eq!(cycled_horizontal["target_type"], "virtual");

    let cycled_vertical = env.run_json(["cycle", "--json"])?;
    assert_eq!(cycled_vertical["target"], "vertical");
    assert_eq!(cycled_vertical["target_type"], "virtual");

    let cycled = env.run_json(["cycle", "--json"])?;
    assert_eq!(cycled["command"], "cycle");
    assert_eq!(cycled["target"], "desk");
    assert_eq!(cycled["target_type"], "profile");
    assert_default_and_active(env, Some("desk-alt"), Some("desk"))?;
    assert_eq!(env.run_json(["status", "--json"])?["profile"], "desk");

    let auto_set = env.run_json(["set", "auto", "--json"])?;
    assert_eq!(auto_set["command"], "set");
    assert_eq!(auto_set["selection"], "auto");
    assert_eq!(auto_set["target"], "desk-alt");
    assert_default_and_active(env, Some("desk-alt"), Some("desk-alt"))?;
    let status = env.run_json(["status", "--json"])?;
    assert_eq!(status["profile"], "desk-alt");
    assert_eq!(status["setup_name"], "office");
    assert_eq!(status["show_all"], false);
    assert_eq!(status["setups"].as_array().map(Vec::len), Some(1));

    let save_auto = env.run_json(["save", "auto", "--json"])?;
    assert_eq!(save_auto["saved"], true);

    let force_saved_auto = env.run_json(["set", "--profile", "auto", "--json"])?;
    assert_eq!(force_saved_auto["target"], "auto");
    assert_eq!(force_saved_auto["target_type"], "profile");
    Ok(())
}

#[allow(clippy::too_many_lines)]
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

    let set_setup_default = env.run_json(["set", "vertical", "--default", "--json"])?;
    assert_eq!(set_setup_default["target"], "vertical");
    assert_eq!(set_setup_default["saved_profile"], "default");
    assert_eq!(set_setup_default["default_set"], true);
    assert_eq!(set_setup_default["default_scope"], "setup");
    assert_eq!(set_setup_default["default_target"], "default");
    assert_saved_profiles(env, &["default"])?;
    assert_default_and_active(env, Some("default"), Some("default"))?;
    assert_eq!(env.run_json(["status", "--json"])?["profile"], "default");

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

    let set_external = env.run_json(["set", "external", "--json"])?;
    assert_eq!(set_external["target"], "external");
    assert_eq!(set_external["target_type"], "virtual");

    let topology = env.backend_topology()?;
    assert!(topology.outputs["DP-1"].enabled);
    assert!(!topology.outputs["eDP-1"].enabled);

    let top_level_help = env.run_text(["--help"])?;
    let set_help = env.run_text(["set", "--help"])?;
    let save_help = env.run_text(["save", "--help"])?;
    let service_run_help = env.run_text(["service", "run", "--help"])?;
    assert!(!top_level_help.contains("builtin"));
    assert!(set_help.contains("builtin"));
    assert!(!save_help.contains("builtin"));
    assert!(!service_run_help.contains("builtin"));

    Ok(())
}

#[test]
fn cli_help_renders_correctly() -> Result<(), Box<dyn Error>> {
    let env = TestEnvironment::new("wlroots")?;
    env.write_backend_topology(&fixture_topology())?;

    let bare_set = env.run_failure(["set"])?;
    assert!(bare_set.stderr.contains("<target>") || bare_set.stderr.contains("<TARGET>"));
    assert!(bare_set.stderr.contains("Usage") || bare_set.stderr.contains("USAGE"));

    let top_level_help = env.run_text(["--help"])?;
    assert!(top_level_help.contains("USAGE") || top_level_help.contains("Usage"));
    assert!(top_level_help.contains("waytorandr"));

    let save_help = env.run_text(["save", "--help"])?;
    assert!(save_help.contains("USAGE") || save_help.contains("Usage"));
    assert!(save_help.contains("save"));

    let remove_help = env.run_text(["remove", "--help"])?;
    assert!(remove_help.contains("current setup"));

    let set_help = env.run_text(["set", "--help"])?;
    assert!(set_help.contains("USAGE") || set_help.contains("Usage"));
    assert!(set_help.contains("set"));
    assert!(set_help.contains("waytorandr set auto"));
    assert!(set_help
        .contains("Prefer external outputs; if none are present, keep built-in panels enabled"));

    let service_run_help = env.run_text(["service", "run", "--help"])?;
    assert!(service_run_help.contains("Usage") || service_run_help.contains("USAGE"));
    assert!(service_run_help.contains("run"));

    Ok(())
}

fn exercise_remove_workflows(env: &TestEnvironment) -> Result<(), Box<dyn Error>> {
    let remove_dry_run = env.run_json(["remove", "desk-alt", "--dry-run", "--json"])?;
    assert_eq!(remove_dry_run["would_remove"], true);
    assert_eq!(remove_dry_run.get("removed"), None);
    assert_saved_profiles(env, &["auto", "desk", "desk-alt"])?;

    let remove = env.run_json(["remove", "desk-alt", "--json"])?;
    assert_eq!(remove["removed"], true);
    assert_eq!(remove.get("would_remove"), None);
    assert_saved_profiles(env, &["auto", "desk"])?;

    let remove_missing_text = env.run_failure(["remove", "desk-alt"])?;
    assert!(remove_missing_text
        .stderr
        .contains("profile 'desk-alt' not found for the current setup"));

    let remove_missing_json = env.run_json_failure(["remove", "desk-alt", "--json"])?;
    assert!(remove_missing_json
        .stderr
        .contains("profile 'desk-alt' not found for the current setup"));
    let remove_missing_json_stdout: Value = serde_json::from_str(&remove_missing_json.stdout)?;
    assert_eq!(remove_missing_json_stdout["command"], "remove");
    assert_eq!(remove_missing_json_stdout["removed"], false);
    assert_eq!(remove_missing_json_stdout["profile"], "desk-alt");
    assert_eq!(remove_missing_json_stdout["dry_run"], false);
    assert!(remove_missing_json_stdout.get("would_remove").is_none());

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
    assert!(status_text.contains("setup fingerprint:"));

    Ok(())
}
