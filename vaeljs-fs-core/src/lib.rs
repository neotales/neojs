//! Engine-neutral core filesystem operations for the `vael:fs` module.
//!
//! This crate provides synchronous, fail-closed, cross-platform filesystem
//! primitives shared by both the V8 runtime (`vaeljs-core`) and the QuickJS
//! runtime (`vaeljs-lite-core`). It carries zero JavaScript engine dependencies
//! and zero network dependencies.
//!
//! # Contract
//!
//! * Every operation validates input paths up front, rejecting empty paths and
//!   paths containing NUL (`\0`) bytes with [`FsErrorCode::InvalidPath`].
//! * Errors are returned via [`FsError`] and never panic. Error messages
//!   describe the cause without echoing caller-supplied paths.
//! * [`read_file`] and [`read_text_file`] enforce optional `max_bytes` limits
//!   via metadata pre-checks and a bounded `take(max_bytes + 1)` stream read.
//! * [`write_file_atomic`] writes to a unique sibling temporary file using
//!   `create_new(true)` and fallible OS entropy from `getrandom`, optionally
//!   syncs, cleans up temporary files on error without masking the original
//!   error, and respects `overwrite: false` without clobbering existing entries.
//! * [`read_dir`] sorts entries deterministically by [`DirEntry::name`] and
//!   fails closed with [`FsErrorCode::InvalidUtf8`] on non-UTF-8 names.
//!
//! # Example
//!
//! ```
//! use vaeljs_fs_core::{
//!     AtomicWriteOptions, FsErrorCode, ReadOptions, read_text_file, write_file_atomic,
//! };
//!
//! let dir = tempfile::tempdir().unwrap();
//! let file = dir.path().join("greeting.txt");
//!
//! write_file_atomic(&file, b"hello vael:fs", AtomicWriteOptions::default()).unwrap();
//! assert_eq!(
//!     read_text_file(&file, ReadOptions::default()).unwrap(),
//!     "hello vael:fs"
//! );
//!
//! let err = read_text_file("", ReadOptions::default()).unwrap_err();
//! assert_eq!(err.code(), FsErrorCode::InvalidPath);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod ops;
mod types;

pub use error::{FsError, FsErrorCode};
pub use ops::{
    copy_file, exists, lstat, mkdir, read_dir, read_file, read_text_file, realpath, remove, rename,
    stat, write_file, write_file_atomic, write_text_file,
};
pub use types::{
    AtomicWriteOptions, CopyOptions, DirEntry, FileInfo, MkdirOptions, ReadOptions, RemoveOptions,
    WriteOptions,
};

/// The canonical `vael:fs` import specifier and module registry key.
pub const SPECIFIER: &str = "vael:fs";

/// The named exports of the `vael:fs` JavaScript module, in property order.
pub const EXPORTS: &[&str] = &[
    "readFile",
    "readTextFile",
    "writeFile",
    "writeTextFile",
    "writeFileAtomic",
    "exists",
    "stat",
    "lstat",
    "readDir",
    "mkdir",
    "remove",
    "rename",
    "copyFile",
    "realpath",
];
