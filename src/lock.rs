//! Inter-process lock file support for shared Dinopod mutations.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_STALE_LOCK_AFTER: Duration = Duration::from_hours(1);

/// Exclusive lock represented by a create-new lock file.
#[derive(Debug)]
pub struct FileLock {
    path: PathBuf,
    _file: File,
}

impl FileLock {
    /// Attempts to acquire an exclusive lock.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for filesystem failures other than an already-held lock.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        Self::try_acquire_with_stale_after(path, SystemTime::now(), DEFAULT_STALE_LOCK_AFTER)
    }

    /// Attempts to acquire an exclusive lock, recovering locks older than `stale_after`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for filesystem failures other than an already-held,
    /// non-stale lock.
    pub fn try_acquire_with_stale_after(
        path: &Path,
        now: SystemTime,
        stale_after: Duration,
    ) -> io::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        match create_lock(path, now) {
            Ok(lock) => Ok(Some(lock)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if lock_is_stale(path, now, stale_after)? {
                    fs::remove_file(path)?;
                    create_lock(path, now).map(Some)
                } else {
                    Ok(None)
                }
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_lock(path: &Path, now: SystemTime) -> io::Result<FileLock> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    write!(
        file,
        "pid={}\ncreated_at_unix_seconds={}\n",
        std::process::id(),
        unix_seconds(now)?
    )?;
    Ok(FileLock {
        path: path.to_path_buf(),
        _file: file,
    })
}

fn lock_is_stale(path: &Path, now: SystemTime, stale_after: Duration) -> io::Result<bool> {
    let content = fs::read_to_string(path)?;
    let Some(created_at) = parse_created_at(&content) else {
        return Ok(false);
    };
    let created_at = UNIX_EPOCH + Duration::from_secs(created_at);
    Ok(now
        .duration_since(created_at)
        .is_ok_and(|age| age >= stale_after))
}

fn parse_created_at(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        line.strip_prefix("created_at_unix_seconds=")
            .and_then(|value| value.parse::<u64>().ok())
    })
}

fn unix_seconds(time: SystemTime) -> io::Result<u64> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}
