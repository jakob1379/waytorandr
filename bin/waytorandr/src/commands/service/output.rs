use serde::Serialize;

use super::systemctl::ServiceStatus;
use crate::commands::output::{failure, key, status_label, success, warning, yes_no};

#[derive(Debug, Serialize)]
pub(super) struct JsonServiceActionResponse {
    pub(super) command: &'static str,
    pub(super) unit: &'static str,
    pub(super) installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unit_file_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) active_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sub_state: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct JsonServiceStatusResponse {
    pub(super) command: &'static str,
    pub(super) unit: &'static str,
    pub(super) installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unit_file_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) active_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sub_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) fragment_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) load_state: Option<String>,
}

pub(super) fn print_status_summary(status: &ServiceStatus) {
    println!("{}: {}", key("Service"), status.unit);
    println!("{}: {}", key("Installed"), yes_no(status.installed));
    if let Some(state) = &status.unit_file_state {
        println!("{}: {}", key("Enabled"), status_label(state));
    }
    if let Some(active) = &status.active_state {
        if let Some(sub) = &status.sub_state {
            println!(
                "{}: {} ({})",
                key("Active"),
                status_label(active),
                service_sub_state(active, sub)
            );
        } else {
            println!("{}: {}", key("Active"), status_label(active));
        }
    }
    if let Some(path) = &status.fragment_path {
        println!("{}: {path}", key("Unit file"));
    }
}

fn service_sub_state(active_state: &str, sub_state: &str) -> String {
    match (active_state, sub_state) {
        ("failed", _) | (_, "failed") => failure(sub_state),
        ("active", "running" | "listening" | "exited") => success(sub_state),
        _ => warning(sub_state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::service::UNIT_NAME;

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
    fn service_json_contracts_map_status_fields() {
        let status = status_fixture();

        let action = serde_json::to_value(JsonServiceActionResponse {
            command: "service-restart",
            unit: UNIT_NAME,
            installed: status.installed,
            path: status.fragment_path.clone(),
            unit_file_state: status.unit_file_state.clone(),
            active_state: status.active_state.clone(),
            sub_state: status.sub_state.clone(),
        })
        .expect("action response should serialize");
        assert_eq!(action["command"], "service-restart");
        assert_eq!(action["unit"], UNIT_NAME);
        assert_eq!(action["path"], "/tmp/waytorandrd.service");
        assert_eq!(action["active_state"], "active");

        let status_json = serde_json::to_value(JsonServiceStatusResponse {
            command: "service-status",
            unit: status.unit,
            installed: status.installed,
            unit_file_state: status.unit_file_state,
            active_state: status.active_state,
            sub_state: status.sub_state,
            fragment_path: status.fragment_path,
            load_state: status.load_state,
        })
        .expect("status response should serialize");
        assert_eq!(status_json["command"], "service-status");
        assert_eq!(status_json["unit_file_state"], "enabled");
        assert_eq!(status_json["load_state"], "loaded");
    }

    #[test]
    fn service_sub_state_classifies_terminal_states() {
        assert_eq!(service_sub_state("active", "running"), success("running"));
        assert_eq!(service_sub_state("inactive", "dead"), warning("dead"));
        assert_eq!(service_sub_state("active", "failed"), failure("failed"));
    }
}
