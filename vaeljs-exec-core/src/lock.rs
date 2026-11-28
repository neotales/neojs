//! Cross-platform OS advisory file locking built on safe `std::fs::File` locking.
//!
//! Uses Rust 1.100.0's stabilized `File::{lock, lock_shared, try_lock, try_lock_shared, unlock}`
//! with zero `unsafe` code, and rejects symbolic-link / reparse-point targets.

use std::borrow::Cow;
use std::fs::{self, File, Metadata, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{ExecError, map_io_error};
use crate::types::{validate_path_not_empty_or_nul, validate_positive_duration};

/// Advisory lock sharing mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum LockMode {
    /// Shared reader lock (multiple concurrent shared locks allowed; blocks exclusive locks).
    Shared,
    /// Exclusive writer lock (blocks all other shared and exclusive locks).
    Exclusive,
}

/// Configuration for [`FileLock::acquire`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockOptions {
    /// Lock sharing mode (default [`LockMode::Exclusive`]).
    pub mode: LockMode,
    /// Create the target file if it does not already exist (default `true`).
    pub create: bool,
    /// Fail immediately with [`crate::ExecErrorCode::LockWouldBlock`] if contended (default `false`).
    pub non_blocking: bool,
    /// Optional bounded wait duration; returns [`crate::ExecErrorCode::LockTimeout`] on expiry.
    pub timeout: Option<Duration>,
}

impl Default for LockOptions {
    fn default() -> Self {
        Self {
            mode: LockMode::Exclusive,
            create: true,
            non_blocking: false,
            timeout: None,
        }
    }
}

impl LockOptions {
    /// Creates [`LockOptions`] with the given [`LockMode`] and default flags
    /// (`create: true`, `non_blocking: false`, `timeout: None`).
    #[must_use]
    pub const fn new(mode: LockMode) -> Self {
        Self {
            mode,
            create: true,
            non_blocking: false,
            timeout: None,
        }
    }

    /// Sets whether the lock file is created if missing.
    #[must_use]
    pub const fn create(mut self, create: bool) -> Self {
        self.create = create;
        self
    }

    /// Sets whether lock acquisition returns immediately if contended.
    #[must_use]
    pub const fn non_blocking(mut self, non_blocking: bool) -> Self {
        self.non_blocking = non_blocking;
        self
    }

    /// Sets a bounded polling timeout for acquiring the lock.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// An active OS advisory file lock guard.
///
/// Releases the underlying advisory lock when [`FileLock::unlock`] is called or
/// when the guard is dropped.
#[derive(Debug)]
pub struct FileLock {
    file: Option<File>,
    mode: LockMode,
}

impl FileLock {
    /// Opens `path` (rejecting symbolic links) and acquires an OS advisory lock
    /// according to `opts`.
    ///
    /// * Rejects empty paths, NUL bytes, trailing `.` segments, and symlink targets
    ///   with [`crate::ExecErrorCode::InvalidArgument`].
    /// * On Unix, opens with `O_NOFOLLOW` so symlink races cannot follow targets.
    /// * Uses [`File::lock`], [`File::lock_shared`], [`File::try_lock`], and
    ///   [`File::try_lock_shared`] with zero `unsafe`.
    pub fn acquire(path: &Path, opts: &LockOptions) -> Result<Self, ExecError> {
        validate_path_not_empty_or_nul(path, "path")?;
        let normalized = normalize_no_follow_path(path)?;

        if opts.non_blocking && opts.timeout.is_some() {
            return Err(ExecError::InvalidArgument {
                reason: "LockOptions cannot set both non_blocking and timeout",
            });
        }
        if let Some(timeout) = opts.timeout {
            validate_positive_duration(timeout, "timeout")?;
        }

        if let Ok(meta) = fs::symlink_metadata(&normalized)
            && is_symlink_or_reparse(&meta)
        {
            return Err(ExecError::InvalidArgument {
                reason: "lock path must not be a symbolic link",
            });
        }

        let mut open_opts = OpenOptions::new();
        if opts.create {
            open_opts.read(true).write(true).create(true);
        } else if opts.mode == LockMode::Exclusive {
            open_opts.read(true).write(true);
        } else {
            open_opts.read(true);
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            open_opts.custom_flags(libc::O_NOFOLLOW);
        }

        let file = open_opts.open(&*normalized).map_err(|err| {
            #[cfg(unix)]
            if err.raw_os_error() == Some(libc::ELOOP) {
                return ExecError::InvalidArgument {
                    reason: "lock path must not be a symbolic link",
                };
            }
            if fs::symlink_metadata(&normalized).is_ok_and(|meta| is_symlink_or_reparse(&meta)) {
                return ExecError::InvalidArgument {
                    reason: "lock path must not be a symbolic link",
                };
            }
            map_io_error("open lock file", err)
        })?;

        #[cfg(windows)]
        if fs::symlink_metadata(&normalized).is_ok_and(|meta| is_symlink_or_reparse(&meta)) {
            return Err(ExecError::InvalidArgument {
                reason: "lock path must not be a symbolic link",
            });
        }

        let meta = file
            .metadata()
            .map_err(|err| map_io_error("inspect lock file metadata", err))?;
        if !meta.is_file() {
            return Err(ExecError::InvalidArgument {
                reason: "lock path must refer to a regular file",
            });
        }

        if opts.non_blocking {
            try_lock_once(&file, opts.mode)?;
        } else if let Some(timeout) = opts.timeout {
            acquire_with_timeout(&file, opts.mode, timeout)?;
        } else {
            acquire_blocking(&file, opts.mode)?;
        }

        Ok(Self {
            file: Some(file),
            mode: opts.mode,
        })
    }

