use std::borrow::Cow;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{FsError, map_getrandom_error, map_io_error};
use crate::types::{
    AtomicWriteOptions, CopyOptions, DirEntry, FileInfo, MkdirOptions, ReadOptions, RemoveOptions,
    WriteOptions,
};

const MAX_PREALLOC_BYTES: usize = 1024 * 1024;
const MAX_TEMP_ATTEMPTS: usize = 16;
const TEMP_RANDOM_BYTES: usize = 16;

/// RAII guard that removes a newly created temporary or partial destination file
/// on drop unless disarmed after commit.
struct FileCleanupGuard {
    path: PathBuf,
    armed: bool,
}

impl FileCleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FileCleanupGuard {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort cleanup so a cleanup failure never masks the original error.
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn is_path_sep(byte: u8) -> bool {
    byte == b'/' || (cfg!(windows) && byte == b'\\')
}

fn validate_path(path: &Path) -> Result<(), FsError> {
    let os = path.as_os_str();
    if os.is_empty() {
        return Err(FsError::InvalidPath {
            reason: "path must not be empty",
            source: None,
        });
    }
    // `as_encoded_bytes()` returns raw bytes on Unix and WTF-8 on Windows; 0x00
    // only ever encodes U+0000 NUL in both encodings.
    if os.as_encoded_bytes().contains(&0) {
        return Err(FsError::InvalidPath {
            reason: "path must not contain a NUL byte",
            source: None,
        });
    }
    Ok(())
}

fn has_trailing_dot_segment(path: &Path) -> bool {
    let mut bytes = path.as_os_str().as_encoded_bytes();
    while bytes.len() > 1 && is_path_sep(bytes[bytes.len() - 1]) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes.ends_with(b"/.") || (cfg!(windows) && bytes.ends_with(b"\\."))
}

/// Normalizes a path for `lstat` and `remove` so trailing directory separators
/// do not cause the OS to follow a symbolic link to a directory.
fn normalize_no_follow_path(path: &Path) -> Result<Cow<'_, Path>, FsError> {
    if has_trailing_dot_segment(path) {
        return Err(FsError::InvalidPath {
            reason: "path must not end with a trailing '.' segment",
            source: None,
        });
    }
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.last().copied().is_some_and(is_path_sep) {
        // `Path::components()` strips trailing separators on non-root paths
        // while preserving filesystem roots (`/` on Unix, drive/UNC roots on Windows).
        let normalized: PathBuf = path.components().collect();
        return Ok(Cow::Owned(normalized));
    }
    Ok(Cow::Borrowed(path))
}

fn map_open_file_error(path: &Path, err: io::Error) -> FsError {
    // On Windows, opening a directory as a file (or copying over one) reports
    // `PermissionDenied` rather than `IsADirectory`.
    #[cfg(windows)]
    if err.kind() == io::ErrorKind::PermissionDenied
        && fs::metadata(path).is_ok_and(|meta| meta.is_dir())
    {
        return FsError::IsADirectory {
            source: io::Error::from(io::ErrorKind::IsADirectory),
        };
    }
    #[cfg(not(windows))]
    let _ = path;
    map_io_error(err)
}

fn is_same_file(from: &Path, src_meta: &Metadata, to: &Path) -> Result<bool, FsError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = from;
        match fs::metadata(to) {
            Ok(dst_meta) => {
                Ok(src_meta.dev() == dst_meta.dev() && src_meta.ino() == dst_meta.ino())
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(map_io_error(err)),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = src_meta;
        match fs::metadata(to) {
            Ok(_) => {
                let canon_from = fs::canonicalize(from).map_err(map_io_error)?;
                let canon_to = fs::canonicalize(to).map_err(map_io_error)?;
                Ok(canon_from == canon_to)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(map_io_error(err)),
        }
    }
}

fn system_time_to_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).ok(),
        Err(err) => {
            let millis = i128::try_from(err.duration().as_millis()).ok()?;
            i64::try_from(-millis).ok()
        }
    }
}

