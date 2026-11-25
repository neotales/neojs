//! Package `"exports"`, `"imports"` (`#...`), self-reference, restricted
//! `"main"` fallback, and nearest-`package.json` ESM classification.
//!
//! Active conditions are fixed to `["import", "default"]` only, preserving
//! exact JSON object key insertion order from `package.json`.

use std::path::Path;

use indexmap::IndexMap;
use vaeljs_module_policy::{ModuleFormat, PackageType, package_module_format};

use crate::cas::{CasError, CasStore};
use crate::package_json::{ExportsField, PackageJson, PackageJsonError};
use crate::specifier::{
    PackageName, SpecifierError, split_bare_package_specifier, validate_subpath,
};
use crate::tarball::PackageTree;

/// Fixed active export/import conditions supported by `vaeljs`.
pub const ACTIVE_CONDITIONS: &[&str] = &["import", "default"];

/// Failure returned when resolving `exports`, `#imports`, `main`, or module format.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ExportsError {
    /// The requested subpath is not exported by the package's `"exports"` map.
    #[error("package `{package}` does not export subpath `{subpath}`")]
    PackagePathNotExported { package: String, subpath: String },

    /// The `#...` import specifier is not defined in `"imports"`.
    #[error("package `{package}` does not define import `{specifier}`")]
    PackageImportNotDefined { package: String, specifier: String },

    /// The `"exports"` or `"imports"` object has an invalid shape (such as mixed `.` and condition keys).
    #[error("invalid exports/imports configuration in `{package}`: {reason}")]
    InvalidConfig { package: String, reason: String },

    /// A resolved target string violated safety or normalization rules.
    #[error("invalid target `{target}` in `{package}`: {reason}")]
    InvalidTarget {
        package: String,
        target: String,
        reason: String,
    },

    /// The resolved target file does not exist in the package `.tree`.
    #[error("missing target file `{target}` in package `{package}`")]
    MissingTargetFile { package: String, target: String },

    /// The target resolved to an excluded native binary (`.node`, `.dll`, `.so`, `.dylib`).
    #[error("native binary `{target}` in package `{package}` is not supported")]
    ExcludedNativeBinary { package: String, target: String },

    /// The target file failed `vaeljs_module_policy::package_module_format` (e.g. CJS `.js` or `.cjs`).
    #[error("unsupported module `{path}` in package `{package}`: {reason}")]
    UnsupportedModuleFormat {
        package: String,
        path: String,
        reason: String,
    },

    /// Underlying CAS error when reading a nested `package.json`.
    #[error("CAS error while classifying package: {0}")]
    Cas(#[from] CasError),

    /// Invalid nested `package.json` encountered during nearest-type lookup.
    #[error("invalid nested package.json: {0}")]
    PackageJson(#[from] PackageJsonError),
}

/// Resolved internal file inside a package `.tree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPackageFile {
    /// Normalized package-relative path (no leading `./` or `/`).
    pub rel_path: String,
    /// Classified module format (`JavaScript` or `TypeScript`).
    pub format: ModuleFormat,
    /// Nearest `package.json` `"type"` governing this file.
    pub package_type: PackageType,
}

/// Resolution result for a `#...` package import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageImportTarget {
    /// Resolved to an internal file inside the same package.
    InternalFile(ResolvedPackageFile),
    /// Resolved to a declared external dependency (`pkg` or `pkg/sub`).
    ExternalPackage {
        /// External package name.
        name: PackageName,
        /// Optional exported subpath.
        subpath: Option<String>,
    },
}

/// Engine-neutral resolver for npm `exports`, `#imports`, `main`, and nearest
/// `package.json` ESM classification.
pub struct ExportsResolver;

impl ExportsResolver {
    /// Resolves a package entrypoint (`subpath = None`) or subpath (`subpath = Some("sub")`)
    /// against `pkg_json` and `tree`, verifying file existence and strict ESM classification.
    pub fn resolve_package_entry(
        tree: &PackageTree,
        cas: &CasStore,
        pkg_json: &PackageJson,
        pkg_name: &str,
        subpath: Option<&str>,
    ) -> Result<ResolvedPackageFile, ExportsError> {
        let rel_path = match &pkg_json.exports {
            Some(exports) => resolve_exports_field(exports, pkg_name, subpath)?,
            None => resolve_legacy_fallback(tree, pkg_json, pkg_name, subpath)?,
        };
        classify_tree_file(tree, cas, pkg_name, &rel_path)
    }

