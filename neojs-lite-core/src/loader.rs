//! Module resolution and loading for local ECMAScript files and shared
//! `neo:` built-ins.
//!
//! The resolver turns a specifier into one canonical absolute path, which is
//! also the module identity. A canonical specifier listed by the engine-neutral
//! `neojs-builtins` crate resolves to itself instead; the loader declares the
//! matching native QuickJS module from that crate's Rust logic. The loader
//! accepts only formats that QuickJS can execute: `.js`, `.mjs`, and names
//! without an extension. The format decision comes from `neojs-module-policy`,
//! the same policy the V8 runtime uses.
//!
//! With the off-by-default `node-compat` feature, a runtime built with
//! `Runtime::with_node_compat` also resolves the exact `node:child_process`
//! specifier to the native QuickJS module from `neojs-lite-node`. It is never
//! read from disk, has no bare or `neo:` alias, and is not added to the `Neo`
//! registry. A default runtime rejects every `node:` specifier.

use std::path::{Path, PathBuf};

use neojs_module_policy::{ModuleFormat, is_path_specifier, module_format};
use rquickjs::{
    Ctx, Error, Module, Result,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use url::Url;

const BARE_SPECIFIER_REASON: &str = "bare module specifiers are not implemented in neojs-lite; use a relative path, an absolute path, or a file: URL";
const TYPE_ATTRIBUTE_REASON: &str =
    "import attributes are not implemented; neojs-lite loads ECMAScript modules only";

/// Returns the default-runtime rejection, naming the requested module.
fn node_disabled(specifier: &str) -> String {
    format!(
        "Node compatibility is disabled for `{specifier}`; create the runtime with Runtime::with_node_compat and an explicit process policy"
    )
}

/// Returns why an unsupported Node specifier is not resolved.
///
/// With the feature enabled, the shared crate owns the supported set. Without
/// it, the same message is produced locally so a default runtime never points a
/// Node specifier at a Neo module.
#[cfg(feature = "node-compat")]
fn node_unsupported(specifier: &str) -> String {
    neojs_lite_node::unsupported_module_reason(specifier)
}

#[cfg(not(feature = "node-compat"))]
fn node_unsupported(specifier: &str) -> String {
    format!(
        "the Node module `{specifier}` is not implemented; supported Node modules: node:child_process"
    )
}

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

/// Resolves relative paths, absolute paths, `file:` URLs, and shared `neo:`
/// built-ins to one canonical identity.
///
/// With the `node-compat` feature, a resolver built for an opted-in runtime
/// also accepts the exact `node:child_process` specifier. A resolver built for a
/// default runtime rejects every `node:` specifier with a reason. A bare Node
/// name is never accepted, even when the feature is enabled.
///
/// A canonical path is the module identity, so the same file reached by two
/// spellings is evaluated once. A shared built-in resolves to its canonical
/// `neo:` specifier, so static and dynamic imports also reach one instance.
/// Bare specifiers and every other URL scheme are rejected with a reason. The
/// returned name is always valid UTF-8 because QuickJS module names are strings.
pub(super) struct FileResolver {
    /// Whether this resolver belongs to a runtime built with Node compatibility.
    pub(super) node: bool,
}

impl Resolver for FileResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        reject_attributes(base, name, attributes)?;
        if name == "node:child_process" {
            if self.node {
                return Ok(name.to_owned());
            }
            return Err(Error::new_resolving_message(
                base,
                name,
                node_disabled(name),
            ));
        }
        if name.starts_with("node:") {
            if self.node {
                return Err(Error::new_resolving_message(
                    base,
                    name,
                    node_unsupported(name),
                ));
            }
            return Err(Error::new_resolving_message(
                base,
                name,
                node_disabled(name),
            ));
        }
        #[cfg(feature = "node-compat")]
        if self.node && neojs_lite_node::is_known_node_bare_name(name) {
            return Err(Error::new_resolving_message(
                base,
                name,
                format!(
                    "Node built-in modules must use the `node:` prefix; `{name}` is not resolved as a Node or Neo module"
                ),
            ));
        }
        // A shared built-in is a canonical `neo:` specifier. It is exact: the
        // shared crate does not accept a bare name, another scheme, a query, or
        // a fragment. Resolving to the same string gives the engine one module
        // identity for static and dynamic imports. With `neo-chars` off the
        // dependency is absent and this block is not compiled.
        #[cfg(feature = "neo-chars")]
        if let Some(builtin) = neojs_builtins::module(name) {
            return Ok(builtin.specifier.to_owned());
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
///
/// A loader built for an opted-in runtime serves the exact
/// `node:child_process` source from `neojs-lite-node`. It never reads that
/// module from disk. A default loader rejects every `node:` name before this
/// point, and this loader re-checks the opt-in rather than trusting the
/// resolver alone.
pub(super) struct FileLoader {
    /// Whether this loader belongs to a runtime built with Node compatibility.
    pub(super) node: bool,
}

impl Loader for FileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
        if name == "node:child_process" {
            if !self.node {
                return Err(Error::new_loading_message(name, node_disabled(name)));
            }
            #[cfg(feature = "node-compat")]
            return neojs_lite_node::module(ctx.clone(), name);
            #[cfg(not(feature = "node-compat"))]
            return Err(Error::new_loading_message(
                name,
                "the Node compatibility adapter is not compiled into this neojs-lite-core",
            ));
        }
        if name.starts_with("node:") {
            return Err(Error::new_loading_message(name, node_unsupported(name)));
        }

        // A shared canonical module is a native QuickJS module. The module
        // declaration is the only interface; its evaluation exports the function
        // objects created by `crate::chars`. No ECMAScript source is read.
        #[cfg(feature = "neo-chars")]
        if neojs_builtins::module(name).is_some() {
            return Module::declare_def::<crate::chars::CharsModule, _>(ctx.clone(), name);
        }
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