fn file_info_from_metadata(meta: &Metadata) -> FileInfo {
    let file_type = meta.file_type();
    FileInfo {
        is_file: file_type.is_file(),
        is_directory: file_type.is_dir(),
        is_symlink: file_type.is_symlink(),
        size: meta.len(),
        modified_ms: meta.modified().ok().and_then(system_time_to_ms),
        readonly: meta.permissions().readonly(),
    }
}

fn random_temp_name() -> Result<String, FsError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0u8; TEMP_RANDOM_BYTES];
    getrandom::fill(&mut bytes).map_err(map_getrandom_error)?;

    let mut name = String::with_capacity(".vaeljs-tmp-".len() + TEMP_RANDOM_BYTES * 2);
    name.push_str(".vaeljs-tmp-");
    for byte in bytes {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(name)
}

fn create_unique_temp_file(parent: &Path) -> Result<(FileCleanupGuard, File), FsError> {
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let candidate = parent.join(random_temp_name()?);
        let mut open_opts = OpenOptions::new();
        open_opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_opts.mode(0o600);
        }
        match open_opts.open(&candidate) {
            Ok(file) => return Ok((FileCleanupGuard::new(candidate), file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(map_io_error(err)),
        }
    }
    Err(FsError::Io {
        reason: "could not allocate a unique temporary file name".to_owned(),
        source: None,
    })
}

/// Reads the entire file at `path` into memory, optionally enforcing a byte limit.
///
/// Follows symbolic links. Rejects empty paths and paths containing NUL bytes
/// with [`crate::FsErrorCode::InvalidPath`], and directories with
/// [`crate::FsErrorCode::IsADirectory`].
///
/// When [`ReadOptions::max_bytes`] is `Some(limit)`, both file metadata (for
/// regular files) and a bounded `take(limit + 1)` stream read are checked so
/// oversized or growing files fail with [`crate::FsErrorCode::TooLarge`]
/// without unbounded allocation.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, ReadOptions, read_file, write_file, WriteOptions};
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("data.bin");
/// write_file(&path, b"hello", WriteOptions::default()).unwrap();
///
/// let bytes = read_file(&path, ReadOptions { max_bytes: Some(16) }).unwrap();
/// assert_eq!(bytes, b"hello");
///
/// let err = read_file(&path, ReadOptions { max_bytes: Some(4) }).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::TooLarge);
/// ```
pub fn read_file(path: impl AsRef<Path>, options: ReadOptions) -> Result<Vec<u8>, FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    let file = File::open(path).map_err(|err| map_open_file_error(path, err))?;
    let meta = file.metadata().map_err(map_io_error)?;
    if meta.is_dir() {
        return Err(FsError::IsADirectory {
            source: io::Error::from(io::ErrorKind::IsADirectory),
        });
    }

    if let Some(max_bytes) = options.max_bytes {
        if meta.is_file() && meta.len() > max_bytes {
            return Err(FsError::TooLarge {
                limit: max_bytes,
                actual: Some(meta.len()),
            });
        }
        let initial_cap = if meta.is_file() {
            usize::try_from(meta.len().min(max_bytes))
                .unwrap_or(0)
                .min(MAX_PREALLOC_BYTES)
        } else {
            0
        };
        let mut buf = Vec::with_capacity(initial_cap);
        let read_cap = max_bytes.saturating_add(1);
        (&file)
            .take(read_cap)
            .read_to_end(&mut buf)
            .map_err(map_io_error)?;
        let read_len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if read_len > max_bytes {
            return Err(FsError::TooLarge {
                limit: max_bytes,
                actual: None,
            });
        }
        Ok(buf)
    } else {
        let initial_cap = if meta.is_file() {
            usize::try_from(meta.len())
                .unwrap_or(0)
                .min(MAX_PREALLOC_BYTES)
        } else {
            0
        };
        let mut buf = Vec::with_capacity(initial_cap);
        (&file).read_to_end(&mut buf).map_err(map_io_error)?;
        Ok(buf)
    }
}

