//! Option structs and metadata result types for `vael:fs`.

/// Options controlling [`crate::read_file`] and [`crate::read_text_file`].
///
/// By default (`ReadOptions::default()`), `max_bytes` is `None` (no size limit).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReadOptions {
    /// Maximum number of bytes allowed when reading a file.
    ///
    /// When set, files larger than `max_bytes` fail with
    /// [`crate::FsErrorCode::TooLarge`] before allocating the full buffer.
    pub max_bytes: Option<u64>,
}

/// Options controlling [`crate::write_file`] and [`crate::write_text_file`].
///
/// Default values are `create: true`, `create_new: false`, and `append: false`,
/// which creates the file if missing and truncates it if it already exists.
/// Setting both `create_new` and `append` to `true` is rejected with
/// [`crate::FsErrorCode::InvalidArgument`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    /// Create the file if it does not already exist (default: `true`).
    pub create: bool,
    /// Require exclusive creation and fail with [`crate::FsErrorCode::AlreadyExists`]
    /// if the file already exists (default: `false`).
    pub create_new: bool,
    /// Append writes to the end of the file instead of truncating it (default: `false`).
    pub append: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            create: true,
            create_new: false,
            append: false,
        }
    }
}

/// Options controlling [`crate::write_file_atomic`].
///
/// Default values are `sync: true` and `overwrite: true`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtomicWriteOptions {
    /// Call `sync_all` on the temporary file before replacing the target
    /// (default: `true`).
    pub sync: bool,
    /// Replace an existing target file if present (default: `true`).
    ///
    /// When `false`, existing files and symlinks are never overwritten and fail
    /// with [`crate::FsErrorCode::AlreadyExists`].
    pub overwrite: bool,
}

impl Default for AtomicWriteOptions {
    fn default() -> Self {
        Self {
            sync: true,
            overwrite: true,
        }
    }
}

/// Options controlling [`crate::mkdir`].
///
/// Default value is `recursive: false`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MkdirOptions {
    /// Create missing parent directories and succeed if the directory already
    /// exists (default: `false`).
    pub recursive: bool,
}

/// Options controlling [`crate::remove`].
///
/// Default value is `recursive: false`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RemoveOptions {
    /// Recursively remove directory contents without following symbolic links
    /// (default: `false`).
    pub recursive: bool,
}

/// Options controlling [`crate::copy_file`].
///
/// Default value is `overwrite: false`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CopyOptions {
    /// Overwrite the destination file if it already exists (default: `false`).
    ///
    /// When `false`, the destination is opened with exclusive creation
    /// (`create_new(true)`) so existing targets fail with
    /// [`crate::FsErrorCode::AlreadyExists`] without TOCTOU clobbering.
    pub overwrite: bool,
}

/// Snapshot metadata returned by [`crate::stat`] and [`crate::lstat`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    /// Whether the entry is a regular file.
    pub is_file: bool,
    /// Whether the entry is a directory.
    pub is_directory: bool,
    /// Whether the entry is a symbolic link (always `false` for [`crate::stat`]).
    pub is_symlink: bool,
    /// Size of the entry in bytes.
    pub size: u64,
    /// Last modification time in whole milliseconds since the Unix epoch, if
    /// available from the filesystem and representable as `i64`.
    pub modified_ms: Option<i64>,
    /// Whether the entry's permissions mark it as read-only.
    pub readonly: bool,
}

/// Snapshot directory entry returned in sorted order by [`crate::read_dir`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirEntry {
    /// Single path component name (valid UTF-8).
    pub name: String,
    /// Whether the entry is a regular file (does not follow symlinks).
    pub is_file: bool,
    /// Whether the entry is a directory (does not follow symlinks).
    pub is_directory: bool,
    /// Whether the entry is a symbolic link.
    pub is_symlink: bool,
}
