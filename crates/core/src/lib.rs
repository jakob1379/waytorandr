//! waytorandr core library
//!
//! Provides the shared data model, profile management, matching, and planning
//! for Wayland display configuration.

mod atomic {
    use crate::error::{CoreError, CoreResult};
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn with_exclusive_lock<T>(
        path: &Path,
        action: impl FnOnce() -> CoreResult<T>,
    ) -> CoreResult<T> {
        let lock_path = lock_path(path);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .map_err(|source| CoreError::WriteFile {
                path: lock_path.clone(),
                source,
            })?;
        lock_file.lock().map_err(|source| CoreError::WriteFile {
            path: lock_path.clone(),
            source,
        })?;

        let result = action();
        let unlock_result = lock_file.unlock().map_err(|source| CoreError::WriteFile {
            path: lock_path,
            source,
        });

        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(err), _) => Err(err),
            (Ok(_), Err(err)) => Err(err),
        }
    }

    pub(crate) fn atomic_write(path: &Path, content: &[u8]) -> CoreResult<()> {
        let temp_path = temp_path_for(path);
        let write_result = write_temp_and_rename(path, &temp_path, content);

        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }

        write_result
    }

    fn write_temp_and_rename(path: &Path, temp_path: &Path, content: &[u8]) -> CoreResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp_path)
            .map_err(|source| CoreError::WriteFile {
                path: temp_path.to_path_buf(),
                source,
            })?;
        file.write_all(content)
            .map_err(|source| CoreError::WriteFile {
                path: temp_path.to_path_buf(),
                source,
            })?;
        file.sync_all().map_err(|source| CoreError::WriteFile {
            path: temp_path.to_path_buf(),
            source,
        })?;
        drop(file);

        fs::rename(temp_path, path).map_err(|source| CoreError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;

        if let Some(parent) = path.parent() {
            sync_dir(parent, path)?;
        }

        Ok(())
    }

    fn sync_dir(dir: &Path, path: &Path) -> CoreResult<()> {
        File::open(dir)
            .and_then(|dir| dir.sync_all())
            .map_err(|source| CoreError::WriteFile {
                path: path.to_path_buf(),
                source,
            })
    }

    fn lock_path(path: &Path) -> PathBuf {
        let mut name = path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_else(|| "waytorandr".into());
        name.push(".lock");
        path.with_file_name(name)
    }

    fn temp_path_for(path: &Path) -> PathBuf {
        let mut name = path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_else(|| "waytorandr".into());
        name.push(format!(
            ".tmp-{}-{}",
            std::process::id(),
            WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        path.with_file_name(name)
    }
}

pub mod engine;
pub mod error;
pub mod matcher;
pub mod model;
pub mod normalize;
pub mod planner;
pub mod profile;
pub mod state;
pub mod store;
pub mod terminal {
    #[must_use]
    pub fn escape_terminal_text(value: impl AsRef<str>) -> String {
        let mut escaped = String::new();
        for ch in value.as_ref().chars() {
            match ch {
                '\u{1b}' => escaped.push_str("\\e"),
                '\u{07}' => escaped.push_str("\\a"),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                '\t' => escaped.push_str("\\t"),
                ch if ch.is_control() => escaped.push_str(&format!("\\u{{{:04x}}}", ch as u32)),
                ch => escaped.push(ch),
            }
        }
        escaped
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn escapes_terminal_controls() {
            assert_eq!(
                escape_terminal_text("a\x1b]0;pwn\x07\r\nb"),
                "a\\e]0;pwn\\a\\r\\nb"
            );
        }

        #[test]
        fn keeps_printable_unicode() {
            assert_eq!(escape_terminal_text("Dell 日本"), "Dell 日本");
        }
    }
}
pub mod workflow;
