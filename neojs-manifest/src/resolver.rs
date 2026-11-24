//! Import-map and workspace package resolution.
//!
//! Resolves import specifiers against a standalone [`DiscoveredManifest`] or an
//! expanded [`Workspace`] with an optional active member context. This module is
//! pure configuration and filesystem path resolution: it performs zero network
//! I/O and leaves remote/built-in policy enforcement to the runtime loader.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::specifier::{MAX_SUBPATH_BYTES, validate_module_path};
use crate::{
    DiscoveredManifest, Error, ExportKey, Exports, ImportMap, NormalizedSpecifier, PackageName,
    Workspace, normalize_specifier,
};

/// Canonical built-in module schemes that are never intercepted by manifest
/// `imports` or workspace package names.
const BUILTIN_SCHEMES: &[&str] = &["neo:", "node:"];

/// Outcome of resolving an import specifier through [`ImportResolver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedImport {
    /// A canonical built-in specifier (`neo:*` or `node:*`) that is never
    /// intercepted by `imports` or workspace packages.
    ///
    /// The caller's runtime loader enforces built-in availability and policy.
    Builtin(String),
    /// A local module path resolved against the declaring manifest's directory.
    Local(PathBuf),
    /// A normalized non-local (or `file:`) specifier target, preserving its
    /// [`NormalizedSpecifier`] and [`crate::Support`] status so the runtime
    /// enforces `--allow-remote` and unsupported-scheme gates.
    Specifier(NormalizedSpecifier),
    /// Neither `imports` nor any workspace package matched the specifier.
    Unmapped,
}

#[derive(Clone, Debug)]
struct ImportScope {
    dir: PathBuf,
    imports: ImportMap,
}

#[derive(Clone, Debug)]
struct MemberPackage {
    dir: PathBuf,
    exports: Option<Exports>,
}

/// Resolves import specifiers using a manifest or workspace configuration.
///
/// Resolution follows a single documented precedence order:
///
/// 1. Canonical built-in schemes (`neo:*`, `node:*`) are never intercepted and
///    return [`ResolvedImport::Builtin`].
/// 2. Exact match in the active member's `imports`, then the root's `imports`.
/// 3. Longest trailing-`/` prefix match in the active member's `imports`, then
///    the root's `imports` (rejecting path traversal `..` escaping the mapped
///    prefix).
/// 4. Workspace package match (`@scope/pkg` or `@scope/pkg/subpath`), resolved
///    strictly through the target member's `exports` (`.` or `./subpath`);
///    unexported subpaths fail with [`Error::UnexportedSubpath`].
/// 5. Otherwise [`ResolvedImport::Unmapped`].
///
/// Relative `./...` targets resolve against the **declaring manifest's
/// directory** (not the referrer's directory).
#[derive(Clone, Debug)]
pub struct ImportResolver {
    root: ImportScope,
    active: Option<ImportScope>,
    members: BTreeMap<PackageName, MemberPackage>,
}

impl ImportResolver {
    /// Builds a resolver from a standalone [`DiscoveredManifest`].
    pub fn from_manifest(discovered: &DiscoveredManifest) -> Self {
        Self {
            root: ImportScope {
                dir: discovered.dir.clone(),
                imports: discovered.manifest.imports.clone(),
            },
            active: None,
            members: BTreeMap::new(),
        }
    }

    /// Builds a resolver from an expanded [`Workspace`] and an optional active
    /// workspace member context.
    ///
    /// When `active_member` is `Some`, its `imports` take precedence over the
    /// workspace root's `imports`. When `None`, only the root's `imports` and
    /// workspace member packages are consulted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownWorkspaceMember`] when `active_member` is `Some`
    /// and does not name a member of `workspace`.
    pub fn from_workspace(
        workspace: &Workspace,
        active_member: Option<&PackageName>,
    ) -> Result<Self, Error> {
        let active = match active_member {
            Some(name) => {
                let member = workspace
                    .member(name)
                    .ok_or(Error::UnknownWorkspaceMember)?;
                Some(ImportScope {
                    dir: member.dir.clone(),
                    imports: member.manifest.imports.clone(),
                })
            }
            None => None,
        };

        let mut members = BTreeMap::new();
        for member in workspace.members() {
            members.insert(
                member.name().clone(),
                MemberPackage {
                    dir: member.dir.clone(),
                    exports: member.manifest.exports.clone(),
                },
            );
        }

        Ok(Self {
            root: ImportScope {
                dir: workspace.root_dir().to_path_buf(),
                imports: workspace.root_manifest().imports.clone(),
            },
            active,
            members,
        })
    }