    /// Resolves an `"exports"` field to a normalized package-relative target
    /// path without inspecting a `.tree` archive (useful for project `package.json`
    /// self-references).
    pub fn resolve_exports_target(
        exports: &ExportsField,
        pkg_name: &str,
        subpath: Option<&str>,
    ) -> Result<String, ExportsError> {
        resolve_exports_field(exports, pkg_name, subpath)
    }

    /// Resolves a `#...` import specifier against a package's `pkg_json.imports`
    /// and verified `.tree`, classifying internal files via
    /// [`vaeljs_module_policy::package_module_format`].
    ///
    /// External targets (`"dep"` or `"dep/sub"`) are only allowed if `dep` is
    /// declared in `dependencies`, `optionalDependencies`, or `peerDependencies`.
    pub fn resolve_package_import(
        tree: &PackageTree,
        cas: &CasStore,
        pkg_json: &PackageJson,
        pkg_name: &str,
        specifier: &str,
    ) -> Result<PackageImportTarget, ExportsError> {
        resolve_import_impl(
            Some((tree, cas)),
            None,
            pkg_json,
            pkg_name,
            specifier,
            false,
        )
    }

    /// Resolves a `#...` import specifier against a project `package.json`,
    /// classifying local `./...` targets via
    /// [`vaeljs_module_policy::package_module_format`].
    ///
    /// External targets (`"dep"` or `"dep/sub"`) must be declared in the
    /// project `package.json`'s `dependencies`, `devDependencies`, or
    /// `optionalDependencies`.
    pub fn resolve_project_import(
        pkg_json: &PackageJson,
        project_root: &Path,
        pkg_name: &str,
        specifier: &str,
    ) -> Result<PackageImportTarget, ExportsError> {
        resolve_import_impl(
            None,
            Some(project_root),
            pkg_json,
            pkg_name,
            specifier,
            true,
        )
    }

    /// Determines the nearest `package.json` `"type"` for `rel_path` inside `tree`
    /// by walking ancestor directories up to the package root.
    pub fn nearest_package_type(
        tree: &PackageTree,
        cas: &CasStore,
        pkg_name: &str,
        rel_path: &str,
    ) -> Result<PackageType, ExportsError> {
        let mut dir = match rel_path.rsplit_once('/') {
            Some((parent, _)) => parent,
            None => "",
        };

        loop {
            let candidate = if dir.is_empty() {
                "package.json".to_owned()
            } else {
                format!("{dir}/package.json")
            };
            if let Some(entry) = tree.get_file(&candidate) {
                let bytes = cas.read_verified(&entry.sha256, entry.size)?;
                let parsed = PackageJson::parse_bytes(&bytes).map_err(|err| {
                    ExportsError::InvalidConfig {
                        package: pkg_name.to_owned(),
                        reason: format!("invalid `{candidate}`: {err}"),
                    }
                })?;
                return Ok(parsed.package_type);
            }
            if dir.is_empty() {
                break;
            }
            dir = match dir.rsplit_once('/') {
                Some((parent, _)) => parent,
                None => "",
            };
        }

        Ok(PackageType::Unspecified)
    }

    /// Validates that `rel_path` exists in `tree`, is not an excluded native
    /// binary, and passes [`vaeljs_module_policy::package_module_format`].
    pub fn classify_file(
        tree: &PackageTree,
        cas: &CasStore,
        pkg_name: &str,
        rel_path: &str,
    ) -> Result<ResolvedPackageFile, ExportsError> {
        classify_tree_file(tree, cas, pkg_name, rel_path)
    }
}

