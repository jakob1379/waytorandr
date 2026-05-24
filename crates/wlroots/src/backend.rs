use std::convert::TryFrom;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use wayland_client::globals::{registry_queue_init, GlobalList};
use wayland_client::protocol::wl_output;
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1;
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_manager_v1::ZwlrOutputManagerV1;

use waytorandr_core::LayoutPlan;
use waytorandr_core::{ApplyResult, Backend, ConfigFailureKind, OutputWatcher, ValidationResult};
use waytorandr_core::{BackendConnectionError, CoreError, CoreResult};
use waytorandr_core::{BackendKind, Capabilities, Mode, OutputState, Topology, Transform};

mod protocol;

use protocol::{ConfigStatus, HeadInfo, State};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const SLOW_WATCH_REFRESH: Duration = Duration::from_secs(1);

enum SubmitError {
    Transport(anyhow::Error),
    Rejected(anyhow::Error),
}

pub struct WlrootsBackend {
    inner: Mutex<WaylandClient>,
}

struct WaylandClient {
    event_queue: EventQueue<State>,
    state: State,
}

struct WlrootsOutputWatcher {
    client: WaylandClient,
    interval: Duration,
    last_setup_fingerprint: Option<String>,
}

impl WlrootsBackend {
    /// Connects to the wlroots backend.
    ///
    /// # Errors
    /// Returns an error if the Wayland display or output-management protocol is unavailable.
    pub fn connect() -> CoreResult<Self> {
        Ok(Self {
            inner: Mutex::new(WaylandClient::connect()?),
        })
    }

    fn enumerate_live_topology(&self) -> CoreResult<Topology> {
        let mut inner = self.inner.lock().map_err(|_| CoreError::Backend {
            source: anyhow!("backend lock poisoned"),
        })?;
        inner
            .sync()
            .map_err(|source| CoreError::Backend { source })?;
        Ok(inner.state.to_topology())
    }

    fn snapshot_topology() -> CoreResult<Topology> {
        Self::connect()?.enumerate_live_topology()
    }
}

impl WlrootsOutputWatcher {
    fn connect(interval: Duration) -> CoreResult<Self> {
        let client = WaylandClient::connect()?;
        let initial = client.state.to_topology();
        Ok(Self {
            client,
            interval,
            last_setup_fingerprint: Some(initial.setup_fingerprint()),
        })
    }
}

impl OutputWatcher for WlrootsOutputWatcher {
    fn poll_changed(&mut self) -> CoreResult<Option<Topology>> {
        let sleep_start = Instant::now();
        thread::sleep(self.interval);
        let sleep_elapsed = sleep_start.elapsed();

        let refresh_start = Instant::now();
        self.client
            .sync()
            .map_err(|source| CoreError::Backend { source })?;
        if self.client.state.manager.is_none() {
            return Err(CoreError::Backend {
                source: anyhow!("wlroots output manager finished"),
            });
        }
        let refresh_elapsed = refresh_start.elapsed();

        let topology = self.client.state.to_topology();
        let setup_fingerprint = topology.setup_fingerprint();
        let changed = self.last_setup_fingerprint.as_ref() != Some(&setup_fingerprint);

        if changed {
            self.last_setup_fingerprint = Some(setup_fingerprint);
            tracing::debug!(
                sleep_ms = sleep_elapsed.as_millis(),
                refresh_ms = refresh_elapsed.as_millis(),
                setup_fingerprint = %topology.setup_fingerprint(),
                state_fingerprint = %topology.state_fingerprint(),
                "wlroots output watcher observed setup change"
            );
            return Ok(Some(topology));
        }

        if refresh_elapsed >= SLOW_WATCH_REFRESH {
            tracing::debug!(
                sleep_ms = sleep_elapsed.as_millis(),
                refresh_ms = refresh_elapsed.as_millis(),
                setup_fingerprint = %setup_fingerprint,
                "wlroots output watcher refresh was slow without setup change"
            );
        } else {
            tracing::trace!(
                sleep_ms = sleep_elapsed.as_millis(),
                refresh_ms = refresh_elapsed.as_millis(),
                setup_fingerprint = %setup_fingerprint,
                "wlroots output watcher refresh completed without setup change"
            );
        }

        Ok(None)
    }
}

