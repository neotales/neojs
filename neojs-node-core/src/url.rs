//! `file:` URL to filesystem path conversions backing `node:url`.
//!
//! Enforces strict scheme, host, NUL, and percent-encoded separator checks so
//! `file:` URLs cannot smuggle remote hosts on POSIX or path separators via
//! `%2f`/`%5c`.

use std::path::{Path, PathBuf};

use url::Url;

use crate::error::NodeCoreError;

/// Maximum input byte length accepted by [`file_url_to_path`] and
/// [`path_to_file_url`] (`64 KiB`).
pub const MAX_URL_BYTES: usize = 64 * 1024;

/// Converts a `file:` URL string into a local [`PathBuf`].
///
/// Rejects:
/// * empty strings, strings exceeding [`MAX_URL_BYTES`], or strings with NUL (`\0` / `%00`)
/// * non-`file:` URLs
/// * non-local hosts on POSIX (anything other than empty or `"localhost"`)
/// * percent-encoded path separators (`%2f`, `%2F`, `%5c`, `%5C`) in the URL path
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use neojs_node_core::url::{file_url_to_path, path_to_file_url};
///
/// if cfg!(windows) {
///     let p = file_url_to_path("file:///C:/hello%20world.txt").unwrap();
///     assert_eq!(p, Path::new("C:\\hello world.txt"));
/// } else {
///     let p = file_url_to_path("file:///tmp/hello%20world.txt").unwrap();
///     assert_eq!(p, Path::new("/tmp/hello world.txt"));
///     assert_eq!(path_to_file_url(&p).unwrap(), "file:///tmp/hello%20world.txt");
/// }
/// ```
pub fn file_url_to_path(url_str: &str) -> Result<PathBuf, NodeCoreError> {
    if url_str.is_empty() {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must not be empty",
        });
    }
    if url_str.len() > MAX_URL_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_URL_BYTES,
            actual: Some(url_str.len()),
        });
    }
    if url_str.as_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must not contain a NUL byte",
        });
    }

    let parsed = Url::parse(url_str).map_err(|_| NodeCoreError::InvalidUrl {
        reason: "malformed URL",
    })?;

    if parsed.scheme() != "file" {
        return Err(NodeCoreError::InvalidUrl {
            reason: "URL scheme must be 'file:'",
        });
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must not contain userinfo credentials",
        });
    }

    let raw_path = parsed.path();
    if contains_disallowed_percent_escape(raw_path) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL path must not contain encoded NUL or path separators",
        });
    }
    if is_bare_drive_url_path(raw_path) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL with a Windows drive letter must include a path separator after ':'",
        });
    }

    #[cfg(not(windows))]
    {
        if let Some(host) = parsed.host_str()
            && !host.is_empty()
            && !host.eq_ignore_ascii_case("localhost")
        {
            return Err(NodeCoreError::InvalidUrl {
                reason: "file URL host must be empty or localhost on POSIX",
            });
        }
    }

    let path = parsed
        .to_file_path()
        .map_err(|()| NodeCoreError::InvalidUrl {
            reason: "file URL could not be converted to a local filesystem path",
        })?;

    if !path.is_absolute() {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must resolve to an absolute local path",
        });
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "decoded file path must not contain a NUL byte",
        });
    }

    Ok(path)
}

/// Converts an absolute local [`Path`] into a canonical `file:` URL string.
///
/// Rejects empty paths, relative paths, paths containing NUL bytes, and paths
/// exceeding [`MAX_URL_BYTES`].
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use neojs_node_core::url::path_to_file_url;
///
/// if cfg!(windows) {
///     assert_eq!(path_to_file_url(Path::new("C:\\a b\\c.txt")).unwrap(), "file:///C:/a%20b/c.txt");
/// } else {
///     assert_eq!(path_to_file_url(Path::new("/a b/c.txt")).unwrap(), "file:///a%20b/c.txt");
/// }
/// ```
pub fn path_to_file_url(path: impl AsRef<Path>) -> Result<String, NodeCoreError> {
    let path = path.as_ref();
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() {
        return Err(NodeCoreError::InvalidPath {
            reason: "path must not be empty",
        });
    }
    if bytes.len() > MAX_URL_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_URL_BYTES,
            actual: Some(bytes.len()),
        });
    }
    if bytes.contains(&0) {
        return Err(NodeCoreError::InvalidPath {
            reason: "path must not contain a NUL byte",
        });
    }
    if !path.is_absolute() {
        return Err(NodeCoreError::InvalidPath {
            reason: "path_to_file_url requires an absolute path",
        });
    }

    let url = Url::from_file_path(path).map_err(|()| NodeCoreError::InvalidPath {
        reason: "path could not be represented as a file URL",
    })?;
    Ok(url.into())
}

fn is_bare_drive_url_path(raw_path: &str) -> bool {
    let bytes = raw_path.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'/' || !bytes[1].is_ascii_alphabetic() {
        return false;
    }
    if bytes[2] == b':' {
        return bytes.get(3).copied() != Some(b'/');
    }
    if bytes.len() >= 5
        && bytes[2] == b'%'
        && bytes[3] == b'3'
        && bytes[4].eq_ignore_ascii_case(&b'a')
    {
        return bytes.get(5).copied() != Some(b'/');
    }
    false
}

fn contains_disallowed_percent_escape(raw_path: &str) -> bool {
    let bytes = raw_path.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes[i + 1].to_ascii_lowercase();
            let lo = bytes[i + 2].to_ascii_lowercase();
            if matches!((hi, lo), (b'0', b'0') | (b'2', b'f') | (b'5', b'c')) {
                return true;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NodeCoreErrorCode;

    #[test]
    fn rejects_non_file_schemes_and_encoded_separators() {
        assert_eq!(
            file_url_to_path("https://example.com/a")
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%2fb").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%2Fb").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%5cb").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%00b").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///C:").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///c:rel").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///C%3A").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            path_to_file_url("relative/file.txt").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn posix_accepts_localhost_and_rejects_remote_host() {
        assert_eq!(
            file_url_to_path("file://localhost/tmp/test.txt").unwrap(),
            Path::new("/tmp/test.txt")
        );
        assert_eq!(
            file_url_to_path("file://remote.host/tmp/test.txt")
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidUrl
        );
    }
}
