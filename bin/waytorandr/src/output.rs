use anyhow::Result;

use waytorandr_core::engine::TestResult;
use waytorandr_core::model::Mode;
use waytorandr_core::planner::LayoutPlan;
use waytorandr_core::model::Topology;

pub(crate) fn print_topology(title: &str, topology: &Topology) {
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
            println!("    description: {}", description);
        }
        if let Some(make) = &state.identity.make {
            println!("    make: {}", make);
        }
        if let Some(model) = &state.identity.model {
            println!("    model: {}", model);
        }
        if let Some(serial) = &state.identity.serial {
            println!("    serial: {}", serial);
        }
    }
}

pub(crate) fn print_plan_summary(plan: &LayoutPlan) {
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
                .map(|target| format!(" mirror {}", target))
                .unwrap_or_default(),
        );
    }
}

pub(crate) fn print_validation_result(test: &Result<TestResult>) {
    match test {
        Ok(test) => println!(
            "Backend validation: {}{}",
            if test.success { "ok" } else { "failed" },
            test.message
                .as_deref()
                .map(|msg| format!(" ({msg})"))
                .unwrap_or_default()
        ),
        Err(err) => println!("Backend validation: failed ({})", err),
    }
}

pub(crate) fn format_mode(mode: Option<Mode>) -> String {
    mode.map(|mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh))
        .unwrap_or_else(|| "no mode".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use waytorandr_core::runtime::default_profile_for_setup;
    use waytorandr_core::store::State;

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
}
