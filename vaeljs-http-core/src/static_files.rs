//! Safe static file resolution and web MIME type mapping.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::HttpCoreError;
use crate::error::map_io_error;
use crate::policy::percent_decode_segment;

/// Metadata and canonical filesystem path for a resolved static file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticFile {
    /// Canonicalized filesystem path guaranteed to reside inside `root_dir`.
    pub path: PathBuf,
    /// Inferred `Content-Type` MIME string for this file.
    pub mime_type: &'static str,
    /// File length in bytes.
    pub size: u64,
}

/// Resolves a raw (percent-encoded) `url_path` safely beneath `root_dir`,
/// optionally resolving directories to `index_file`.
///
/// `url_path` must be a raw URL path before percent-decoding. Callers passing an
/// already-decoded path (such as a [`crate::Router`] `{path...}` wildcard
/// capture) should use [`resolve_static_relative_path`] instead to avoid
/// double percent-decoding.
///
/// Security guarantees:
/// * Rejects ASCII control characters (including CR, LF, NUL `\0` / `%00`),
///   backslashes (`\`, `%5c`), colons / Windows drive prefixes (`:`, `%3a`),
///   encoded forward slashes (`%2f`), and `..` traversal segments (including
///   `%2e%2e`) with [`HttpCoreError::InvalidPath`].
/// * Validates `index_file` as a single plain filename with no separators,
///   colons, control characters, or traversal segments.
/// * Walks component-by-component from canonical `root_dir`, checking
///   `symlink_metadata` and canonicalizing any intermediate symbolic link (and
///   any resolved `index_file`) so symlinks escaping `root_dir` fail with
///   [`HttpCoreError::InvalidPath`] even when a descendant does not exist.
/// * Requires a directory when `url_path` ends with `/` (other than `"/"`), and
///   returns [`HttpCoreError::NotFound`] when the target does not exist, is not
///   a regular file, or is a directory without a matching `index_file`.
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidPath`] for traversal attempts, forbidden
/// characters, invalid percent-encoding, or symlink escapes;
/// [`HttpCoreError::NotFound`] when no regular file exists at the resolved
/// location; or [`HttpCoreError::Io`] on other OS filesystem errors.
///
/// # Examples
///
/// ```
/// use vaeljs_http_core::{HttpCoreErrorCode, resolve_static_file};
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::write(dir.path().join("index.html"), b"<h1>hello</h1>").unwrap();
///
/// let file = resolve_static_file(dir.path(), "/", Some("index.html")).unwrap();
/// assert_eq!(file.mime_type, "text/html; charset=utf-8");
/// assert_eq!(file.size, 14);
///
/// let err = resolve_static_file(dir.path(), "/../etc/passwd", None).unwrap_err();
/// assert_eq!(err.code(), HttpCoreErrorCode::InvalidPath);
/// ```
pub fn resolve_static_file(
    root_dir: &Path,
    url_path: &str,
    index_file: Option<&str>,
) -> Result<StaticFile, HttpCoreError> {
    if let Some(index) = index_file {
        validate_index_file(index)?;
    }

    let requires_dir = url_path != "/" && url_path.ends_with('/');
    let rel_path = parse_safe_relative_path(url_path, true)?;
    resolve_relative_path_buf(root_dir, &rel_path, requires_dir, index_file)
}

/// Resolves an already percent-decoded `relative_path` (such as a
/// [`crate::Router`] `{path...}` wildcard capture) safely beneath `root_dir`,
/// optionally resolving directories to `index_file`.
///
/// Unlike [`resolve_static_file`], `relative_path` is NOT percent-decoded, so
/// literal `%` characters in filenames are preserved without double-decoding.
/// All traversal, colon, backslash, control-character, trailing-slash, and
/// symlink-containment checks still apply.
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidPath`] for traversal attempts, forbidden
/// characters, or symlink escapes; [`HttpCoreError::NotFound`] when no regular
/// file exists at the resolved location; or [`HttpCoreError::Io`] on other OS
/// filesystem errors.
///
/// # Examples
///
/// ```
/// use vaeljs_http_core::resolve_static_relative_path;
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::write(dir.path().join("100%real.txt"), b"ok").unwrap();
///
/// let file = resolve_static_relative_path(dir.path(), "100%real.txt", None).unwrap();
/// assert_eq!(file.size, 2);
/// ```
pub fn resolve_static_relative_path(
    root_dir: &Path,
    relative_path: &str,
    index_file: Option<&str>,
) -> Result<StaticFile, HttpCoreError> {
    if let Some(index) = index_file {
        validate_index_file(index)?;
    }

    let requires_dir = !relative_path.is_empty() && relative_path.ends_with('/');
    let rel_path = parse_safe_relative_path(relative_path, false)?;
    resolve_relative_path_buf(root_dir, &rel_path, requires_dir, index_file)
}

