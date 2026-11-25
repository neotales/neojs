//! Current process state helpers backing `node:process`.
//!
//! All operations avoid `unsafe` code and handle non-UTF-8 environment variables
//! gracefully without panicking.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use crate::error::{NodeCoreError, map_io_error};

static HRTIME_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Returns the current working directory of the process.
///
/// # Example
///
/// ```
/// use neojs_node_core::process::cwd;
///
/// let dir = cwd().unwrap();
/// assert!(dir.is_absolute());
/// ```
pub fn cwd() -> Result<PathBuf, NodeCoreError> {
    std::env::current_dir().map_err(|err| map_io_error("get current working directory", err))
}

/// Changes the current working directory of the process.
///
/// Rejects empty paths and paths containing NUL bytes before delegating to
/// [`std::env::set_current_dir`].
///
/// # Example
///
/// ```
/// use neojs_node_core::process::{chdir, cwd};
///
/// let original = cwd().unwrap();
/// chdir(&original).unwrap();
/// assert_eq!(cwd().unwrap(), original);
/// ```
pub fn chdir(path: impl AsRef<Path>) -> Result<(), NodeCoreError> {
    let path = path.as_ref();
    let os = path.as_os_str();
    if os.is_empty() {
        return Err(NodeCoreError::InvalidPath {
            reason: "directory path must not be empty",
        });
    }
    if os.as_encoded_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidPath {
            reason: "directory path must not contain a NUL byte",
        });
    }
    std::env::set_current_dir(path).map_err(|err| map_io_error("change working directory", err))
}

/// Returns a deterministic snapshot of UTF-8 environment `(key, value)` pairs.
///
/// Uses [`std::env::vars_os`] and skips entries whose key or value is not valid
/// UTF-8 rather than panicking, then sorts entries by key.
///
/// # Example
///
/// ```
/// use neojs_node_core::process::env_entries;
///
/// let entries = env_entries();
/// assert!(entries.windows(2).all(|w| w[0].0 <= w[1].0));
/// ```
#[must_use]
pub fn env_entries() -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    entries
}

/// Looks up a single environment variable by `key`.
///
/// Rejects empty keys and keys containing `=` or NUL bytes with
/// [`NodeCoreError::InvalidInput`]. Returns `Ok(None)` if unset, or
/// [`NodeCoreError::InvalidUtf8`] if the value is not valid UTF-8.
///
/// # Example
///
/// ```
/// use neojs_node_core::process::env_get;
///
/// let missing = env_get("NEOJS_NODE_CORE_DEFINITELY_UNSET_VAR_12345").unwrap();
/// assert_eq!(missing, None);
/// ```
pub fn env_get(key: &str) -> Result<Option<String>, NodeCoreError> {
    if key.is_empty() {
        return Err(NodeCoreError::InvalidInput {
            reason: "environment variable key must not be empty",
        });
    }
    if key.contains('\0') || key.contains('=') {
        return Err(NodeCoreError::InvalidInput {
            reason: "environment variable key must not contain '=' or NUL",
        });
    }
    match std::env::var_os(key) {
        None => Ok(None),
        Some(val) => val
            .into_string()
            .map(Some)
            .map_err(|_| NodeCoreError::InvalidUtf8 {
                reason: "environment variable value is not valid UTF-8",
            }),
    }
}

/// Returns the operating-system process identifier of the current process.
///
/// # Example
///
/// ```
/// use neojs_node_core::process::pid;
///
/// assert!(pid() > 0);
/// ```
#[must_use]
pub fn pid() -> u32 {
    std::process::id()
}

/// Returns monotonic nanoseconds elapsed since an internal process epoch.
///
/// Backed by [`std::time::Instant`], suitable for high-resolution timing
/// (`process.hrtime.bigint()`).
///
/// # Example
///
/// ```
/// use neojs_node_core::process::hrtime_nanos;
///
/// let t1 = hrtime_nanos();
/// let t2 = hrtime_nanos();
/// assert!(t2 >= t1);
/// ```
#[must_use]
pub fn hrtime_nanos() -> u128 {
    let epoch = HRTIME_EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NodeCoreErrorCode;

    #[test]
    fn env_get_validates_keys() {
        assert_eq!(
            env_get("").unwrap_err().code(),
            NodeCoreErrorCode::InvalidInput
        );
        assert_eq!(
            env_get("A=B").unwrap_err().code(),
            NodeCoreErrorCode::InvalidInput
        );
        assert_eq!(
            env_get("A\0B").unwrap_err().code(),
            NodeCoreErrorCode::InvalidInput
        );
    }

    #[test]
    fn chdir_validates_empty_and_nul() {
        assert_eq!(
            chdir("").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
        assert_eq!(
            chdir("bad\0dir").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
    }
}
