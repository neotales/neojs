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

/// Resolves `url_path` safely beneath `root_dir`, optionally resolving
/// directories to `index_file`.
///
/// Security guarantees:
/// * Rejects raw or percent-encoded NUL bytes (`\0`, `%00`), backslashes
///   (`\`, `%5c`), encoded forward slashes (`%2f`), and `..` traversal segments
///   (including `%2e%2e`) with [`HttpCoreError::InvalidPath`].
/// * Validates `index_file` as a single plain filename with no separators or
///   traversal segments.
/// * Canonicalizes both `root_dir` and the resolved target (including any
///   `index_file`) and verifies `canonical_target.starts_with(&canonical_root)`
///   so symbolic links escaping `root_dir` are rejected with
///   [`HttpCoreError::InvalidPath`].
/// * Returns [`HttpCoreError::NotFound`] when the target does not exist, is not
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
/// use neojs_http_core::{HttpCoreErrorCode, resolve_static_file};
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

    let rel_path = parse_safe_relative_path(url_path)?;

    let canonical_root = fs::canonicalize(root_dir).map_err(map_io_error)?;
    if !canonical_root.is_dir() {
        return Err(HttpCoreError::NotFound);
    }

    let candidate = if rel_path.as_os_str().is_empty() {
        canonical_root.clone()
    } else {
        canonical_root.join(&rel_path)
    };

    let canonical_candidate = fs::canonicalize(&candidate).map_err(map_io_error)?;
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

    if !meta.is_file() {
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
/// use neojs_http_core::mime_type_for_path;
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
        || index.contains('\0')
        || index.contains('/')
        || index.contains('\\')
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

fn parse_safe_relative_path(url_path: &str) -> Result<PathBuf, HttpCoreError> {
    if url_path.contains('\0') {
        return Err(HttpCoreError::InvalidPath {
            reason: "URL path must not contain NUL bytes",
        });
    }
    if url_path.contains('\\') {
        return Err(HttpCoreError::InvalidPath {
            reason: "URL path must not contain backslashes",
        });
    }

    let trimmed = url_path.strip_prefix('/').unwrap_or(url_path);
    let mut rel_path = PathBuf::new();

    for raw_seg in trimmed.split('/') {
        if raw_seg.is_empty() {
            continue;
        }
        let decoded = percent_decode_segment(raw_seg)
            .map_err(|reason| HttpCoreError::InvalidPath { reason })?;

        if decoded.contains('\0') || decoded.contains('/') || decoded.contains('\\') {
            return Err(HttpCoreError::InvalidPath {
                reason: "decoded URL path segment contains forbidden characters",
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
                    reason: "URL path segment is not a normal relative component",
                });
            }
        }
    }

    Ok(rel_path)
}