impl WaylandClient {
    fn connect() -> CoreResult<Self> {
        let connection = Connection::connect_to_env()
            .context("failed to connect to Wayland display")
            .map_err(|source| {
                CoreError::BackendConnection(BackendConnectionError::Initialize {
                    backend: BackendKind::Wlroots.as_str(),
                    source,
                })
            })?;
        let (globals, event_queue) = registry_queue_init::<State>(&connection)
            .context("failed to initialize Wayland registry")
            .map_err(|source| {
                CoreError::BackendConnection(BackendConnectionError::Initialize {
                    backend: BackendKind::Wlroots.as_str(),
                    source,
                })
            })?;
        let qh = event_queue.handle();

        let manager = bind_manager(&globals, &qh).map_err(|source| {
            CoreError::BackendConnection(BackendConnectionError::Initialize {
                backend: BackendKind::Wlroots.as_str(),
                source,
            })
        })?;
        let mut client = WaylandClient {
            event_queue,
            state: State {
                manager: Some(manager),
                ..State::default()
            },
        };
        client.sync().map_err(|source| {
            CoreError::BackendConnection(BackendConnectionError::Initialize {
                backend: BackendKind::Wlroots.as_str(),
                source,
            })
        })?;
        client.sync().map_err(|source| {
            CoreError::BackendConnection(BackendConnectionError::Initialize {
                backend: BackendKind::Wlroots.as_str(),
                source,
            })
        })?;

        Ok(client)
    }
}

impl Backend for WlrootsBackend {
    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new(BackendKind::Wlroots);
        capabilities.can_validate = true;
        capabilities
    }

    fn enumerate_outputs(&self) -> CoreResult<Topology> {
        // Use a fresh output-manager snapshot for reads so stale head objects do
        // not leak into later polls after disconnect/reconfigure churn.
        Self::snapshot_topology()
    }

    fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
        Ok(Box::new(WlrootsOutputWatcher::connect(POLL_INTERVAL)?))
    }

    fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
        let mut inner = self.inner.lock().map_err(|_| CoreError::Backend {
            source: anyhow!("backend lock poisoned"),
        })?;
        let status = match inner.submit_with_retry(plan, true, 3) {
            Ok(status) => status,
            Err(SubmitError::Rejected(source)) => {
                return Ok(validation_rejection_from_submit_error(&source));
            }
            Err(SubmitError::Transport(source)) => return Err(CoreError::Backend { source }),
        };
        let message = Some(match status {
            ConfigStatus::Succeeded => {
                format!("wlroots validated {} output changes", plan.outputs.len())
            }
            ConfigStatus::Failed => "wlroots compositor rejected the configuration".to_string(),
            ConfigStatus::Cancelled => {
                "wlroots compositor cancelled the configuration because topology changed"
                    .to_string()
            }
        });
        Ok(match status {
            ConfigStatus::Succeeded => ValidationResult::supported(message),
            ConfigStatus::Failed | ConfigStatus::Cancelled => {
                ValidationResult::rejected(config_failure(status), message)
            }
        })
    }

    fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
        let status = {
            let mut inner = self.inner.lock().map_err(|_| CoreError::Backend {
                source: anyhow!("backend lock poisoned"),
            })?;
            match inner.submit_with_retry(plan, false, 3) {
                Ok(status) => status,
                Err(SubmitError::Rejected(source)) => {
                    return Ok(apply_failure_from_submit_error(&source));
                }
                Err(SubmitError::Transport(source)) => return Err(CoreError::Backend { source }),
            }
        };
        let message = Some(match status {
            ConfigStatus::Succeeded => "applied successfully".to_string(),
            ConfigStatus::Failed => "compositor rejected the configuration".to_string(),
            ConfigStatus::Cancelled => {
                "configuration cancelled because topology changed".to_string()
            }
        });
        if matches!(status, ConfigStatus::Succeeded) {
            Ok(ApplyResult::applied(
                message,
                Some(Self::snapshot_topology()?),
            ))
        } else {
            Ok(ApplyResult::failed(config_failure(status), message))
        }
    }
}

impl WaylandClient {
    fn sync(&mut self) -> Result<()> {
        self.event_queue
            .roundtrip(&mut self.state)
            .context("failed to roundtrip Wayland event queue")?;
        Ok(())
    }