/// Reads the entire file at `path` and validates it as strict UTF-8.
///
/// Behaves identically to [`read_file`] except that non-UTF-8 byte sequences
/// fail with [`crate::FsErrorCode::InvalidUtf8`] rather than performing lossy
/// replacement.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{
///     FsErrorCode, ReadOptions, WriteOptions, read_text_file, write_file, write_text_file,
/// };
///
/// let dir = tempfile::tempdir().unwrap();
/// let text_path = dir.path().join("hello.txt");
/// write_text_file(&text_path, "hello world", WriteOptions::default()).unwrap();
/// assert_eq!(
///     read_text_file(&text_path, ReadOptions::default()).unwrap(),
///     "hello world"
/// );
///
/// let bin_path = dir.path().join("bad.bin");
/// write_file(&bin_path, [0xff, 0xfe], WriteOptions::default()).unwrap();
/// let err = read_text_file(&bin_path, ReadOptions::default()).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::InvalidUtf8);
/// ```
pub fn read_text_file(path: impl AsRef<Path>, options: ReadOptions) -> Result<String, FsError> {
    let bytes = read_file(path, options)?;
    String::from_utf8(bytes).map_err(|_| FsError::InvalidUtf8 {
        reason: "file contents are not valid UTF-8",
    })
}

/// Writes `bytes` to `path` according to `options`.
///
/// By default (`WriteOptions::default()`), creates the file if missing and
/// truncates any existing file. Rejects empty/NUL paths with
/// [`crate::FsErrorCode::InvalidPath`] and rejects conflicting
/// `create_new + append` options with [`crate::FsErrorCode::InvalidArgument`].
///
/// This is a direct, non-atomic write: an I/O failure mid-write can leave a
/// truncated or partially written destination file. Use [`write_file_atomic`]
/// when all-or-nothing replacement is required.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, ReadOptions, WriteOptions, read_file, write_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("log.bin");
/// write_file(&path, b"ab", WriteOptions::default()).unwrap();
/// write_file(&path, b"cd", WriteOptions { append: true, ..Default::default() }).unwrap();
/// assert_eq!(read_file(&path, ReadOptions::default()).unwrap(), b"abcd");
///
/// let err = write_file(
///     &path,
///     b"x",
///     WriteOptions { create: true, create_new: true, append: true },
/// )
/// .unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::InvalidArgument);
/// ```
pub fn write_file(
    path: impl AsRef<Path>,
    bytes: impl AsRef<[u8]>,
    options: WriteOptions,
) -> Result<(), FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    if options.create_new && options.append {
        return Err(FsError::InvalidArgument {
            reason: "create_new and append cannot both be enabled",
        });
    }

    let mut open_opts = OpenOptions::new();
    open_opts
        .write(true)
        .create(options.create)
        .create_new(options.create_new)
        .append(options.append)
        .truncate(!options.append && !options.create_new);

    let mut file = open_opts
        .open(path)
        .map_err(|err| map_open_file_error(path, err))?;
    file.write_all(bytes.as_ref()).map_err(map_io_error)?;
    Ok(())
}

/// Writes UTF-8 `text` to `path` according to `options`.
///
/// Delegates to [`write_file`] with `text.as_ref().as_bytes()`.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{ReadOptions, WriteOptions, read_text_file, write_text_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("note.txt");
/// write_text_file(&path, "vael:fs", WriteOptions::default()).unwrap();
/// assert_eq!(read_text_file(&path, ReadOptions::default()).unwrap(), "vael:fs");
/// ```
pub fn write_text_file(
    path: impl AsRef<Path>,
    text: impl AsRef<str>,
    options: WriteOptions,
) -> Result<(), FsError> {
    write_file(path, text.as_ref().as_bytes(), options)
}