fn resolve_import_impl(
    package_store: Option<(&PackageTree, &CasStore)>,
    project_root: Option<&Path>,
    pkg_json: &PackageJson,
    pkg_name: &str,
    specifier: &str,
    allow_dev_deps: bool,
) -> Result<PackageImportTarget, ExportsError> {
    if !specifier.starts_with('#') || specifier == "#" || specifier.starts_with("#/") {
        return Err(ExportsError::PackageImportNotDefined {
            package: pkg_name.to_owned(),
            specifier: specifier.to_owned(),
        });
    }
    let Some(imports_map) = &pkg_json.imports else {
        return Err(ExportsError::PackageImportNotDefined {
            package: pkg_name.to_owned(),
            specifier: specifier.to_owned(),
        });
    };

    let target = resolve_map_lookup(imports_map, pkg_name, specifier, false)?.ok_or_else(|| {
        ExportsError::PackageImportNotDefined {
            package: pkg_name.to_owned(),
            specifier: specifier.to_owned(),
        }
    })?;

    if let Some(rel) = target.strip_prefix("./") {
        let clean = normalize_target_rel(pkg_name, &target, rel)?;
        if let Some((tree, cas)) = package_store {
            let resolved = classify_tree_file(tree, cas, pkg_name, &clean)?;
            return Ok(PackageImportTarget::InternalFile(resolved));
        }
        let classify_path = match project_root {
            Some(root) => root.join(&clean),
            None => Path::new(&clean).to_path_buf(),
        };
        let format =
            package_module_format(&classify_path, pkg_json.package_type).map_err(|reason| {
                ExportsError::UnsupportedModuleFormat {
                    package: pkg_name.to_owned(),
                    path: clean.clone(),
                    reason,
                }
            })?;
        return Ok(PackageImportTarget::InternalFile(ResolvedPackageFile {
            rel_path: clean,
            format,
            package_type: pkg_json.package_type,
        }));
    }

    if target.starts_with('/') || target.starts_with("../") {
        return Err(ExportsError::InvalidTarget {
            package: pkg_name.to_owned(),
            target,
            reason: "import target may not use `/` or `../`".to_owned(),
        });
    }

    let (ext_name, ext_subpath) =
        split_bare_package_specifier(&target).map_err(|err: SpecifierError| {
            ExportsError::InvalidTarget {
                package: pkg_name.to_owned(),
                target: target.clone(),
                reason: err.to_string(),
            }
        })?;
    let is_declared = if allow_dev_deps {
        pkg_json.is_declared_project_dependency(ext_name.as_str())
    } else {
        pkg_json.is_declared_dependency(ext_name.as_str())
    };
    if !is_declared {
        return Err(ExportsError::InvalidTarget {
            package: pkg_name.to_owned(),
            target,
            reason: format!(
                "external `#imports` target `{}` is not declared in dependencies/optionalDependencies/peerDependencies",
                ext_name
            ),
        });
    }
    Ok(PackageImportTarget::ExternalPackage {
        name: ext_name,
        subpath: ext_subpath,
    })
}

fn classify_tree_file(
    tree: &PackageTree,
    cas: &CasStore,
    pkg_name: &str,
    rel_path: &str,
) -> Result<ResolvedPackageFile, ExportsError> {
    if tree.is_skipped_native(rel_path) {
        return Err(ExportsError::ExcludedNativeBinary {
            package: pkg_name.to_owned(),
            target: rel_path.to_owned(),
        });
    }
    if tree.get_file(rel_path).is_none() {
        return Err(ExportsError::MissingTargetFile {
            package: pkg_name.to_owned(),
            target: rel_path.to_owned(),
        });
    }
    let package_type = ExportsResolver::nearest_package_type(tree, cas, pkg_name, rel_path)?;
    let format = package_module_format(Path::new(rel_path), package_type).map_err(|reason| {
        ExportsError::UnsupportedModuleFormat {
            package: pkg_name.to_owned(),
            path: rel_path.to_owned(),
            reason,
        }
    })?;
    Ok(ResolvedPackageFile {
        rel_path: rel_path.to_owned(),
        format,
        package_type,
    })
}

