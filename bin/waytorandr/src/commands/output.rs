use anyhow::Result;
use serde::Serialize;
use std::io::IsTerminal;
use std::sync::OnceLock;

use waytorandr_core::engine::TestResult;
use waytorandr_core::model::Mode;
use waytorandr_core::model::Topology;
use waytorandr_core::planner::LayoutPlan;

const RESET: &str = "\x1b[0m";

fn color_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let term = std::env::var("TERM").ok();
        let force = std::env::var("CLICOLOR_FORCE").ok();
        color_enabled_for(
            std::io::stdout().is_terminal(),
            term.as_deref(),
            std::env::var_os("NO_COLOR").is_some(),
            force.as_deref(),
        )
    })
}

fn color_enabled_for(
    stdout_is_terminal: bool,
    term: Option<&str>,
    no_color: bool,
    force_color: Option<&str>,
) -> bool {
    if no_color {
        return false;
    }
    if force_color.is_some_and(|value| value != "0") {
        return true;
    }
    stdout_is_terminal && term != Some("dumb")
}

fn paint_with(text: impl AsRef<str>, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{}{}", text.as_ref(), RESET)
    } else {
        text.as_ref().to_string()
    }
}

fn paint(text: impl AsRef<str>, code: &str) -> String {
    paint_with(text, code, color_enabled())
}

pub(super) fn heading(text: impl AsRef<str>) -> String {
    paint(text, "1")
}

pub(super) fn key(text: impl AsRef<str>) -> String {
    paint(text, "36")
}

pub(super) fn success(text: impl AsRef<str>) -> String {
    paint(text, "32")
}

pub(super) fn warning(text: impl AsRef<str>) -> String {
    paint(text, "33")
}

pub(super) fn failure(text: impl AsRef<str>) -> String {
    paint(text, "31")
}

pub(super) fn enabled_label(value: bool) -> String {
    if value {
        success("enabled")
    } else {
        paint("disabled", "90")
    }
}

pub(super) fn yes_no(value: bool) -> String {
    if value {
        paint("yes", "32")
    } else {
        paint("no", "90")
    }
}

pub(super) fn status_label(label: &str) -> String {
    match label {
        "ok" | "supported" | "active" | "enabled" | "installed" | "yes" | "running" => {
            success(label)
        }
        "warning" | "partial" | "unsupported" | "none" | "inactive" | "activating" => {
            warning(label)
        }
        "failed" | "rejected" | "error" | "no" | "disabled" | "not found" | "dead" => {
            failure(label)
        }
        _ => label.to_string(),
    }
}

pub(super) fn value(text: impl AsRef<str>) -> String {
    paint(text, "1")
}

const fn validation_status_label(test: &TestResult) -> &'static str {
    match test.status {
        waytorandr_core::engine::ValidationStatus::Supported => "ok",
        waytorandr_core::engine::ValidationStatus::Rejected => "failed",
        waytorandr_core::engine::ValidationStatus::Unsupported => "unsupported",
    }
}

pub fn print_topology(title: &str, topology: &Topology) {
    println!("{}", heading(title));
    if topology.outputs.is_empty() {
        println!("  (no outputs detected)");
        return;
    }

    let mut outputs: Vec<_> = topology.outputs.iter().collect();
    outputs.sort_by(|a, b| a.0.cmp(b.0));

    for (name, state) in outputs {
        println!(
            "  {}: {} at ({},{}) scale {} mode {}",
            key(name),
            enabled_label(state.enabled),
            state.position.x,
            state.position.y,
            state.scale,
            format_mode(state.mode)
        );
        if let Some(description) = &state.identity.description {
            println!("    description: {description}");
        }
        if let Some(make) = &state.identity.make {
            println!("    make: {make}");
        }
        if let Some(model) = &state.identity.model {
            println!("    model: {model}");
        }
        if let Some(serial) = &state.identity.serial {
            println!("    serial: {serial}");
        }
    }
}

pub fn print_plan_summary(plan: &LayoutPlan) {
    let mut outputs: Vec<_> = plan.outputs.iter().collect();
    outputs.sort_by(|a, b| a.0.cmp(b.0));
    for (name, state) in outputs {
        println!(
            "  {} -> {} at ({},{}) scale {} mode {} transform {}{}",
            key(name),
            enabled_label(state.enabled),
            state.position.x,
            state.position.y,
            state.scale,
            format_mode(state.mode),
            state.transform,
            state
                .mirror_target
                .as_deref()
                .map_or_else(String::new, |target| format!(" mirror {target}"),),
        );
    }
}

pub fn print_validation_result(test: &Result<TestResult>) {
    match test {
        Ok(test) => {
            let label = validation_status_label(test);
            let message = test
                .message
                .as_deref()
                .map_or_else(String::new, |msg| format!(" ({msg})"));
            println!("Backend validation: {}{message}", status_label(label));
        }
        Err(err) => println!("Backend validation: {} ({err})", status_label("failed")),
    }
}

pub fn write_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

pub fn format_mode(mode: Option<Mode>) -> String {
    mode.map_or_else(
        || "no mode".to_string(),
        |mode| {
            format!(
                "{width}x{height}@{refresh}",
                width = mode.width,
                height = mode.height,
                refresh = mode.refresh
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_mode_handles_absent_mode() {
        assert_eq!(format_mode(None), "no mode");
    }

    #[test]
    fn validation_status_label_renders_unsupported() {
        let result = TestResult::unsupported(Some("no test mode".to_string()));

        assert_eq!(validation_status_label(&result), "unsupported");
    }

    #[test]
    fn paint_with_wraps_text_when_enabled() {
        assert_eq!(paint_with("ok", "32", true), "\x1b[32mok\x1b[0m");
    }

    #[test]
    fn paint_with_leaves_text_plain_when_disabled() {
        assert_eq!(paint_with("ok", "32", false), "ok");
    }

    #[test]
    fn color_enabled_for_disables_colors_for_dumb_terminal() {
        assert!(!color_enabled_for(true, Some("dumb"), false, None));
    }

    #[test]
    fn color_enabled_for_disables_colors_when_no_color_is_set() {
        assert!(!color_enabled_for(
            true,
            Some("xterm-256color"),
            true,
            Some("1")
        ));
    }

    #[test]
    fn color_enabled_for_honors_nonzero_force_color() {
        assert!(color_enabled_for(false, None, false, Some("1")));
    }

    #[test]
    fn color_enabled_for_ignores_zero_force_color() {
        assert!(!color_enabled_for(false, None, false, Some("0")));
    }
}