/// Atomically writes `bytes` to `path` via a sibling temporary file.
///
/// Writes to a unique sibling file in `path`'s parent directory created with
/// `create_new(true)` and a random suffix from [`getrandom::fill`] (`0o600` on
/// Unix). Optionally calls `sync_all()` (`options.sync`, plus a best-effort
/// parent directory sync on Unix), closes the handle and unlinks the temporary
/// file via RAII on any failure without masking the original error, and commits
/// the file to `path`.
///
/// Rejects empty/NUL paths, trailing directory separators, `.`, and trailing
/// `/.` (`\.` on Windows) segments with [`crate::FsErrorCode::InvalidPath`].
/// When `options.overwrite` is `false`, any existing entry at `path` (including
/// a symlink) fails with [`crate::FsErrorCode::AlreadyExists`] without
/// clobbering the destination.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{
///     AtomicWriteOptions, FsErrorCode, ReadOptions, read_text_file, write_file_atomic,
/// };
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("config.json");
/// write_file_atomic(&path, b"{}", AtomicWriteOptions::default()).unwrap();
/// assert_eq!(read_text_file(&path, ReadOptions::default()).unwrap(), "{}");
///
/// let err = write_file_atomic(
///     &path,
///     b"{\"v\":2}",
///     AtomicWriteOptions { sync: true, overwrite: false },
/// )
/// .unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::AlreadyExists);
/// assert_eq!(read_text_file(&path, ReadOptions::default()).unwrap(), "{}");
/// ```
pub fn write_file_atomic(
    path: impl AsRef<Path>,
    bytes: impl AsRef<[u8]>,
    options: AtomicWriteOptions,
) -> Result<(), FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    let encoded = path.as_os_str().as_encoded_bytes();
    if encoded.last().copied().is_some_and(is_path_sep) {
        return Err(FsError::InvalidPath {
            reason: "target path must not end with a directory separator",
            source: None,
        });
    }
    if encoded == b"." || has_trailing_dot_segment(path) {
        return Err(FsError::InvalidPath {
            reason: "target path must name a file, not '.'",
            source: None,
        });
    }
    if path.file_name().is_none() {
        return Err(FsError::InvalidPath {
            reason: "target path has no file name",
            source: None,
        });
    }

    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    if !options.overwrite {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(FsError::AlreadyExists {
                    source: io::Error::from(io::ErrorKind::AlreadyExists),
                });
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(map_io_error(err)),
        }
    }

    // `guard` is bound before `file` and `file` is scoped below so the OS handle
    // closes before `guard` unlinks the temp file on any error return.
    let (mut guard, file) = create_unique_temp_file(parent)?;
    {
        let mut file = file;
        file.write_all(bytes.as_ref()).map_err(map_io_error)?;
        if options.sync {
            file.sync_all().map_err(map_io_error)?;
        }
    }

    if options.overwrite {
        fs::rename(guard.path(), path).map_err(map_io_error)?;
        guard.disarm();
    } else {
        match fs::hard_link(guard.path(), path) {
            Ok(()) => {
                let _ = fs::remove_file(guard.path());
                guard.disarm();
            }
            Err(err) if err.kind() == io::ErrorKind::Unsupported => {
                match fs::symlink_metadata(path) {
                    Ok(_) => {
                        return Err(FsError::AlreadyExists {
                            source: io::Error::from(io::ErrorKind::AlreadyExists),
                        });
                    }
                    Err(meta_err) if meta_err.kind() == io::ErrorKind::NotFound => {
                        fs::rename(guard.path(), path).map_err(map_io_error)?;
                        guard.disarm();
                    }
                    Err(meta_err) => return Err(map_io_error(meta_err)),
                }
            }
            Err(err) => return Err(map_io_error(err)),
        }
    }

    #[cfg(unix)]
    if options.sync
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Checks whether `path` resolves to an existing filesystem entry.
///
/// Follows symbolic links (`stat` semantics). Returns `Ok(false)` **only** when
/// the operating system reports `NotFound`; permission errors, non-directory
/// intermediate path components, and invalid paths are returned as `Err`
/// instead of being swallowed.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, WriteOptions, exists, write_text_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("present.txt");
/// assert!(!exists(&path).unwrap());
///
/// write_text_file(&path, "ok", WriteOptions::default()).unwrap();
/// assert!(exists(&path).unwrap());
///
/// let err = exists("").unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::InvalidPath);
/// ```
pub fn exists(path: impl AsRef<Path>) -> Result<bool, FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(map_io_error(err)),
    }
}

