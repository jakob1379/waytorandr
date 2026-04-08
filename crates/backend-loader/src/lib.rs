use anyhow::{anyhow, Result};

use waytorandr_core::engine::Backend;

pub fn connect_backend() -> Result<Box<dyn Backend>> {
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
    let mut errors = Vec::new();

    for label in backend_labels_for_env(&env) {
        let result = match label {
            "gnome" => connect_gnome(),
            "kscreen" => connect_kscreen(),
            "wlroots" => connect_wlroots(),
            other => Err(anyhow!("unknown backend label `{other}`")),
        };

        match result {
            Ok(backend) => return Ok(backend),
            Err(err) => errors.push(format!("{label}: {err}")),
        }
    }

    Err(anyhow!(
        "failed to connect to a supported backend (WAYLAND_DISPLAY={wayland_display}, XDG_RUNTIME_DIR={xdg_runtime_dir}{display_hint}); attempts: {}",
        errors.join("; ")
    ))
}

fn connect_gnome() -> Result<Box<dyn Backend>> {
    waytorandr_gnome::backend::GnomeBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

fn connect_kscreen() -> Result<Box<dyn Backend>> {
    waytorandr_kscreen::backend::KScreenBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

fn connect_wlroots() -> Result<Box<dyn Backend>> {
    waytorandr_wlroots::backend::WlrootsBackend::connect()
        .map(|backend| Box::new(backend) as Box<dyn Backend>)
}

fn backend_labels_for_env(env: &SessionEnvironment) -> Vec<&'static str> {
    if env.is_kde_session() {
        vec!["kscreen", "wlroots", "gnome"]
    } else if env.is_gnome_session() {
        vec!["gnome", "wlroots", "kscreen"]
    } else {
        vec!["wlroots", "kscreen", "gnome"]
    }
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
        .map(|value| value.to_ascii_lowercase())
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
            backend_labels_for_env(&env),
            vec!["gnome", "wlroots", "kscreen"]
        );
    }

    #[test]
    fn kde_sessions_prefer_kscreen_backend() {
        let env = SessionEnvironment {
            session_desktop: Some("plasma".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backend_labels_for_env(&env),
            vec!["kscreen", "wlroots", "gnome"]
        );
    }

    #[test]
    fn unknown_sessions_prefer_wlroots_backend() {
        let env = SessionEnvironment {
            desktop_session: Some("niri".to_string()),
            ..SessionEnvironment::default()
        };

        assert_eq!(
            backend_labels_for_env(&env),
            vec!["wlroots", "kscreen", "gnome"]
        );
    }
}