fn resolve_relative_path_buf(
    root_dir: &Path,
    rel_path: &Path,
    requires_dir: bool,
    index_file: Option<&str>,
) -> Result<StaticFile, HttpCoreError> {
    let canonical_root = fs::canonicalize(root_dir).map_err(map_io_error)?;
    if !canonical_root.is_dir() {
        return Err(HttpCoreError::NotFound);
    }

    let mut current = canonical_root.clone();
    for component in rel_path.components() {
        current.push(component);
        let link_meta = fs::symlink_metadata(&current).map_err(map_io_error)?;
        if link_meta.file_type().is_symlink() {
            let resolved = fs::canonicalize(&current).map_err(map_io_error)?;
            if !resolved.starts_with(&canonical_root) {
                return Err(HttpCoreError::InvalidPath {
                    reason: "resolved path escapes static root directory",
                });
            }
            current = resolved;
        }
    }

    let canonical_candidate = fs::canonicalize(&current).map_err(map_io_error)?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(HttpCoreError::InvalidPath {
            reason: "resolved path escapes static root directory",
        });
    }

    let meta = fs::metadata(&canonical_candidate).map_err(map_io_error)?;
    if meta.is_dir() {
        let Some(index) = index_file else {
            return Err(HttpCoreError::NotFound);
        };

        let index_candidate = canonical_candidate.join(index);
        let _ = fs::symlink_metadata(&index_candidate).map_err(map_io_error)?;
        let canonical_index = fs::canonicalize(&index_candidate).map_err(map_io_error)?;
        if !canonical_index.starts_with(&canonical_root) {
            return Err(HttpCoreError::InvalidPath {
                reason: "resolved index file escapes static root directory",
            });
        }

        let index_meta = fs::metadata(&canonical_index).map_err(map_io_error)?;
        if !index_meta.is_file() {
            return Err(HttpCoreError::NotFound);
        }

        return Ok(StaticFile {
            mime_type: mime_type_for_path(&canonical_index),
            path: canonical_index,
            size: index_meta.len(),
        });
    }

    if requires_dir || !meta.is_file() {
        return Err(HttpCoreError::NotFound);
    }

    Ok(StaticFile {
        mime_type: mime_type_for_path(&canonical_candidate),
        path: canonical_candidate,
        size: meta.len(),
    })
}

/// Returns the web MIME `Content-Type` string for `path` based on its
/// extension (ASCII case-insensitive), defaulting to
/// `"application/octet-stream"`.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use vaeljs_http_core::mime_type_for_path;
///
/// assert_eq!(mime_type_for_path(Path::new("app.mjs")), "text/javascript; charset=utf-8");
/// assert_eq!(mime_type_for_path(Path::new("logo.SVG")), "image/svg+xml");
/// assert_eq!(mime_type_for_path(Path::new("data.bin")), "application/octet-stream");
/// ```
#[must_use]
pub fn mime_type_for_path(path: &Path) -> &'static str {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return "application/octet-stream";
    };

    if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
        "text/html; charset=utf-8"
    } else if ext.eq_ignore_ascii_case("css") {
        "text/css; charset=utf-8"
    } else if ext.eq_ignore_ascii_case("js") || ext.eq_ignore_ascii_case("mjs") {
        "text/javascript; charset=utf-8"
    } else if ext.eq_ignore_ascii_case("json") {
        "application/json"
    } else if ext.eq_ignore_ascii_case("svg") {
        "image/svg+xml"
    } else if ext.eq_ignore_ascii_case("png") {
        "image/png"
    } else if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        "image/jpeg"
    } else if ext.eq_ignore_ascii_case("gif") {
        "image/gif"
    } else if ext.eq_ignore_ascii_case("webp") {
        "image/webp"
    } else if ext.eq_ignore_ascii_case("ico") {
        "image/x-icon"
    } else if ext.eq_ignore_ascii_case("txt") {
        "text/plain; charset=utf-8"
    } else if ext.eq_ignore_ascii_case("xml") {
        "application/xml"
    } else if ext.eq_ignore_ascii_case("wasm") {
        "application/wasm"
    } else if ext.eq_ignore_ascii_case("woff") {
        "font/woff"
    } else if ext.eq_ignore_ascii_case("woff2") {
        "font/woff2"
    } else if ext.eq_ignore_ascii_case("pdf") {
        "application/pdf"
    } else {
        "application/octet-stream"
    }
}

fn validate_index_file(index: &str) -> Result<(), HttpCoreError> {
    if index.is_empty()
        || index == "."
        || index == ".."
        || index.contains('/')
        || index.contains('\\')
        || index.contains(':')
        || index.bytes().any(|b| b < 0x20 || b == 0x7F)
    {
        return Err(HttpCoreError::InvalidPath {
            reason: "index file must be a single plain filename",
        });
    }

    let mut components = Path::new(index).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(HttpCoreError::InvalidPath {
            reason: "index file must be a single normal path component",
        }),
    }
}

fn parse_safe_relative_path(
    raw_path: &str,
    decode_percent: bool,
) -> Result<PathBuf, HttpCoreError> {
    if raw_path.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(HttpCoreError::InvalidPath {
            reason: "path must not contain control characters",
        });
    }
    if raw_path.contains('\\') {
        return Err(HttpCoreError::InvalidPath {
            reason: "path must not contain backslashes",
        });
    }

    let trimmed = raw_path.strip_prefix('/').unwrap_or(raw_path);
    let mut rel_path = PathBuf::new();

    for raw_seg in trimmed.split('/') {
        if raw_seg.is_empty() {
            continue;
        }
        let decoded = if decode_percent {
            percent_decode_segment(raw_seg)
                .map_err(|reason| HttpCoreError::InvalidPath { reason })?
        } else {
            raw_seg.to_owned()
        };

        if decoded.contains('/')
            || decoded.contains('\\')
            || decoded.contains(':')
            || decoded.bytes().any(|b| b < 0x20 || b == 0x7F)
        {
            return Err(HttpCoreError::InvalidPath {
                reason: "path segment contains forbidden characters",
            });
        }
        if decoded == ".." {
            return Err(HttpCoreError::InvalidPath {
                reason: "parent directory traversal ('..') is not allowed",
            });
        }
        if decoded == "." {
            continue;
        }

        let mut components = Path::new(&decoded).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => rel_path.push(&decoded),
            _ => {
                return Err(HttpCoreError::InvalidPath {
                    reason: "path segment is not a normal relative component",
                });
            }
        }
    }

    Ok(rel_path)
}