/// Returns metadata for `path`, following symbolic links.
///
/// Fails with [`crate::FsErrorCode::NotFound`] if `path` or a symbolic link
/// target does not exist.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, WriteOptions, stat, write_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("item.bin");
/// write_file(&path, b"12345", WriteOptions::default()).unwrap();
///
/// let info = stat(&path).unwrap();
/// assert!(info.is_file);
/// assert!(!info.is_symlink);
/// assert_eq!(info.size, 5);
///
/// let err = stat(dir.path().join("missing")).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::NotFound);
/// ```
pub fn stat(path: impl AsRef<Path>) -> Result<FileInfo, FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    let meta = fs::metadata(path).map_err(map_io_error)?;
    Ok(file_info_from_metadata(&meta))
}

/// Returns metadata for `path` without following symbolic links.
///
/// When `path` is a symbolic link (even a dangling one), `info.is_symlink` is
/// `true`. Trailing directory separators on non-root paths are stripped before
/// inspecting metadata so `lstat("link/")` inspects the symlink itself rather
/// than resolving through it, and trailing `/.` (`\.` on Windows) segments are
/// rejected with [`crate::FsErrorCode::InvalidPath`].
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, lstat};
///
/// let dir = tempfile::tempdir().unwrap();
/// let info = lstat(dir.path()).unwrap();
/// assert!(info.is_directory);
///
/// let err = lstat(dir.path().join("missing")).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::NotFound);
/// ```
pub fn lstat(path: impl AsRef<Path>) -> Result<FileInfo, FsError> {
    let path = path.as_ref();
    validate_path(path)?;
    let path = normalize_no_follow_path(path)?;

    let meta = fs::symlink_metadata(path.as_ref()).map_err(map_io_error)?;
    Ok(file_info_from_metadata(&meta))
}

