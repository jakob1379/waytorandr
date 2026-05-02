use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub(crate) fn xdg_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

pub(crate) fn scoped_env_var(
    key: &'static str,
    value: impl AsRef<std::ffi::OsStr>,
) -> ScopedEnvVar {
    let previous = std::env::var_os(key);
    std::env::set_var(key, value);
    ScopedEnvVar { key, previous }
}

pub(crate) struct XdgTestEnv {
    _root: tempfile::TempDir,
    _config_home: ScopedEnvVar,
    _state_home: ScopedEnvVar,
    state_home: PathBuf,
}

impl XdgTestEnv {
    pub(crate) fn new() -> std::io::Result<Self> {
        let root = tempfile::tempdir()?;
        let config_home = root.path().join("config");
        let state_home = root.path().join("state");
        std::fs::create_dir_all(&config_home)?;
        std::fs::create_dir_all(&state_home)?;

        Ok(Self {
            _root: root,
            _config_home: scoped_env_var("XDG_CONFIG_HOME", &config_home),
            _state_home: scoped_env_var("XDG_STATE_HOME", &state_home),
            state_home,
        })
    }

    pub(crate) fn state_file(&self) -> PathBuf {
        self.state_home.join("waytorandr").join("state.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_test_env_points_state_file_under_state_home() -> std::io::Result<()> {
        let _guard = xdg_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = XdgTestEnv::new()?;

        assert!(env.state_file().ends_with("waytorandr/state.toml"));
        Ok(())
    }
}