fn resolve_exports_field(
    exports: &ExportsField,
    pkg_name: &str,
    subpath: Option<&str>,
) -> Result<String, ExportsError> {
    let match_key = match subpath {
        None => ".".to_owned(),
        Some(sub) => {
            let clean = validate_subpath(sub).map_err(|err| ExportsError::InvalidTarget {
                package: pkg_name.to_owned(),
                target: sub.to_owned(),
                reason: err.to_string(),
            })?;
            format!("./{clean}")
        }
    };

    match exports {
        ExportsField::Null => Err(ExportsError::PackagePathNotExported {
            package: pkg_name.to_owned(),
            subpath: match_key,
        }),
        ExportsField::String(_) | ExportsField::Array(_) => {
            if subpath.is_some() {
                return Err(ExportsError::PackagePathNotExported {
                    package: pkg_name.to_owned(),
                    subpath: match_key,
                });
            }
            let target = resolve_target(exports, pkg_name, None, true)?.ok_or_else(|| {
                ExportsError::PackagePathNotExported {
                    package: pkg_name.to_owned(),
                    subpath: ".".to_owned(),
                }
            })?;
            let rel = target
                .strip_prefix("./")
                .ok_or_else(|| ExportsError::InvalidTarget {
                    package: pkg_name.to_owned(),
                    target: target.clone(),
                    reason: "export target must start with `./`".to_owned(),
                })?;
            normalize_target_rel(pkg_name, &target, rel)
        }
        ExportsField::Object(map) => {
            if map.is_empty() {
                return Err(ExportsError::PackagePathNotExported {
                    package: pkg_name.to_owned(),
                    subpath: match_key,
                });
            }
            let has_dot_keys = map.keys().any(|k| k.starts_with('.'));
            let has_cond_keys = map.keys().any(|k| !k.starts_with('.'));
            if has_dot_keys && has_cond_keys {
                return Err(ExportsError::InvalidConfig {
                    package: pkg_name.to_owned(),
                    reason: "exports object cannot mix subpath keys and condition keys".to_owned(),
                });
            }

            let target = if has_dot_keys {
                resolve_map_lookup(map, pkg_name, &match_key, true)?
            } else if subpath.is_none() {
                resolve_target(exports, pkg_name, None, true)?
            } else {
                None
            };

            let target = target.ok_or_else(|| ExportsError::PackagePathNotExported {
                package: pkg_name.to_owned(),
                subpath: match_key,
            })?;
            let rel = target
                .strip_prefix("./")
                .ok_or_else(|| ExportsError::InvalidTarget {
                    package: pkg_name.to_owned(),
                    target: target.clone(),
                    reason: "export target must start with `./`".to_owned(),
                })?;
            normalize_target_rel(pkg_name, &target, rel)
        }
    }
}

fn resolve_map_lookup(
    map: &IndexMap<String, ExportsField>,
    pkg_name: &str,
    match_key: &str,
    require_dot_slash_target: bool,
) -> Result<Option<String>, ExportsError> {
    if !match_key.contains('*')
        && let Some(exact) = map.get(match_key)
    {
        return resolve_target(exact, pkg_name, None, require_dot_slash_target);
    }

    let mut best: Option<(&str, &str, &ExportsField, &str)> = None;
    for (pattern_key, value) in map {
        if let Some((prefix, suffix)) = pattern_key.split_once('*') {
            if suffix.contains('*') {
                return Err(ExportsError::InvalidConfig {
                    package: pkg_name.to_owned(),
                    reason: format!("export pattern `{pattern_key}` contains multiple `*`"),
                });
            }
            if match_key.len() >= prefix.len() + suffix.len()
                && match_key.starts_with(prefix)
                && match_key.ends_with(suffix)
            {
                let submatch = &match_key[prefix.len()..match_key.len() - suffix.len()];
                let replace = match best {
                    None => true,
                    Some((best_prefix, best_suffix, _, _)) => {
                        prefix.len() > best_prefix.len()
                            || (prefix.len() == best_prefix.len()
                                && suffix.len() > best_suffix.len())
                    }
                };
                if replace {
                    best = Some((prefix, suffix, value, submatch));
                }
            }
        }
    }

    if let Some((_, _, target_field, submatch)) = best {
        if submatch.contains('\\')
            || submatch
                .split('/')
                .any(|seg| seg == "." || seg == ".." || seg.eq_ignore_ascii_case("node_modules"))
        {
            return Err(ExportsError::InvalidTarget {
                package: pkg_name.to_owned(),
                target: match_key.to_owned(),
                reason: "wildcard match contains traversal or `node_modules`".to_owned(),
            });
        }
        return resolve_target(
            target_field,
            pkg_name,
            Some(submatch),
            require_dot_slash_target,
        );
    }
    Ok(None)
}

