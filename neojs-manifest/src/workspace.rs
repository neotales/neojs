//! Bounded workspace expansion and member indexing.
//!
//! Expands a root manifest's `workspaces` globs deterministically, validates
//! canonical containment within the workspace root, loads every member's
//! manifest, and indexes members by [`PackageName`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use glob::{MatchOptions, glob_with};

use crate::{Error, Exports, MANIFEST_FILE_NAME, Manifest, PackageName};

/// Maximum number of workspace members accepted by [`Workspace::load`].
pub const MAX_WORKSPACE_MEMBERS: usize = 256;

/// Upper bound on total filesystem entries visited across all workspace globs
/// so a broad recursive pattern cannot force unbounded directory traversal.
const MAX_WORKSPACE_GLOB_ENTRIES: usize = 4096;

/// One validated workspace member package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceMember {
    /// Canonical path to the member's manifest file.
    pub manifest_path: PathBuf,
    /// Canonical directory containing the member manifest.
    pub dir: PathBuf,
    /// Parsed and validated member manifest.
    pub manifest: Manifest,
}

impl WorkspaceMember {
    /// Returns the member's scoped package name.
    pub fn name(&self) -> &PackageName {
        &self.manifest.name
    }

    /// Returns the member's declared `exports`, if any.
    pub fn exports(&self) -> Option<&Exports> {
        self.manifest.exports.as_ref()
    }
}

/// An expanded, validated workspace rooted at a [`Manifest`].
///
/// Members are expanded deterministically, sorted by canonical directory path,
/// and indexed by [`PackageName`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root_manifest_path: PathBuf,
    root_dir: PathBuf,
    root_manifest: Manifest,
    members: BTreeMap<PackageName, WorkspaceMember>,
    member_order: Vec<PackageName>,
}

impl Workspace {
    /// Loads and validates a workspace from its root manifest path.
    ///
    /// Expands every pattern in `root_manifest.workspaces` deterministically
    /// (sorted by canonical path, deduplicated, bounded by
    /// [`MAX_WORKSPACE_MEMBERS`]).
    ///
    /// # Errors
    ///
    /// Rejects:
    /// - unreadable, oversized, or malformed root or member manifests,
    /// - member paths or symlinks escaping (or pointing back to) the workspace
    ///   root ([`Error::WorkspaceEscape`]),
    /// - missing member directories or manifests
    ///   ([`Error::MissingMemberManifest`]),
    /// - more than [`MAX_WORKSPACE_MEMBERS`] expanded members
    ///   ([`Error::WorkspaceTooManyMembers`]),
    /// - members declaring `workspaces` ([`Error::NestedWorkspace`]),
    /// - duplicate `@scope/name` package names across the root and members
    ///   ([`Error::DuplicateWorkspacePackage`]).
    pub fn load(root_manifest_path: impl AsRef<Path>) -> Result<Self, Error> {
        let root_manifest = Manifest::from_path(root_manifest_path.as_ref())?;
        let canonical_root_manifest = std::fs::canonicalize(root_manifest_path.as_ref())
            .map_err(|source| Error::Io { source })?;
        let root_dir = canonical_root_manifest
            .parent()
            .ok_or_else(|| Error::Io {
                source: std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "root manifest path has no parent directory",
                ),
            })?
            .to_path_buf();

        let member_dirs = expand_member_dirs(&root_dir, &root_manifest)?;

        let mut seen_names = BTreeSet::new();
        seen_names.insert(root_manifest.name.clone());

        let mut members = BTreeMap::new();
        let mut member_order = Vec::with_capacity(member_dirs.len());

        for member_dir in member_dirs {
            let manifest_path = member_dir.join(MANIFEST_FILE_NAME);
            match std::fs::symlink_metadata(&manifest_path) {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    return Err(Error::MissingMemberManifest);
                }
                Err(source) => return Err(Error::Io { source }),
            }

            let canonical_manifest = canonicalize_contained(&manifest_path, &root_dir)?;
            let manifest = Manifest::from_path(&canonical_manifest)?;

            if !manifest.workspaces.is_empty() {
                return Err(Error::NestedWorkspace);
            }
            if !seen_names.insert(manifest.name.clone()) {
                return Err(Error::DuplicateWorkspacePackage);
            }