    /// Returns the [`LockMode`] held by this guard.
    #[must_use]
    pub const fn mode(&self) -> LockMode {
        self.mode
    }

    /// Returns a shared reference to the underlying locked [`File`].
    ///
    /// # Panics
    ///
    /// Never panics for a live [`FileLock`] (the inner file is present until `unlock` or `Drop`).
    #[must_use]
    pub fn file(&self) -> &File {
        self.file
            .as_ref()
            .expect("FileLock inner file always present before unlock/drop")
    }

    /// Returns a mutable reference to the underlying locked [`File`].
    ///
    /// # Panics
    ///
    /// Never panics for a live [`FileLock`] (the inner file is present until `unlock` or `Drop`).
    pub fn file_mut(&mut self) -> &mut File {
        self.file
            .as_mut()
            .expect("FileLock inner file always present before unlock/drop")
    }

    /// Explicitly unlocks and closes the underlying file handle, propagating any OS unlock error.
    pub fn unlock(mut self) -> Result<(), ExecError> {
        if let Some(file) = self.file.take() {
            file.unlock()
                .map_err(|err| map_io_error("unlock file", err))?;
        }
        Ok(())
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
    }
}

fn try_lock_once(file: &File, mode: LockMode) -> Result<(), ExecError> {
    let res = match mode {
        LockMode::Exclusive => file.try_lock(),
        LockMode::Shared => file.try_lock_shared(),
    };
    match res {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(ExecError::LockWouldBlock),
        Err(TryLockError::Error(err)) => Err(map_io_error("try_lock file", err)),
    }
}

fn acquire_blocking(file: &File, mode: LockMode) -> Result<(), ExecError> {
    loop {
        let res = match mode {
            LockMode::Exclusive => file.lock(),
            LockMode::Shared => file.lock_shared(),
        };
        match res {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(map_io_error("lock file", err)),
        }
    }
}

fn acquire_with_timeout(file: &File, mode: LockMode, timeout: Duration) -> Result<(), ExecError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(ExecError::InvalidArgument {
            reason: "timeout overflows monotonic clock",
        })?;
    let mut sleep_dur = Duration::from_millis(1);

    loop {
        match try_lock_once(file, mode) {
            Ok(()) => return Ok(()),
            Err(ExecError::LockWouldBlock) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(ExecError::LockTimeout);
                }
                let remaining = deadline.saturating_duration_since(now);
                std::thread::sleep(sleep_dur.min(remaining));
                sleep_dur = (sleep_dur * 2).min(Duration::from_millis(10));
            }
            Err(err) => return Err(err),
        }
    }
}

fn is_path_sep(byte: u8) -> bool {
    byte == b'/' || (cfg!(windows) && byte == b'\\')
}

fn has_trailing_dot_segment(path: &Path) -> bool {
    let mut bytes = path.as_os_str().as_encoded_bytes();
    while bytes.len() > 1 && is_path_sep(bytes[bytes.len() - 1]) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes.ends_with(b"/.") || (cfg!(windows) && bytes.ends_with(b"\\."))
}

fn normalize_no_follow_path(path: &Path) -> Result<Cow<'_, Path>, ExecError> {
    if has_trailing_dot_segment(path) {
        return Err(ExecError::InvalidArgument {
            reason: "lock path must not end with a trailing '.' segment",
        });
    }
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.last().copied().is_some_and(is_path_sep) {
        let normalized: PathBuf = path.components().collect();
        return Ok(Cow::Owned(normalized));
    }
    Ok(Cow::Borrowed(path))
}

fn is_symlink_or_reparse(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
