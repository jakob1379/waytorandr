//! Backend selection for waytorandr.

// Cross-target dependency trees currently pull windows-sys through independent
// upstream paths that cannot be unified from this crate.
#![allow(clippy::multiple_crate_versions)]

use anyhow::anyhow;
#[cfg(all(debug_assertions, feature = "test-backend"))]
mod test_backend;

use waytorandr_core::Backend;
use waytorandr_core::BackendKind;
use waytorandr_core::{BackendConnectionAttempt, BackendConnectionError, CoreError, CoreResult};

/// Connects to the first available backend.
///
/// # Errors
/// Returns an error if no supported backend can be initialized.
pub fn connect_backend() -> CoreResult<Box<dyn Backend>> {
    #[cfg(all(debug_assertions, feature = "test-backend"))]
    if let Some(backend) = test_backend::connect() {
        return Ok(Box::new(backend));
    }

    let env = SessionEnvironment::from_process();
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".to_string());
    let xdg_runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "<unset>".to_string());
    let display_hint = if wayland_display.contains('/') {
        "; WAYLAND_DISPLAY should be a socket name like 'wayland-0', not a path"
    } else {
        ""
    };
    let mut attempts = Vec::new();

    for backend in backends_for_process(&env)? {
        let result = connect_backend_kind(backend);

        match result {
            Ok(backend) => return Ok(backend),
            Err(
                err @ CoreError::BackendConnection(BackendConnectionError::UnknownBackendLabel {
                    ..
                }),
            ) => return Err(err),
            Err(err) => attempts.push(BackendConnectionAttempt::new(
                backend.as_str(),
                anyhow!(err),
            )),
        }
    }

    Err(CoreError::BackendConnection(
        BackendConnectionError::NoSupportedBackend {
            wayland_display,
            xdg_runtime_dir,
            display_hint: display_hint.to_string(),
            attempts,
        },
    ))
}

const BACKEND_OVERRIDE_ENV: &str = "WAYTORANDR_BACKEND";

#[derive(Clone, Copy)]
struct BackendDescriptor {
    kind: BackendKind,
    aliases: &'static [&'static str],
    gnome_priority: u8,
    kde_priority: u8,
    fallback_priority: u8,
    connect: fn() -> CoreResult<Box<dyn Backend>>,
}

impl BackendDescriptor {
    fn priority_for(self, env: &SessionEnvironment) -> u8 {
        if env.is_kde_session() {
            self.kde_priority
        } else if env.is_gnome_session() {
            self.gnome_priority
        } else {
            self.fallback_priority
        }
    }

    fn accepts_override(self, value: &str) -> bool {
        self.aliases.contains(&value)
    }
}

static PRODUCTION_BACKENDS: &[BackendDescriptor] = &[
    #[cfg(feature = "gnome")]
    BackendDescriptor {
        kind: BackendKind::Gnome,
        aliases: &["gnome"],
        gnome_priority: 0,
        kde_priority: 2,
        fallback_priority: 2,
        connect: connect_gnome_backend,
    },
    #[cfg(feature = "kscreen")]
    BackendDescriptor {
        kind: BackendKind::KScreen,
        aliases: &["kscreen", "kde"],
        gnome_priority: 2,
        kde_priority: 0,
        fallback_priority: 1,
        connect: connect_kscreen_backend,
    },
    #[cfg(feature = "wlroots")]
    BackendDescriptor {
        kind: BackendKind::Wlroots,
        aliases: &["wlroots", "wlr"],
        gnome_priority: 1,
        kde_priority: 1,
        fallback_priority: 0,
        connect: connect_wlroots_backend,
    },
];

