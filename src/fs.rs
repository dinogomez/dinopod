//! Atomic file write helpers for generated Dinopod artifacts.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Minimal filesystem operations needed for atomic writes.
pub trait AtomicFileSystem {
    /// Writes `contents` to `path`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the write fails.
    fn write_file(&mut self, path: &Path, contents: &str) -> io::Result<()>;

    /// Renames `from` to `to`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the rename fails.
    fn rename_file(&mut self, from: &Path, to: &Path) -> io::Result<()>;

    /// Removes `path`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when removal fails.
    fn remove_file(&mut self, path: &Path) -> io::Result<()>;
}

impl<T> AtomicFileSystem for &mut T
where
    T: AtomicFileSystem + ?Sized,
{
    fn write_file(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        (**self).write_file(path, contents)
    }

    fn rename_file(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        (**self).rename_file(from, to)
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        (**self).remove_file(path)
    }
}

/// Production filesystem adapter for atomic writes.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdAtomicFileSystem;

impl AtomicFileSystem for StdAtomicFileSystem {
    fn write_file(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }

    fn rename_file(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Writes files through a temp-file-plus-rename sequence.
#[derive(Debug)]
pub struct AtomicWriter<F> {
    fs: F,
}

impl<F> AtomicWriter<F>
where
    F: AtomicFileSystem,
{
    /// Creates an atomic writer.
    #[must_use]
    pub fn new(fs: F) -> Self {
        Self { fs }
    }

    /// Writes `contents` atomically to `path`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when writing the temp file, renaming it, or cleaning it
    /// up fails.
    pub fn write_atomic(&mut self, path: &Path, contents: &str) -> io::Result<()> {
        let temp_path = temp_path_for(path);
        self.fs.write_file(&temp_path, contents)?;

        match self.fs.rename_file(&temp_path, path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                self.fs.remove_file(path)?;
                self.fs.rename_file(&temp_path, path)
            }
            Err(error) => {
                self.fs.remove_file(&temp_path)?;
                Err(error)
            }
        }
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path.file_name().map_or_else(
        || ".dinopod.tmp".to_owned(),
        |name| format!("{}.tmp", name.to_string_lossy()),
    );
    path.with_file_name(file_name)
}
