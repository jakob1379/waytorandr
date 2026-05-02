use std::time::Duration;

use anyhow::{Context, Result};
use zbus::blocking::{Connection, Proxy};

use waytorandr_core::LayoutPlan;
use waytorandr_core::{
    ApplyResult, Backend, ConfigFailureKind, OutputWatcher, PollingOutputWatcher, ValidationResult,
};
use waytorandr_core::{BackendConnectionError, CoreError, CoreResult};
use waytorandr_core::{BackendKind, Capabilities, Topology};

mod apply;
mod state;

#[cfg(test)]
use state::LogicalMonitorConfig;
use state::{
    export_current_state_topology as export_state_topology, CurrentState, CurrentStateReply,
    PropertyMap,
};

const DISPLAY_CONFIG_DESTINATION: &str = "org.gnome.Mutter.DisplayConfig";
const DISPLAY_CONFIG_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const DISPLAY_CONFIG_INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const METHOD_VERIFY: u32 = 0;
const METHOD_TEMPORARY: u32 = 1;

enum SubmitRequestError {
    Transport(anyhow::Error),
    Rejected(anyhow::Error),
}

enum SubmittedPlanRequest {
    Submitted,
    InvalidConfig(anyhow::Error),
    Rejected(anyhow::Error),
}

#[derive(Clone)]
pub struct GnomeBackend;

impl GnomeBackend {
    /// Connects to the GNOME backend.
    ///
    /// # Errors
    /// Returns an error if the session bus or Mutter `DisplayConfig` service is unavailable.
    pub fn connect() -> CoreResult<Self> {
        let backend = Self;
        Self::load_state()
            .context("failed to query Mutter DisplayConfig state")
            .map_err(|source| {
                CoreError::BackendConnection(BackendConnectionError::Initialize {
                    backend: BackendKind::Gnome.as_str(),
                    source,
                })
            })?;
        Ok(backend)
    }

    fn load_state() -> Result<CurrentState> {
        let connection = Connection::session().context("failed to connect to the session bus")?;
        let proxy = Proxy::new(
            &connection,
            DISPLAY_CONFIG_DESTINATION,
            DISPLAY_CONFIG_PATH,
            DISPLAY_CONFIG_INTERFACE,
        )
        .context("failed to create Mutter DisplayConfig proxy")?;
        let reply: CurrentStateReply = proxy
            .call("GetCurrentState", &())
            .context("failed to call GetCurrentState on org.gnome.Mutter.DisplayConfig")?;
        Ok(CurrentState::from_reply(reply))
    }

    fn topology_from_current_state(state: &CurrentState) -> Topology {
        export_state_topology(state)
    }

    fn submit_request(
        serial: u32,
        method: u32,
        logical_monitors: Vec<apply::ApplyLogicalMonitorTuple>,
        properties: PropertyMap,
    ) -> Result<(), SubmitRequestError> {
        let connection = Connection::session()
            .context("failed to connect to the session bus")
            .map_err(SubmitRequestError::Transport)?;
        let proxy = Proxy::new(
            &connection,
            DISPLAY_CONFIG_DESTINATION,
            DISPLAY_CONFIG_PATH,
            DISPLAY_CONFIG_INTERFACE,
        )
        .context("failed to create Mutter DisplayConfig proxy")
        .map_err(SubmitRequestError::Transport)?;
        let result: zbus::Result<()> = proxy.call(
            "ApplyMonitorsConfig",
            &(serial, method, logical_monitors, properties),
        );
        result.map_err(|source| {
            let rejected = matches!(source, zbus::Error::MethodError(..));
            let source = anyhow::Error::new(source)
                .context("failed to call ApplyMonitorsConfig on org.gnome.Mutter.DisplayConfig");
            if rejected {
                SubmitRequestError::Rejected(source)
            } else {
                SubmitRequestError::Transport(source)
            }
        })
    }

    fn submit_plan_request(plan: &LayoutPlan, method: u32) -> CoreResult<SubmittedPlanRequest> {
        let state = Self::load_state().map_err(|source| CoreError::Backend { source })?;
        let (serial, logical_monitors, properties) = match apply::build_apply_config(&state, plan) {
            Ok(request) => request,
            Err(source) => return Ok(SubmittedPlanRequest::InvalidConfig(source)),
        };

        match Self::submit_request(serial, method, logical_monitors, properties) {
            Ok(()) => Ok(SubmittedPlanRequest::Submitted),
            Err(SubmitRequestError::Rejected(source)) => Ok(SubmittedPlanRequest::Rejected(source)),
            Err(SubmitRequestError::Transport(source)) => Err(CoreError::Backend { source }),
        }
    }
}

impl Backend for GnomeBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Gnome);
        capabilities.can_validate = true;
        capabilities.supports_mirror = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        let state = Self::load_state().map_err(|source| CoreError::Backend { source })?;
        Ok(Self::topology_from_current_state(&state))
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        let initial = self.enumerate_outputs()?;
        Ok(Box::new(PollingOutputWatcher::new(
            self.clone(),
            POLL_INTERVAL,
            Some(initial.setup_fingerprint()),
        )))
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        match Self::submit_plan_request(plan, METHOD_VERIFY)? {
            SubmittedPlanRequest::Submitted => Ok(ValidationResult::supported(Some(format!(
                "GNOME validated {} output changes",
                plan.outputs.len()
            )))),
            SubmittedPlanRequest::InvalidConfig(source)
            | SubmittedPlanRequest::Rejected(source) => {
                Ok(validation_rejection_from_apply_error(&source))
            }
        }
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        match Self::submit_plan_request(plan, METHOD_TEMPORARY)? {
            SubmittedPlanRequest::Submitted => {
                let applied_state = self.enumerate_outputs()?;
                Ok(ApplyResult::applied(
                    Some("GNOME applied the configuration".to_string()),
                    Some(applied_state),
                ))
            }
            SubmittedPlanRequest::InvalidConfig(source)
            | SubmittedPlanRequest::Rejected(source) => Ok(apply_failure_from_apply_error(&source)),
        }
    }
}

fn classify_apply_failure(error: &anyhow::Error) -> ConfigFailureKind {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("stale")
        || message.contains("serial")
        || message.contains("changed")
        || message.contains("out of date")
    {
        ConfigFailureKind::TopologyChanged
    } else {
        ConfigFailureKind::Rejected
    }
}

fn validation_rejection_from_apply_error(source: &anyhow::Error) -> ValidationResult {
    ValidationResult::rejected(
        Some(classify_apply_failure(source)),
        Some(format!("GNOME rejected the configuration: {source:#}")),
    )
}

fn apply_failure_from_apply_error(source: &anyhow::Error) -> ApplyResult {
    ApplyResult::failed(
        Some(classify_apply_failure(source)),
        Some(format!(
            "GNOME failed to apply the configuration: {source:#}"
        )),
    )
}

#[cfg(test)]
mod tests;
