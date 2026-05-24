use std::time::{Duration, Instant};

use anyhow::Result;
use waytorandr_backend_loader::connect_backend;
use waytorandr_core::workflow;
use waytorandr_core::{
    Backend, BackendKind, Capabilities, OutputWatcher, ProfileStore, StateStore, Topology,
};

use super::{duration_ms, elapsed_ms, enforce_topology_policy, record_daemon_started};

const INITIAL_POLICY_CONTEXT: &str = "initial";
const RECONNECTED_POLICY_CONTEXT: &str = "after_reconnect";
const SLOW_WATCH_POLL: Duration = Duration::from_secs(1);

pub(crate) fn run_watch_loop(
    backend: &mut Box<dyn Backend>,
    store: &ProfileStore,
    state_store: &StateStore,
    reconnect_interval: Duration,
    no_hooks: bool,
) -> Result<()> {
    let mut capabilities = backend.capabilities();
    let mut watcher = backend.watch_outputs()?;

    record_daemon_started(state_store, capabilities.backend)?;
    enforce_initial_policy(
        backend.as_ref(),
        store,
        state_store,
        INITIAL_POLICY_CONTEXT,
        no_hooks,
    );

    tracing::info!(backend = %capabilities.backend, "daemon ready, watching outputs");

    loop {
        run_watch_step(
            backend,
            store,
            state_store,
            &mut capabilities,
            &mut watcher,
            &mut |backend| reconnect_backend(backend, reconnect_interval),
            no_hooks,
        )?;
    }
}

fn run_watch_step(
    backend: &mut Box<dyn Backend>,
    store: &ProfileStore,
    state_store: &StateStore,
    capabilities: &mut Capabilities,
    watcher: &mut Box<dyn OutputWatcher>,
    reconnect: &mut impl FnMut(&mut Box<dyn Backend>) -> Result<(Capabilities, Box<dyn OutputWatcher>)>,
    no_hooks: bool,
) -> Result<()> {
    let poll_start = Instant::now();
    match watcher.poll_changed() {
        Ok(Some(topology)) => {
            tracing::debug!(
                elapsed_ms = elapsed_ms(poll_start),
                backend = %capabilities.backend,
                setup_fingerprint = %topology.setup_fingerprint(),
                state_fingerprint = %topology.state_fingerprint(),
                "daemon output watcher detected setup change"
            );
            handle_topology_change(
                backend.as_ref(),
                store,
                state_store,
                capabilities.backend,
                &topology,
                no_hooks,
            )
        }
        Ok(None) => {
            let poll_elapsed = poll_start.elapsed();
            if poll_elapsed >= SLOW_WATCH_POLL {
                tracing::debug!(
                    elapsed_ms = duration_ms(poll_elapsed),
                    backend = %capabilities.backend,
                    "daemon output watcher poll was slow without setup change"
                );
            } else {
                tracing::trace!(
                    elapsed_ms = duration_ms(poll_elapsed),
                    backend = %capabilities.backend,
                    "daemon output watcher poll completed without setup change"
                );
            }
            Ok(())
        }
        Err(err) => {
            tracing::warn!(
                elapsed_ms = elapsed_ms(poll_start),
                error = %err,
                "output watcher failed; reconnecting backend"
            );
            let (next_capabilities, next_watcher) = reconnect(backend)?;
            *capabilities = next_capabilities;
            *watcher = next_watcher;
            record_daemon_started(state_store, capabilities.backend)?;
            tracing::info!(backend = %capabilities.backend, "backend reconnected");
            enforce_initial_policy(
                backend.as_ref(),
                store,
                state_store,
                RECONNECTED_POLICY_CONTEXT,
                no_hooks,
            );
            Ok(())
        }
    }
}

fn handle_topology_change(
    backend: &(impl Backend + ?Sized),
    store: &ProfileStore,
    state_store: &StateStore,
    backend_kind: BackendKind,
    topology: &Topology,
    no_hooks: bool,
) -> Result<()> {
    let change_start = Instant::now();
    workflow::persist_observed_runtime_state(state_store, Some(backend_kind), topology)?;
    tracing::info!(fingerprint = %topology.fingerprint(), "topology changed");
    enforce_initial_policy(
        backend,
        store,
        state_store,
        INITIAL_POLICY_CONTEXT,
        no_hooks,
    );
    tracing::debug!(
        elapsed_ms = elapsed_ms(change_start),
        backend = %backend_kind,
        setup_fingerprint = %topology.setup_fingerprint(),
        state_fingerprint = %topology.state_fingerprint(),
        "daemon topology change handling completed"
    );
    Ok(())
}