    /// Resolves `specifier` according to the documented precedence order.
    ///
    /// # Errors
    ///
    /// Fails closed if a matched prefix import attempts path traversal (`..` or
    /// unsafe segments) outside the mapped prefix, if a workspace package import
    /// names an unexported or invalid subpath ([`Error::UnexportedSubpath`]),
    /// or if resolving a local target encounters a filesystem error (such as a
    /// symlink loop or dangling symlink).
    pub fn resolve(&self, specifier: &str) -> Result<ResolvedImport, Error> {
        if is_canonical_builtin(specifier) {
            return Ok(ResolvedImport::Builtin(specifier.to_owned()));
        }

        if !specifier.ends_with('/') {
            if let Some(active) = &self.active
                && let Some(target) = active.imports.get(specifier)
            {
                return resolve_exact_target(&active.dir, target);
            }
            if let Some(target) = self.root.imports.get(specifier) {
                return resolve_exact_target(&self.root.dir, target);
            }
        }

        if let Some(active) = &self.active
            && let Some((prefix, target)) = longest_prefix_match(&active.imports, specifier)
        {
            return resolve_prefix_target(&active.dir, prefix, target, specifier);
        }
        if let Some((prefix, target)) = longest_prefix_match(&self.root.imports, specifier) {
            return resolve_prefix_target(&self.root.dir, prefix, target, specifier);
        }

        if let Some(resolved) = self.resolve_workspace_package(specifier)? {
            return Ok(resolved);
        }

        Ok(ResolvedImport::Unmapped)
    }

    fn resolve_workspace_package(&self, specifier: &str) -> Result<Option<ResolvedImport>, Error> {
        if !specifier.starts_with('@') {
            return Ok(None);
        }
        let Some((scope_part, after_scope)) = specifier.split_once('/') else {
            return Ok(None);
        };
        let (pkg_raw, subpath) = match after_scope.split_once('/') {
            Some((name_part, rest)) => (format!("{scope_part}/{name_part}"), Some(rest)),
            None => (specifier.to_owned(), None),
        };
        let Ok(pkg_name) = PackageName::parse(&pkg_raw) else {
            return Ok(None);
        };
        let Some(member) = self.members.get(&pkg_name) else {
            return Ok(None);
        };

        let key_str = match subpath {
            None => ".".to_owned(),
            Some("") => return Err(Error::UnexportedSubpath),
            Some(rest) => format!("./{rest}"),
        };
        let export_key = ExportKey::parse(&key_str).map_err(|_| Error::UnexportedSubpath)?;

        let target = match &member.exports {
            None => return Err(Error::UnexportedSubpath),
            Some(Exports::Single(target)) => {
                if key_str == "." {
                    target
                } else {
                    return Err(Error::UnexportedSubpath);
                }
            }
            Some(Exports::Map(map)) => map.get(&export_key).ok_or(Error::UnexportedSubpath)?,
        };

        resolve_local_path(&member.dir, target.as_str())
            .map(|path| Some(ResolvedImport::Local(path)))
    }
}

fn is_canonical_builtin(specifier: &str) -> bool {
    BUILTIN_SCHEMES
        .iter()
        .any(|scheme| specifier.starts_with(scheme))
}

fn longest_prefix_match<'a>(
    imports: &'a ImportMap,
    specifier: &str,
) -> Option<(&'a str, &'a NormalizedSpecifier)> {
    let mut best: Option<(&'a str, &'a NormalizedSpecifier)> = None;
    for (key, target) in imports.iter() {
        if key.ends_with('/') && specifier.starts_with(key) {
            let replace = match best {
                None => true,
                Some((current, _)) => key.len() > current.len(),
            };
            if replace {
                best = Some((key, target));
            }
        }
    }
    best
}

fn resolve_exact_target(
    declaring_dir: &Path,
    target: &NormalizedSpecifier,
) -> Result<ResolvedImport, Error> {
    match target {
        NormalizedSpecifier::Path(relative) => {
            resolve_local_path(declaring_dir, relative).map(ResolvedImport::Local)
        }
        other => Ok(ResolvedImport::Specifier(other.clone())),
    }
}