    fn submit(&mut self, plan: &LayoutPlan, test_only: bool) -> Result<ConfigStatus, SubmitError> {
        let serial = self
            .state
            .serial
            .ok_or_else(|| {
                anyhow!("wlroots compositor did not provide an output-management serial")
            })
            .map_err(SubmitError::Transport)?;
        let manager = self
            .state
            .manager
            .as_ref()
            .ok_or_else(|| anyhow!("wlroots output manager is unavailable"))
            .map_err(SubmitError::Transport)?
            .clone();
        let qh = self.event_queue.handle();

        self.state.config_status = None;
        let configuration = manager.create_configuration(serial, &qh, ());

        for head in self.state.heads.values() {
            let Some(name) = head.name.as_deref() else {
                continue;
            };

            if let Some(desired) = plan.outputs.get(name) {
                if !desired.enabled {
                    configuration.disable_head(&head.head);
                    continue;
                }

                let conf_head = configuration.enable_head(&head.head, &qh, ());
                apply_head_config(&self.state, desired, head, &conf_head).map_err(|source| {
                    SubmitError::Rejected(
                        source.context(format!("wlroots rejected output `{name}`")),
                    )
                })?;
            } else {
                configuration.disable_head(&head.head);
            }
        }

        if test_only {
            configuration.test();
        } else {
            configuration.apply();
        }

        for _ in 0..5 {
            self.sync().map_err(SubmitError::Transport)?;
            if let Some(status) = self.state.config_status.take() {
                return Ok(status);
            }
        }

        Err(SubmitError::Transport(anyhow!(
            "wlroots compositor did not answer configuration request"
        )))
    }

    fn submit_with_retry(
        &mut self,
        plan: &LayoutPlan,
        test_only: bool,
        attempts: usize,
    ) -> Result<ConfigStatus, SubmitError> {
        let attempts = attempts.max(1);
        let mut attempt = 1;
        loop {
            let attempt_start = Instant::now();
            self.sync().map_err(SubmitError::Transport)?;
            let status = self.submit(plan, test_only)?;
            let elapsed = attempt_start.elapsed();
            if !matches!(status, ConfigStatus::Cancelled) || attempt == attempts {
                tracing::debug!(
                    attempt,
                    total_attempts = attempts,
                    elapsed_ms = elapsed.as_millis(),
                    test_only,
                    status = status.as_label(),
                    "wlroots configuration submission attempt completed"
                );
                return Ok(status);
            }

            tracing::warn!(
                attempt,
                total_attempts = attempts,
                elapsed_ms = elapsed.as_millis(),
                test_only,
                "wlroots configuration cancelled, retrying after refreshing compositor state"
            );
            attempt += 1;
        }
    }
}

fn bind_manager(globals: &GlobalList, qh: &QueueHandle<State>) -> Result<ZwlrOutputManagerV1> {
    globals
        .bind::<ZwlrOutputManagerV1, _, _>(qh, 1..=2, ())
        .map_err(|_| {
            anyhow!("wlroots output-management protocol is not available on this compositor")
        })
}

fn config_failure(status: ConfigStatus) -> Option<ConfigFailureKind> {
    match status {
        ConfigStatus::Succeeded => None,
        ConfigStatus::Failed => Some(ConfigFailureKind::Rejected),
        ConfigStatus::Cancelled => Some(ConfigFailureKind::TopologyChanged),
    }
}

fn validation_rejection_from_submit_error(source: &anyhow::Error) -> ValidationResult {
    ValidationResult::rejected(
        Some(ConfigFailureKind::Rejected),
        Some(format!("wlroots rejected the configuration: {source:#}")),
    )
}

fn apply_failure_from_submit_error(source: &anyhow::Error) -> ApplyResult {
    ApplyResult::failed(
        Some(ConfigFailureKind::Rejected),
        Some(format!(
            "wlroots failed to apply the configuration: {source:#}"
        )),
    )
}

