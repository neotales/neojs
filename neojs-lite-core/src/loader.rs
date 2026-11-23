//! Module resolution and loading for local ECMAScript files.
//!
//! The resolver turns a file specifier into one canonical absolute path, which
//! is also the module identity. When the `crypto` feature is enabled, it also
//! maps `crypto` and `neo:crypto` (case-insensitive `neo:` scheme) to the
//! native module's canonical specifier, `neo:crypto`. With the feature off, it
//! rejects those two spellings with a reason that names the disabled feature
//! instead of reporting a generic bare-specifier failure.
//!
//! The loader accepts only formats that QuickJS can execute: `.js`, `.mjs`, and
//! names without an extension. The format decision comes from
//! `neojs-module-policy`, the same policy the V8 runtime uses. Built-in modules
//! are native QuickJS modules and do not pass through this loader.

use std::path::{Path, PathBuf};

use neojs_module_policy::{ModuleFormat, is_path_specifier, module_format};
use rquickjs::{
    Ctx, Error, Module, Result,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use url::Url;

use crate::builtins;

const BARE_SPECIFIER_REASON: &str = "bare module specifiers are not implemented in neojs-lite; use a relative path, an absolute path, or a file: URL";
const TYPE_ATTRIBUTE_REASON: &str =
    "import attributes are not implemented; neojs-lite loads ECMAScript modules only";
const CRYPTO_DISABLED_REASON: &str =
    "the `neo:crypto` built-in module is not enabled in this build; enable the `crypto` feature";

/// Rejects any non-empty import attribute bag.
///
/// QuickJS calls the resolver before the loader, so one check covers static and
/// dynamic imports. Ignoring `with { type: "json" }` would execute a file as
/// JavaScript when the importer asked for a different representation.
fn reject_attributes(
    base: &str,
    name: &str,
    attributes: Option<ImportAttributes<'_>>,
) -> Result<()> {
    let Some(attributes) = attributes else {
        return Ok(());
    };
    if attributes.keys().next().transpose()?.is_some() {
        return Err(Error::new_resolving_message(
            base,
            name,
            TYPE_ATTRIBUTE_REASON,
        ));
    }
    Ok(())
}

/// Resolves relative paths, absolute paths, and `file:` URLs to one canonical
/// absolute path.
///
/// The canonical path is the module identity, so the same file reached by two
/// spellings is evaluated once. Bare specifiers and every other URL scheme are
/// rejected with a reason. The returned name is always valid UTF-8 because
/// QuickJS module names are strings.
pub(super) struct FileResolver;

impl Resolver for FileResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        reject_attributes(base, name, attributes)?;
        if let Some(canonical) = builtins::resolve(name) {
            return Ok(canonical.to_owned());
        }
        if builtins::is_crypto_specifier(name) {
            return Err(Error::new_resolving_message(
                base,
                name,
                CRYPTO_DISABLED_REASON,
            ));
        }
        let path = candidate_path(base, name)?;
        let canonical = std::fs::canonicalize(&path).map_err(|error| {
            Error::new_resolving_message(
                base,
                name,
                format!("cannot resolve `{}`: {error}", path.display()),
            )
        })?;
        canonical.to_str().map(str::to_owned).ok_or_else(|| {
            Error::new_resolving_message(
                base,
                name,
                format!("resolved path `{}` is not valid UTF-8", canonical.display()),
            )
        })
    }
}

/// Returns the path a specifier names before canonicalization.
///
/// `base` is the importing module's canonical name. A relative specifier is
/// joined to the base's parent directory, never to the process working
/// directory, so imports stay stable when the caller's current directory
/// changes. A `file:` URL is converted to its local path.
fn candidate_path(base: &str, name: &str) -> Result<PathBuf> {
    if is_windows_drive_path(name) {
        return Ok(PathBuf::from(name));
    }

    if let Ok(url) = Url::parse(name) {
        return match url.scheme() {
            "file" => url.to_file_path().map_err(|()| {
                Error::new_resolving_message(
                    base,
                    name,
                    "file: URL does not name a local filesystem path",
                )
            }),
            scheme => Err(Error::new_resolving_message(
                base,
                name,
                format!(
                    "the `{scheme}:` scheme is not implemented in neojs-lite; use a relative path, an absolute path, or a file: URL"
                ),
            )),
        };
    }

    if !is_path_specifier(name) {
        return Err(Error::new_resolving_message(
            base,
            name,
            BARE_SPECIFIER_REASON,
        ));
    }

    let path = Path::new(name);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let directory = Path::new(base).parent().unwrap_or_else(|| Path::new("."));
    Ok(directory.join(path))
}

/// Returns whether a specifier is a Windows drive path such as `C:\main.js`.
///
/// `Url::parse` treats the drive letter as a URL scheme, so this check has to
/// run first on Windows. It is harmless on other targets.
fn is_windows_drive_path(specifier: &str) -> bool {
    let bytes = specifier.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Reads and declares an ECMAScript module for QuickJS.
///
/// TypeScript is classified by the shared policy and rejected here because this
/// runtime has no type stripper. Every unsupported extension carries the shared
/// policy's reason. Read errors name the resolved module.
pub(super) struct FileLoader;

impl Loader for FileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
        let path = Path::new(name);
        match module_format(path) {
            Ok(ModuleFormat::JavaScript) => {}
            Ok(ModuleFormat::TypeScript) => {
                return Err(Error::new_loading_message(
                    name,
                    "TypeScript is not implemented in neojs-lite yet; use a .js or .mjs file",
                ));
            }
            Err(reason) => return Err(Error::new_loading_message(name, reason)),
        }

        let source = std::fs::read(path).map_err(|error| {
            Error::new_loading_message(name, format!("cannot read file: {error}"))
        })?;
        Module::declare(ctx.clone(), name, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_candidates_use_the_importing_module_directory() {
        let path = candidate_path("/work/app/main.js", "./lib/greet.js").expect("candidate");
        assert_eq!(path, Path::new("/work/app/lib/greet.js"));
    }

    #[test]
    fn absolute_candidates_are_not_joined() {
        let path = candidate_path("/work/app/main.js", "/work/shared/greet.js").expect("candidate");
        assert_eq!(path, Path::new("/work/shared/greet.js"));
    }

    #[test]
    fn bare_specifiers_are_rejected() {
        let error = candidate_path("/work/app/main.js", "lodash").expect_err("bare specifier");
        let text = format!("{error:?}");
        assert!(text.contains("bare module specifiers"), "{text}");
    }

    #[test]
    fn windows_drive_paths_are_recognised() {
        assert!(is_windows_drive_path("C:\\work\\main.js"));
        assert!(is_windows_drive_path("c:/work/main.js"));
        assert!(!is_windows_drive_path("neo:assert"));
        assert!(!is_windows_drive_path("C:"));
    }
}
