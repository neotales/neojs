//! Bounded upward manifest discovery.
//!
//! Walks upward from a starting directory looking for [`MANIFEST_FILE_NAME`],
//! bounded by an optional stop directory, [`MAX_DISCOVERY_DEPTH`], and a
//! canonicalized visited-directory set to prevent symlink loops.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{Error, MANIFEST_FILE_NAME, Manifest};

/// Maximum number of directories inspected during upward manifest discovery
/// (including the starting directory).
pub const MAX_DISCOVERY_DEPTH: usize = 64;

/// A validated manifest located on disk together with its canonical paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredManifest {
    /// Canonical path to the manifest file.
    pub path: PathBuf,
    /// Canonical directory containing the manifest file.
    pub dir: PathBuf,
    /// Parsed and validated manifest contents.
    pub manifest: Manifest,
}

/// Searches upward from `start` for [`MANIFEST_FILE_NAME`].
///
/// At most [`MAX_DISCOVERY_DEPTH`] directories are inspected (starting with
/// `start`). When `stop_dir` is provided, discovery inspects directories up to
/// and including `stop_dir` and never walks above it; if `start` is outside
/// `stop_dir`, discovery returns `Ok(None)` without reading any parent.
///
/// Tracks canonicalized visited directories to avoid symlink loops.
///
/// # Errors
///
/// Fails closed if `start` or `stop_dir` cannot be resolved as a directory, if
/// a directory entry cannot be inspected, or if a candidate manifest exists
/// and is malformed, oversized (over [`crate::MAX_MANIFEST_BYTES`]), or
/// unreadable.
pub fn discover_manifest(
    start: impl AsRef<Path>,
    stop_dir: Option<&Path>,
) -> Result<Option<DiscoveredManifest>, Error> {
    let start_dir = canonical_directory(start.as_ref())?;
    let canonical_stop = stop_dir.map(canonical_directory).transpose()?;

    if let Some(ref stop) = canonical_stop
        && !start_dir.starts_with(stop)
    {
        return Ok(None);
    }

    let mut current = start_dir;
    let mut visited = HashSet::new();

    for _ in 0..MAX_DISCOVERY_DEPTH {
        let dir = std::fs::canonicalize(&current).map_err(|source| Error::Io { source })?;
        if !visited.insert(dir.clone()) {
            break;
        }
        if let Some(ref stop) = canonical_stop
            && !dir.starts_with(stop)
        {
            break;
        }

        let candidate = dir.join(MANIFEST_FILE_NAME);
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                let manifest = Manifest::from_path(&candidate)?;
                let path =
                    std::fs::canonicalize(&candidate).map_err(|source| Error::Io { source })?;
                let manifest_dir = path.parent().map(Path::to_path_buf).unwrap_or(dir);
                return Ok(Some(DiscoveredManifest {
                    path,
                    dir: manifest_dir,
                    manifest,
                }));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { source }),
        }

        if canonical_stop.as_ref() == Some(&dir) {
            break;
        }
        let Some(parent) = dir.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }

    Ok(None)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, Error> {
    let canonical = std::fs::canonicalize(path).map_err(|source| Error::Io { source })?;
    if !canonical.is_dir() {
        return Err(Error::Io {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "discovery boundary path is not a directory",
            ),
        });
    }
    Ok(canonical)
}