fn apply_head_config(
    state: &State,
    desired: &OutputState,
    head: &HeadInfo,
    conf_head: &ZwlrOutputConfigurationHeadV1,
) -> Result<()> {
    validate_requested_head_config(desired)?;

    if let Some(mode) = desired.mode {
        if let Some(existing_mode) =
            head.modes
                .iter()
                .filter_map(|id| state.modes.get(id))
                .find(|candidate| {
                    candidate.width == Some(mode.width)
                        && candidate.height == Some(mode.height)
                        && candidate.refresh.unwrap_or(0) / 1000 == mode.refresh
                })
        {
            conf_head.set_mode(&existing_mode.mode);
        } else {
            let (width, height, refresh) = custom_mode_values(mode)?;
            conf_head.set_custom_mode(width, height, refresh);
        }
    }

    conf_head.set_position(desired.position.x, desired.position.y);
    conf_head.set_scale(desired.scale);
    conf_head.set_transform(transform_to_wl(desired.transform));

    if head.name.is_none() {
        bail!("attempted to configure unnamed output")
    }

    Ok(())
}

fn validate_requested_head_config(desired: &OutputState) -> Result<()> {
    if let Some(mode) = desired.mode {
        custom_mode_values(mode)?;
    }

    if desired.mirror_target.is_some() {
        bail!("wlroots mirroring is not implemented in this backend")
    }

    Ok(())
}

fn custom_mode_values(mode: Mode) -> Result<(i32, i32, i32)> {
    Ok((
        i32::try_from(mode.width).context("wlroots mode width does not fit in i32")?,
        i32::try_from(mode.height).context("wlroots mode height does not fit in i32")?,
        i32::try_from(mode.refresh.saturating_mul(1000))
            .context("wlroots mode refresh does not fit in i32")?,
    ))
}

fn transform_to_wl(transform: Transform) -> wl_output::Transform {
    match transform {
        Transform::Normal => wl_output::Transform::Normal,
        Transform::Rot90 => wl_output::Transform::_90,
        Transform::Rot180 => wl_output::Transform::_180,
        Transform::Rot270 => wl_output::Transform::_270,
        Transform::Flipped => wl_output::Transform::Flipped,
        Transform::Flipped90 => wl_output::Transform::Flipped90,
        Transform::Flipped180 => wl_output::Transform::Flipped180,
        Transform::Flipped270 => wl_output::Transform::Flipped270,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_failure_from_submit_error, custom_mode_values, validate_requested_head_config,
        validation_rejection_from_submit_error, ConfigStatus,
    };
    use waytorandr_core::{ConfigFailureKind, Mode, OutputState};

    #[test]
    fn mirror_target_is_structured_plan_rejection() -> anyhow::Result<()> {
        let mut desired = OutputState::new("HDMI-A-1");
        desired.mirror_target = Some("eDP-1".to_string());

        let Err(source) = validate_requested_head_config(&desired) else {
            anyhow::bail!("wlroots mirroring should be rejected before Wayland submission");
        };
        let validation = validation_rejection_from_submit_error(&source);

        assert_eq!(validation.failure(), Some(ConfigFailureKind::Rejected));
        assert_eq!(
            validation.message.as_deref(),
            Some(
                "wlroots rejected the configuration: wlroots mirroring is not implemented in this backend"
            )
        );
        Ok(())
    }

    #[test]
    fn invalid_custom_mode_is_structured_apply_failure() -> anyhow::Result<()> {
        let Err(source) = custom_mode_values(Mode {
            width: u32::MAX,
            height: 1080,
            refresh: 60,
        }) else {
            anyhow::bail!("oversized mode should be rejected before Wayland submission");
        };
        let result = apply_failure_from_submit_error(&source);

        assert_eq!(result.failure(), Some(ConfigFailureKind::Rejected));
        let message = result
            .message()
            .ok_or_else(|| anyhow::anyhow!("failure has context"))?;
        assert!(
            message.starts_with(
                "wlroots failed to apply the configuration: wlroots mode width does not fit in i32"
            ),
            "{message}"
        );
        Ok(())
    }

    #[test]
    fn config_status_labels_are_stable_for_debug_logs() {
        assert_eq!(ConfigStatus::Succeeded.as_label(), "succeeded");
        assert_eq!(ConfigStatus::Failed.as_label(), "failed");
        assert_eq!(ConfigStatus::Cancelled.as_label(), "cancelled");
    }
}