enum TargetOutcome {
    Found(String),
    ExplicitNull,
    NoMatch,
}

fn resolve_target(
    target: &ExportsField,
    pkg_name: &str,
    pattern_match: Option<&str>,
    require_dot_slash: bool,
) -> Result<Option<String>, ExportsError> {
    match resolve_target_outcome(target, pkg_name, pattern_match, require_dot_slash)? {
        TargetOutcome::Found(s) => Ok(Some(s)),
        TargetOutcome::ExplicitNull | TargetOutcome::NoMatch => Ok(None),
    }
}

fn resolve_target_outcome(
    target: &ExportsField,
    pkg_name: &str,
    pattern_match: Option<&str>,
    require_dot_slash: bool,
) -> Result<TargetOutcome, ExportsError> {
    match target {
        ExportsField::Null => Ok(TargetOutcome::ExplicitNull),
        ExportsField::String(raw) => {
            let expanded = match pattern_match {
                Some(sub) => raw.replace('*', sub),
                None => raw.clone(),
            };
            if require_dot_slash && !expanded.starts_with("./") {
                return Err(ExportsError::InvalidTarget {
                    package: pkg_name.to_owned(),
                    target: expanded,
                    reason: "export target must start with `./`".to_owned(),
                });
            }
            Ok(TargetOutcome::Found(expanded))
        }
        ExportsField::Array(items) => {
            for item in items {
                match resolve_target_outcome(item, pkg_name, pattern_match, require_dot_slash) {
                    Ok(TargetOutcome::Found(found)) => return Ok(TargetOutcome::Found(found)),
                    Ok(TargetOutcome::ExplicitNull | TargetOutcome::NoMatch)
                    | Err(ExportsError::InvalidTarget { .. }) => continue,
                    Err(e) => return Err(e),
                }
            }
            Ok(TargetOutcome::NoMatch)
        }
        ExportsField::Object(conds) => {
            // Preserve exact JSON manifest key insertion order.
            for (cond, sub_target) in conds {
                if ACTIVE_CONDITIONS.contains(&cond.as_str()) {
                    match resolve_target_outcome(
                        sub_target,
                        pkg_name,
                        pattern_match,
                        require_dot_slash,
                    )? {
                        TargetOutcome::Found(found) => return Ok(TargetOutcome::Found(found)),
                        TargetOutcome::ExplicitNull => return Ok(TargetOutcome::ExplicitNull),
                        TargetOutcome::NoMatch => continue,
                    }
                }
            }
            Ok(TargetOutcome::NoMatch)
        }
    }
}

fn resolve_legacy_fallback(
    tree: &PackageTree,
    pkg_json: &PackageJson,
    pkg_name: &str,
    subpath: Option<&str>,
) -> Result<String, ExportsError> {
    if let Some(sub) = subpath {
        let clean = validate_subpath(sub).map_err(|err| ExportsError::InvalidTarget {
            package: pkg_name.to_owned(),
            target: sub.to_owned(),
            reason: err.to_string(),
        })?;
        return Ok(clean);
    }

    if let Some(module_entry) = &pkg_json.module {
        let stripped = module_entry
            .strip_prefix("./")
            .unwrap_or(module_entry.as_str());
        let clean = normalize_target_rel(pkg_name, module_entry, stripped)?;
        return Ok(clean);
    }

    if let Some(main) = &pkg_json.main {
        let stripped = main.strip_prefix("./").unwrap_or(main.as_str());
        let clean = normalize_target_rel(pkg_name, main, stripped)?;
        return Ok(clean);
    }

    if tree.get_file("index.mjs").is_some() {
        return Ok("index.mjs".to_owned());
    }
    if tree.get_file("index.js").is_some() {
        return Ok("index.js".to_owned());
    }
    Err(ExportsError::MissingTargetFile {
        package: pkg_name.to_owned(),
        target: "index.mjs / index.js".to_owned(),
    })
}

fn normalize_target_rel(
    pkg_name: &str,
    full_target: &str,
    rel: &str,
) -> Result<String, ExportsError> {
    validate_subpath(rel).map_err(|err| ExportsError::InvalidTarget {
        package: pkg_name.to_owned(),
        target: full_target.to_owned(),
        reason: err.to_string(),
    })
}