            let name = manifest.name.clone();
            member_order.push(name.clone());
            members.insert(
                name,
                WorkspaceMember {
                    manifest_path: canonical_manifest,
                    dir: member_dir,
                    manifest,
                },
            );
        }

        Ok(Self {
            root_manifest_path: canonical_root_manifest,
            root_dir,
            root_manifest,
            members,
            member_order,
        })
    }

    /// Returns the canonical path to the workspace root manifest.
    pub fn root_manifest_path(&self) -> &Path {
        &self.root_manifest_path
    }

    /// Returns the canonical directory of the workspace root.
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Returns the parsed root manifest.
    pub fn root_manifest(&self) -> &Manifest {
        &self.root_manifest
    }

    /// Looks up a workspace member by its [`PackageName`].
    pub fn member(&self, name: &PackageName) -> Option<&WorkspaceMember> {
        self.members.get(name)
    }

    /// Iterates over workspace members in ascending canonical directory order.
    pub fn members(&self) -> impl Iterator<Item = &WorkspaceMember> {
        self.member_order
            .iter()
            .filter_map(|name| self.members.get(name))
    }

    /// Returns the number of expanded workspace members (excluding the root).
    pub fn member_count(&self) -> usize {
        self.member_order.len()
    }

    /// Finds the workspace member whose canonical directory contains `path`,
    /// preferring the deepest matching member directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if `path` cannot be canonicalized.
    pub fn member_for_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<Option<&WorkspaceMember>, Error> {
        let canonical =
            std::fs::canonicalize(path.as_ref()).map_err(|source| Error::Io { source })?;
        let mut best: Option<&WorkspaceMember> = None;
        for member in self.members() {
            if canonical.starts_with(&member.dir) {
                let replace = match best {
                    None => true,
                    Some(current) => member.dir.as_os_str().len() > current.dir.as_os_str().len(),
                };
                if replace {
                    best = Some(member);
                }
            }
        }
        Ok(best)
    }
}

fn expand_member_dirs(
    root_dir: &Path,
    root_manifest: &Manifest,
) -> Result<BTreeSet<PathBuf>, Error> {
    let mut member_dirs = BTreeSet::new();
    let mut visited_entries = 0usize;

    let root_str = root_dir.to_str().ok_or_else(|| Error::Io {
        source: std::io::Error::new(
            ErrorKind::InvalidInput,
            "workspace root path is not valid UTF-8",
        ),
    })?;
    let escaped_root = glob::Pattern::escape(root_str);
    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };

    for pattern in &root_manifest.workspaces {
        let raw = pattern.as_str();
        if !has_glob_metacharacter(raw) {
            let candidate = root_dir.join(raw);
            match std::fs::symlink_metadata(&candidate) {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    return Err(Error::MissingMemberManifest);
                }
                Err(source) => return Err(Error::Io { source }),
            }
            let canonical = canonicalize_contained(&candidate, root_dir)?;
            if !canonical.is_dir() {
                return Err(Error::MissingMemberManifest);
            }
            insert_bounded(&mut member_dirs, canonical)?;
            continue;
        }

        let full_pattern = format!("{escaped_root}/{raw}");
        let matches = glob_with(&full_pattern, options).map_err(|_| Error::InvalidField {
            field: "workspaces",
            reason: "workspace glob pattern is invalid".to_owned(),
        })?;

        for entry in matches {
            visited_entries += 1;
            if visited_entries > MAX_WORKSPACE_GLOB_ENTRIES {
                return Err(Error::WorkspaceTooManyMembers {
                    limit: MAX_WORKSPACE_MEMBERS,
                });
            }
            let path = entry.map_err(|error| Error::Io {
                source: error.into(),
            })?;
            let canonical = canonicalize_contained(&path, root_dir)?;
            if canonical.is_dir() {
                insert_bounded(&mut member_dirs, canonical)?;
            }
        }
    }

    Ok(member_dirs)
}

fn insert_bounded(member_dirs: &mut BTreeSet<PathBuf>, dir: PathBuf) -> Result<(), Error> {
    member_dirs.insert(dir);
    if member_dirs.len() > MAX_WORKSPACE_MEMBERS {
        return Err(Error::WorkspaceTooManyMembers {
            limit: MAX_WORKSPACE_MEMBERS,
        });
    }
    Ok(())
}

fn canonicalize_contained(path: &Path, root_dir: &Path) -> Result<PathBuf, Error> {
    let canonical = std::fs::canonicalize(path).map_err(|source| Error::Io { source })?;
    if !canonical.starts_with(root_dir) || canonical == root_dir {
        return Err(Error::WorkspaceEscape);
    }
    Ok(canonical)
}

fn has_glob_metacharacter(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|character| matches!(character, '*' | '?' | '['))
}