fn resolve_prefix_target(
    declaring_dir: &Path,
    prefix: &str,
    target: &NormalizedSpecifier,
    specifier: &str,
) -> Result<ResolvedImport, Error> {
    let suffix = &specifier[prefix.len()..];
    if suffix.is_empty() {
        return Err(Error::InvalidSpecifier {
            reason: "a prefix import must name a subpath after the trailing `/`".to_owned(),
        });
    }
    if suffix.len() > MAX_SUBPATH_BYTES {
        return Err(Error::InvalidSpecifier {
            reason: format!("the import subpath is over the {MAX_SUBPATH_BYTES}-byte limit"),
        });
    }
    if suffix
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
        || suffix.contains('\\')
    {
        return Err(Error::InvalidSpecifier {
            reason: "the import subpath contains disallowed characters".to_owned(),
        });
    }

    if let NormalizedSpecifier::Https(prefix_url) = target {
        validate_https_prefix_suffix(suffix)?;
        let combined = format!("{prefix_url}{suffix}");
        return normalize_specifier(&combined).map(ResolvedImport::Specifier);
    }

    validate_module_path(suffix, MAX_SUBPATH_BYTES, "the import subpath")?;

    match target {
        NormalizedSpecifier::Path(prefix_path) => {
            let combined = format!("{prefix_path}{suffix}");
            let _ = normalize_specifier(&combined)?;
            resolve_local_path(declaring_dir, &combined).map(ResolvedImport::Local)
        }
        NormalizedSpecifier::Gh(gh) => {
            let combined = format!(
                "gh:{}/{}@{}/{}{}",
                gh.owner(),
                gh.repo(),
                gh.reference(),
                gh.path(),
                suffix
            );
            normalize_specifier(&combined).map(ResolvedImport::Specifier)
        }
        NormalizedSpecifier::Jsr(jsr) => {
            let combined = format!("{}{suffix}", jsr.canonical());
            normalize_specifier(&combined).map(ResolvedImport::Specifier)
        }
        NormalizedSpecifier::Npm(npm) => {
            let combined = format!("{}{suffix}", npm.canonical());
            normalize_specifier(&combined).map(ResolvedImport::Specifier)
        }
        NormalizedSpecifier::FileUrl(file_url) => {
            let combined = format!("{file_url}{suffix}");
            normalize_specifier(&combined).map(ResolvedImport::Specifier)
        }
        NormalizedSpecifier::Node(node_prefix) => {
            let combined = format!("{node_prefix}{suffix}");
            normalize_specifier(&combined).map(ResolvedImport::Specifier)
        }
        NormalizedSpecifier::Https(_) | NormalizedSpecifier::Bare(_) => {
            unreachable!("https and bare targets are handled before matching")
        }
    }
}

fn validate_https_prefix_suffix(suffix: &str) -> Result<(), Error> {
    let path_part = suffix.split(['?', '#']).next().unwrap_or("");
    if path_part.is_empty() {
        return Err(Error::InvalidSpecifier {
            reason: "the import subpath cannot be empty before `?` or `#`".to_owned(),
        });
    }
    let lower = path_part.to_ascii_lowercase();
    if lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return Err(Error::InvalidSpecifier {
            reason: "the import subpath contains an encoded traversal or separator".to_owned(),
        });
    }
    for segment in path_part.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(Error::InvalidSpecifier {
                reason: "the import subpath has an empty, `.`, or `..` segment".to_owned(),
            });
        }
    }
    Ok(())
}

fn resolve_local_path(declaring_dir: &Path, relative: &str) -> Result<PathBuf, Error> {
    let rel = relative.strip_prefix("./").unwrap_or(relative);
    let joined = if rel.is_empty() {
        declaring_dir.to_path_buf()
    } else {
        declaring_dir.join(rel)
    };

    match std::fs::canonicalize(&joined) {
        Ok(canonical) => Ok(canonical),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            // Fail closed on a dangling symlink so it cannot masquerade as an
            // absent file.
            if std::fs::symlink_metadata(&joined).is_ok() {
                return Err(Error::Io { source: err });
            }
            // If the target file does not yet exist on disk, canonicalize the
            // deepest existing ancestor so symlinked parent directories remain
            // normalized.
            let mut missing = Vec::new();
            let mut probe = joined.as_path();
            loop {
                match std::fs::canonicalize(probe) {
                    Ok(mut base) => {
                        for component in missing.into_iter().rev() {
                            base.push(component);
                        }
                        return Ok(base);
                    }
                    Err(probe_err) if probe_err.kind() == ErrorKind::NotFound => {
                        let Some(name) = probe.file_name() else {
                            return Ok(joined);
                        };
                        missing.push(name.to_os_string());
                        let Some(parent) = probe.parent() else {
                            return Ok(joined);
                        };
                        probe = parent;
                    }
                    Err(source) => return Err(Error::Io { source }),
                }
            }
        }
        Err(source) => Err(Error::Io { source }),
    }
}
