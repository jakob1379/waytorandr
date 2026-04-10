use anyhow::Result;
use serde::Serialize;

use waytorandr_core::engine::TestResult;
use waytorandr_core::model::Mode;
use waytorandr_core::model::Topology;
use waytorandr_core::planner::LayoutPlan;

const fn validation_status_label(test: &TestResult) -> &'static str {
    match test.status {
        waytorandr_core::engine::ValidationStatus::Supported => "ok",
        waytorandr_core::engine::ValidationStatus::Rejected => "failed",
        waytorandr_core::engine::ValidationStatus::Unsupported => "unsupported",
    }
}

pub fn print_topology(title: &str, topology: &Topology) {
    println!("{title}");
    if topology.outputs.is_empty() {
        println!("  (no outputs detected)");
        return;
    }

    let mut outputs: Vec<_> = topology.outputs.iter().collect();
    outputs.sort_by(|a, b| a.0.cmp(b.0));

    for (name, state) in outputs {
        println!(
            "  {}: {} at ({},{}) scale {} mode {}",
            name,
            if state.enabled { "enabled" } else { "disabled" },
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
            name,
            if state.enabled { "enabled" } else { "disabled" },
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
            println!("Backend validation: {label}{message}");
        }
        Err(err) => println!("Backend validation: failed ({err})"),
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
    use waytorandr_core::state::State;
    use waytorandr_core::workflow::default_profile_for_setup;

    #[test]
    fn format_mode_handles_absent_mode() {
        assert_eq!(format_mode(None), "no mode");
    }

    #[test]
    fn default_profile_prefers_setup_specific_mapping() {
        let mut state = State::default();
        state.default_profiles = std::collections::HashMap::from([
            ("dock".to_string(), "desk".to_string()),
            (
                State::GLOBAL_DEFAULT_PROFILE_KEY.to_string(),
                "fallback".to_string(),
            ),
        ]);

        assert_eq!(default_profile_for_setup(&state, "dock"), Some("desk"));
    }

    #[test]
    fn validation_status_label_renders_unsupported() {
        let result = TestResult::unsupported(Some("no test mode".to_string()));

        assert_eq!(validation_status_label(&result), "unsupported");
    }
}