fn enforce_initial_policy(
    backend: &(impl Backend + ?Sized),
    store: &ProfileStore,
    state_store: &StateStore,
    policy_context: &'static str,
    no_hooks: bool,
) {
    let policy_start = Instant::now();
    if let Err(err) = enforce_topology_policy(backend, store, state_store, no_hooks) {
        tracing::error!(
            elapsed_ms = elapsed_ms(policy_start),
            error = %err,
            context = policy_context,
            "daemon topology policy failed"
        );
    } else {
        tracing::debug!(
            elapsed_ms = elapsed_ms(policy_start),
            context = policy_context,
            "daemon topology policy enforcement returned"
        );
    }
}

fn reconnect_backend(
    backend: &mut Box<dyn Backend>,
    reconnect_interval: Duration,
) -> Result<(Capabilities, Box<dyn OutputWatcher>)> {
    loop {
        let reconnect_start = Instant::now();
        std::thread::sleep(reconnect_interval);
        match connect_backend().and_then(|next_backend| {
            let next_capabilities = next_backend.capabilities();
            let next_watcher = next_backend.watch_outputs()?;
            Ok((next_backend, next_capabilities, next_watcher))
        }) {
            Ok((next_backend, next_capabilities, next_watcher)) => {
                tracing::debug!(
                    elapsed_ms = elapsed_ms(reconnect_start),
                    backend = %next_capabilities.backend,
                    "daemon backend reconnect completed"
                );
                *backend = next_backend;
                return Ok((next_capabilities, next_watcher));
            }
            Err(err) => {
                tracing::warn!(
                    elapsed_ms = elapsed_ms(reconnect_start),
                    error = %err,
                    "backend reconnect failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::MutexGuard;
    use waytorandr_core::{
        ApplyResult, CoreError, CoreResult, LayoutPlan, OutputState, Position, ValidationResult,
    };

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn policy_contexts_distinguish_initial_and_reconnect_paths() {
        assert_eq!(INITIAL_POLICY_CONTEXT, "initial");
        assert_eq!(RECONNECTED_POLICY_CONTEXT, "after_reconnect");
    }

    #[test]
    fn watch_step_persists_changed_topology() {
        let _guard = env_guard();
        let env = TestEnv::new();
        let state_store = StateStore::bootstrap().expect("bootstrap state store");
        let store = ProfileStore::bootstrap().expect("bootstrap profile store");
        let topology = watch_topology("DP-1");
        let mut backend = boxed_backend(BackendKind::Test, topology.clone());
        let mut capabilities = Capabilities::new(BackendKind::Test);
        let mut watcher = boxed_watcher(Ok(Some(topology.clone())));

        run_watch_step(
            &mut backend,
            &store,
            &state_store,
            &mut capabilities,
            &mut watcher,
            &mut |_| panic!("reconnect should not run"),
            false,
        )
        .expect("watch step");

        let state = state_store
            .load_state()
            .expect("load state")
            .expect("state should be persisted");
        assert_eq!(
            state.last_topology_fingerprint,
            Some(topology.fingerprint())
        );
        assert!(env.state_file().exists());
    }

    #[test]
    fn watch_step_ignores_empty_poll() {
        let _guard = env_guard();
        let env = TestEnv::new();
        let state_store = StateStore::bootstrap().expect("bootstrap state store");
        let store = ProfileStore::bootstrap().expect("bootstrap profile store");
        let mut backend = boxed_backend(BackendKind::Test, watch_topology("DP-1"));
        let mut capabilities = Capabilities::new(BackendKind::Test);
        let mut watcher = boxed_watcher(Ok(None));

        run_watch_step(
            &mut backend,
            &store,
            &state_store,
            &mut capabilities,
            &mut watcher,
            &mut |_| panic!("reconnect should not run"),
            false,
        )
        .expect("watch step");

        assert!(!env.state_file().exists());
    }

    #[test]
    fn watch_step_reconnects_and_records_daemon_start() {
        let _guard = env_guard();
        let _env = TestEnv::new();
        let state_store = StateStore::bootstrap().expect("bootstrap state store");
        let store = ProfileStore::bootstrap().expect("bootstrap profile store");
        let mut backend = boxed_backend(BackendKind::Test, watch_topology("DP-1"));
        let mut capabilities = Capabilities::new(BackendKind::Test);
        let mut watcher = boxed_watcher(Err(CoreError::Backend {
            source: anyhow!("watch failed"),
        }));

        run_watch_step(
            &mut backend,
            &store,
            &state_store,
            &mut capabilities,
            &mut watcher,
            &mut |backend| {
                *backend = boxed_backend(BackendKind::Wlroots, watch_topology("HDMI-A-1"));
                Ok((
                    Capabilities::new(BackendKind::Wlroots),
                    boxed_watcher(Ok(None)),
                ))
            },
            false,
        )
        .expect("watch step");

        assert_eq!(capabilities.backend, BackendKind::Wlroots);
        let state = state_store
            .load_state()
            .expect("load state")
            .expect("daemon start should be recorded");
        assert!(state.daemon_enabled);
        assert_eq!(state.backend, Some(BackendKind::Wlroots));
    }

    #[test]
    fn watch_step_propagates_reconnect_failure() {
        let _guard = env_guard();
        let _env = TestEnv::new();
        let state_store = StateStore::bootstrap().expect("bootstrap state store");
        let store = ProfileStore::bootstrap().expect("bootstrap profile store");
        let mut backend = boxed_backend(BackendKind::Test, watch_topology("DP-1"));
        let mut capabilities = Capabilities::new(BackendKind::Test);
        let mut watcher = boxed_watcher(Err(CoreError::Backend {
            source: anyhow!("watch failed"),
        }));

        let err = run_watch_step(
            &mut backend,
            &store,
            &state_store,
            &mut capabilities,
            &mut watcher,
            &mut |_| Err(anyhow!("reconnect failed")),
            false,
        )
        .expect_err("reconnect failure should propagate");

        assert!(err.to_string().contains("reconnect failed"));
    }

    fn boxed_backend(backend: BackendKind, topology: Topology) -> Box<dyn Backend> {
        Box::new(TestBackend { backend, topology })
    }

    fn boxed_watcher(next: CoreResult<Option<Topology>>) -> Box<dyn OutputWatcher> {
        Box::new(TestWatcher { next: Some(next) })
    }

    fn watch_topology(name: &str) -> Topology {
        let mut output = OutputState::new(name);
        output.enabled = true;
        output.position = Position::new(0, 0);
        Topology {
            outputs: HashMap::from([(name.to_string(), output)]),
        }
    }

    struct TestBackend {
        backend: BackendKind,
        topology: Topology,
    }

    impl Backend for TestBackend {
        fn capabilities(&self) -> Capabilities {
            Capabilities::new(self.backend)
        }

        fn enumerate_outputs(&self) -> CoreResult<Topology> {
            Ok(self.topology.clone())
        }

        fn watch_outputs(&self) -> CoreResult<Box<dyn OutputWatcher>> {
            Ok(boxed_watcher(Ok(None)))
        }

        fn validate(&self, plan: &LayoutPlan) -> CoreResult<ValidationResult> {
            let _ = plan;
            Ok(ValidationResult::unsupported(None))
        }

        fn apply(&self, plan: &LayoutPlan) -> CoreResult<ApplyResult> {
            let _ = plan;
            Ok(ApplyResult::failed(None, None))
        }
    }

    struct TestWatcher {
        next: Option<CoreResult<Option<Topology>>>,
    }

    impl OutputWatcher for TestWatcher {
        fn poll_changed(&mut self) -> CoreResult<Option<Topology>> {
            self.next.take().expect("watcher called once")
        }
    }

    fn env_guard() -> MutexGuard<'static, ()> {
        super::super::xdg_test_guard()
    }

    struct TestEnv {
        temp: PathBuf,
        _config_home: ScopedEnvVar,
        state_home: ScopedEnvVar,
    }

    impl TestEnv {
        fn new() -> Self {
            let temp = std::env::temp_dir().join(format!(
                "waytorandrd-watch-test-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&temp).expect("create tempdir");
            let config_home = ScopedEnvVar::set("XDG_CONFIG_HOME", temp.join("config"));
            let state_home = ScopedEnvVar::set("XDG_STATE_HOME", temp.join("state"));
            Self {
                temp,
                _config_home: config_home,
                state_home,
            }
        }

        fn state_file(&self) -> PathBuf {
            self.state_home.path().join("waytorandr").join("state.toml")
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.temp);
        }
    }

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        value: PathBuf,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: PathBuf) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, &value);
            Self {
                key,
                previous,
                value,
            }
        }

        fn path(&self) -> &Path {
            &self.value
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