#[cfg(feature = "gnome")]
fn connect_gnome_backend() -> CoreResult<Box<dyn Backend>> {
    waytorandr_gnome::backend::GnomeBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

#[cfg(feature = "kscreen")]
fn connect_kscreen_backend() -> CoreResult<Box<dyn Backend>> {
    waytorandr_kscreen::backend::KScreenBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

#[cfg(feature = "wlroots")]
fn connect_wlroots_backend() -> CoreResult<Box<dyn Backend>> {
    waytorandr_wlroots::backend::WlrootsBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

fn backend_descriptor(backend: BackendKind) -> Option<&'static BackendDescriptor> {
    PRODUCTION_BACKENDS
        .iter()
        .find(|descriptor| descriptor.kind == backend)
}

fn connect_backend_kind(backend: BackendKind) -> CoreResult<Box<dyn Backend>> {
    match backend_descriptor(backend) {
        Some(descriptor) => (descriptor.connect)(),
        None => Err(CoreError::BackendConnection(
            BackendConnectionError::UnknownBackendLabel {
                label: backend.as_str().to_string(),
            },
        )),
    }
}

fn backends_for_env(env: &SessionEnvironment) -> Vec<BackendKind> {
    let mut descriptors = PRODUCTION_BACKENDS.to_vec();
    descriptors.sort_by_key(|descriptor| descriptor.priority_for(env));
    descriptors
        .into_iter()
        .map(|descriptor| descriptor.kind)
        .collect()
}

fn backends_for_process(env: &SessionEnvironment) -> CoreResult<Vec<BackendKind>> {
    match std::env::var(BACKEND_OVERRIDE_ENV) {
        Ok(value) => parse_backend_override(&value)
            .map(|backend| vec![backend])
            .ok_or_else(|| {
                CoreError::BackendConnection(BackendConnectionError::UnknownBackendLabel {
                    label: value,
                })
            }),
        Err(_) => Ok(backends_for_env(env)),
    }
}

fn parse_backend_override(value: &str) -> Option<BackendKind> {
    let value = value.trim().to_ascii_lowercase();
    PRODUCTION_BACKENDS
        .iter()
        .find(|descriptor| descriptor.accepts_override(&value))
        .map(|descriptor| descriptor.kind)
}

#[derive(Clone, Debug, Default)]
struct SessionEnvironment {
    current_desktop: Option<String>,
    session_desktop: Option<String>,
    desktop_session: Option<String>,
}

impl SessionEnvironment {
    fn from_process() -> Self {
        Self {
            current_desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
            session_desktop: std::env::var("XDG_SESSION_DESKTOP").ok(),
            desktop_session: std::env::var("DESKTOP_SESSION").ok(),
        }
    }

    fn is_gnome_session(&self) -> bool {
        self.values().any(|value| value.contains("gnome"))
    }

    fn is_kde_session(&self) -> bool {
        self.values()
            .any(|value| value.contains("kde") || value.contains("plasma"))
    }

    fn values(&self) -> impl Iterator<Item = String> + '_ {
        [
            self.current_desktop.as_deref(),
            self.session_desktop.as_deref(),
            self.desktop_session.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_ascii_lowercase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_sessions_prefer_gnome_backend() {
        let env = SessionEnvironment {
            current_desktop: Some("GNOME".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backends_for_env(&env),
            vec![
                BackendKind::Gnome,
                BackendKind::Wlroots,
                BackendKind::KScreen
            ]
        );
    }

    #[test]
    fn kde_sessions_prefer_kscreen_backend() {
        let env = SessionEnvironment {
            session_desktop: Some("plasma".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backends_for_env(&env),
            vec![
                BackendKind::KScreen,
                BackendKind::Wlroots,
                BackendKind::Gnome
            ]
        );
    }

    #[test]
    fn unknown_sessions_prefer_wlroots_backend() {
        let env = SessionEnvironment {
            desktop_session: Some("niri".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backends_for_env(&env),
            vec![
                BackendKind::Wlroots,
                BackendKind::KScreen,
                BackendKind::Gnome
            ]
        );
    }

    #[test]
    fn backend_override_accepts_known_labels() {
        assert_eq!(parse_backend_override("gnome"), Some(BackendKind::Gnome));
        assert_eq!(parse_backend_override("KDE"), Some(BackendKind::KScreen));
        assert_eq!(parse_backend_override("wlr"), Some(BackendKind::Wlroots));
    }

    #[test]
    fn backend_override_rejects_unknown_labels() {
        assert_eq!(parse_backend_override("sway"), None);
    }
}
