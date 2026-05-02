use crate::error::{CoreError, CoreResult};
use fs4::FileExt;
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
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| CoreError::WriteFile {
            path: lock_path.clone(),
            source,
        })?;
    <File as FileExt>::lock(&lock_file).map_err(|source| CoreError::WriteFile {
        path: lock_path.clone(),
        source,
    })?;

    let result = action();
    let unlock_result =
        <File as FileExt>::unlock(&lock_file).map_err(|source| CoreError::WriteFile {
            path: lock_path,
            source,
        });

    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), _) | (Ok(_), Err(err)) => Err(err),
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
        .map_or_else(|| "waytorandr".into(), std::ffi::OsStr::to_os_string);
    name.push(".lock");
    path.with_file_name(name)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| "waytorandr".into(), std::ffi::OsStr::to_os_string);
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        atomic_write(&path, b"old\n").unwrap();
        atomic_write(&path, b"new\n").unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "new\n");
    }

    #[test]
    fn exclusive_lock_wraps_action_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let value = with_exclusive_lock(&path, || Ok(42)).unwrap();

        assert_eq!(value, 42);
        assert!(path.with_file_name("state.json.lock").exists());
    }
}
