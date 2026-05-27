//! Best-effort inter-process guard for shared Dinopod mutations.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_STALE_GUARD_AFTER: Duration = Duration::from_hours(1);

static GUARD_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Best-effort exclusive guard represented by a create-new guard file.
///
/// This is not a kernel advisory lock. Two processes can still race during stale
/// recovery. It prevents most accidental concurrent mutations from a single machine.
#[derive(Debug)]
pub struct MutationGuard {
    path: PathBuf,
    token: u64,
    _file: File,
}

impl MutationGuard {
    /// Attempts to acquire an exclusive guard.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for filesystem failures other than an already-held guard.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        Self::try_acquire_with_stale_after(path, SystemTime::now(), DEFAULT_STALE_GUARD_AFTER)
    }

    /// Attempts to acquire an exclusive guard, recovering guards older than `stale_after`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for filesystem failures other than an already-held,
    /// non-stale guard.
    pub fn try_acquire_with_stale_after(
        path: &Path,
        now: SystemTime,
        stale_after: Duration,
    ) -> io::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        match create_guard(path, now) {
            Ok(guard) => Ok(Some(guard)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if guard_is_stale(path, now, stale_after)? {
                    fs::remove_file(path)?;
                    create_guard(path, now).map(Some)
                } else {
                    Ok(None)
                }
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        if let Ok(content) = fs::read_to_string(&self.path) {
            if parse_token(&content) == Some(self.token) {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

fn create_guard(path: &Path, now: SystemTime) -> io::Result<MutationGuard> {
    let token = GUARD_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    write!(
        file,
        "pid={}\ncreated_at_unix_seconds={}\ntoken={}\n",
        std::process::id(),
        unix_seconds(now)?,
        token
    )?;
    Ok(MutationGuard {
        path: path.to_path_buf(),
        token,
        _file: file,
    })
}

fn guard_is_stale(path: &Path, now: SystemTime, stale_after: Duration) -> io::Result<bool> {
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

fn parse_token(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        line.strip_prefix("token=")
            .and_then(|value| value.parse::<u64>().ok())
    })
}

fn unix_seconds(time: SystemTime) -> io::Result<u64> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}