/// Lists direct entries in the directory at `path`, sorted deterministically by
/// [`DirEntry::name`].
///
/// Does not include `.` or `..` and does not follow symbolic links. Fails
/// closed with [`crate::FsErrorCode::InvalidUtf8`] if any entry name is not
/// valid UTF-8.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, WriteOptions, read_dir, write_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// write_file(dir.path().join("b.txt"), b"", WriteOptions::default()).unwrap();
/// write_file(dir.path().join("a.txt"), b"", WriteOptions::default()).unwrap();
///
/// let entries = read_dir(dir.path()).unwrap();
/// let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
/// assert_eq!(names, ["a.txt", "b.txt"]);
///
/// let err = read_dir(dir.path().join("a.txt")).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::NotADirectory);
/// ```
pub fn read_dir(path: impl AsRef<Path>) -> Result<Vec<DirEntry>, FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    let iter = fs::read_dir(path).map_err(map_io_error)?;
    let mut entries = Vec::new();

    for item in iter {
        let entry = item.map_err(map_io_error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| FsError::InvalidUtf8 {
                reason: "directory entry name is not valid UTF-8",
            })?;
        let file_type = entry.file_type().map_err(map_io_error)?;
        entries.push(DirEntry {
            name,
            is_file: file_type.is_file(),
            is_directory: file_type.is_dir(),
            is_symlink: file_type.is_symlink(),
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Creates a directory at `path`.
///
/// When `options.recursive` is `false`, creates a single directory and fails
/// with [`crate::FsErrorCode::AlreadyExists`] if it already exists or
/// [`crate::FsErrorCode::NotFound`] if its parent is missing. When
/// `options.recursive` is `true`, creates missing parents and succeeds if the
/// directory already exists.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, MkdirOptions, exists, mkdir};
///
/// let dir = tempfile::tempdir().unwrap();
/// let nested = dir.path().join("a/b/c");
///
/// let err = mkdir(&nested, MkdirOptions::default()).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::NotFound);
///
/// mkdir(&nested, MkdirOptions { recursive: true }).unwrap();
/// assert!(exists(&nested).unwrap());
/// ```
pub fn mkdir(path: impl AsRef<Path>, options: MkdirOptions) -> Result<(), FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    if options.recursive {
        fs::create_dir_all(path).map_err(map_io_error)
    } else {
        fs::create_dir(path).map_err(map_io_error)
    }
}

/// Removes a file, symbolic link, or directory at `path`.
///
/// Never follows symbolic links: trailing directory separators on non-root
/// paths are stripped before inspecting metadata so `remove("link/", ...)`
/// unlinks the symlink itself without touching its target, even when
/// `options.recursive` is `true`. Trailing `/.` (`\.` on Windows) segments are
/// rejected with [`crate::FsErrorCode::InvalidPath`].
///
/// When `options.recursive` is `false`, removes files, symlinks, and empty
/// directories; non-empty directories fail with
/// [`crate::FsErrorCode::DirectoryNotEmpty`]. When `options.recursive` is
/// `true`, recursively removes directory trees without following symlinks.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{
///     FsErrorCode, MkdirOptions, RemoveOptions, WriteOptions, exists, mkdir, remove, write_file,
/// };
///
/// let dir = tempfile::tempdir().unwrap();
/// let sub = dir.path().join("sub");
/// mkdir(&sub, MkdirOptions::default()).unwrap();
/// write_file(sub.join("f.txt"), b"x", WriteOptions::default()).unwrap();
///
/// let err = remove(&sub, RemoveOptions::default()).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::DirectoryNotEmpty);
///
/// remove(&sub, RemoveOptions { recursive: true }).unwrap();
/// assert!(!exists(&sub).unwrap());
/// ```
pub fn remove(path: impl AsRef<Path>, options: RemoveOptions) -> Result<(), FsError> {
    let path = path.as_ref();
    validate_path(path)?;
    let path = normalize_no_follow_path(path)?;
    let path = path.as_ref();

    let meta = fs::symlink_metadata(path).map_err(map_io_error)?;
    let file_type = meta.file_type();

    if file_type.is_symlink() {
        #[cfg(windows)]
        if meta.is_dir() {
            return fs::remove_dir(path).map_err(map_io_error);
        }
        return fs::remove_file(path).map_err(map_io_error);
    }

    if file_type.is_dir() {
        if options.recursive {
            return fs::remove_dir_all(path).map_err(map_io_error);
        }
        return fs::remove_dir(path).map_err(map_io_error);
    }

    fs::remove_file(path).map_err(map_io_error)
}

/// Renames or moves a file or directory from `from` to `to`.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{
///     FsErrorCode, ReadOptions, WriteOptions, exists, read_text_file, rename, write_text_file,
/// };
///
/// let dir = tempfile::tempdir().unwrap();
/// let src = dir.path().join("old.txt");
/// let dst = dir.path().join("new.txt");
/// write_text_file(&src, "moved", WriteOptions::default()).unwrap();
///
/// rename(&src, &dst).unwrap();
/// assert!(!exists(&src).unwrap());
/// assert_eq!(read_text_file(&dst, ReadOptions::default()).unwrap(), "moved");
///
/// let err = rename(&src, &dst).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::NotFound);
/// ```
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), FsError> {
    let from = from.as_ref();
    let to = to.as_ref();
    validate_path(from)?;
    validate_path(to)?;

    fs::rename(from, to).map_err(map_io_error)
}

/// Copies a regular file from `from` to `to` according to `options`.
///
/// Fails with [`crate::FsErrorCode::IsADirectory`] if `from` resolves to a
/// directory.
///
/// * When `options.overwrite` is `false` (the default), opens `to` with
///   `create_new(true)` to prevent TOCTOU clobbering and fails with
///   [`crate::FsErrorCode::AlreadyExists`] if `to` already exists. On Unix,
///   the destination file is created with mode `(src_mode & 0o777) | 0o200`
///   before copying bytes, then updated to `src`'s permissions; on partial
///   write failure the newly created `to` is closed and removed via RAII.
/// * When `options.overwrite` is `true`, if `from` and `to` resolve to the
///   same underlying file (same `(dev, ino)` on Unix, or matching canonical
///   paths elsewhere), returns `Ok(())` as a safe no-op without truncating.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{
///     CopyOptions, FsErrorCode, ReadOptions, WriteOptions, copy_file, read_text_file,
///     write_text_file,
/// };
///
/// let dir = tempfile::tempdir().unwrap();
/// let src = dir.path().join("src.txt");
/// let dst = dir.path().join("dst.txt");
/// write_text_file(&src, "v1", WriteOptions::default()).unwrap();
///
/// copy_file(&src, &dst, CopyOptions::default()).unwrap();
/// assert_eq!(read_text_file(&dst, ReadOptions::default()).unwrap(), "v1");
///
/// write_text_file(&src, "v2", WriteOptions::default()).unwrap();
/// let err = copy_file(&src, &dst, CopyOptions::default()).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::AlreadyExists);
///
/// copy_file(&src, &dst, CopyOptions { overwrite: true }).unwrap();
/// assert_eq!(read_text_file(&dst, ReadOptions::default()).unwrap(), "v2");
/// ```
pub fn copy_file(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
    options: CopyOptions,
) -> Result<(), FsError> {
    let from = from.as_ref();
    let to = to.as_ref();
    validate_path(from)?;
    validate_path(to)?;

    let mut src = File::open(from).map_err(|err| map_open_file_error(from, err))?;
    let src_meta = src.metadata().map_err(map_io_error)?;
    if src_meta.is_dir() {
        return Err(FsError::IsADirectory {
            source: io::Error::from(io::ErrorKind::IsADirectory),
        });
    }

    if options.overwrite {
        if is_same_file(from, &src_meta, to)? {
            return Ok(());
        }
        drop(src);
        fs::copy(from, to)
            .map(|_| ())
            .map_err(|err| map_open_file_error(to, err))?;
        return Ok(());
    }

    let mut dst_opts = OpenOptions::new();
    dst_opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        dst_opts.mode((src_meta.mode() & 0o777) | 0o200);
    }

    let dst = dst_opts
        .open(to)
        .map_err(|err| map_open_file_error(to, err))?;
    let mut guard = FileCleanupGuard::new(to.to_path_buf());
    {
        let mut dst = dst;
        io::copy(&mut src, &mut dst).map_err(map_io_error)?;
        if let Err(err) = dst.set_permissions(src_meta.permissions())
            && err.kind() != io::ErrorKind::Unsupported
        {
            return Err(map_io_error(err));
        }
    }
    guard.disarm();
    Ok(())
}

/// Resolves the canonical, absolute filesystem path for `path`, following all
/// symbolic links and normalizing `.` and `..` components.
///
/// # Example
///
/// ```
/// use vaeljs_fs_core::{FsErrorCode, WriteOptions, realpath, write_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// let file = dir.path().join("target.txt");
/// write_file(&file, b"ok", WriteOptions::default()).unwrap();
///
/// let via_dot = dir.path().join("./target.txt");
/// assert_eq!(realpath(&via_dot).unwrap(), realpath(&file).unwrap());
///
/// let err = realpath(dir.path().join("missing")).unwrap_err();
/// assert_eq!(err.code(), FsErrorCode::NotFound);
/// ```
pub fn realpath(path: impl AsRef<Path>) -> Result<PathBuf, FsError> {
    let path = path.as_ref();
    validate_path(path)?;

    fs::canonicalize(path).map_err(map_io_error)
}
