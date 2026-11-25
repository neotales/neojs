//! Module resolution and loading for local ECMAScript files and shared
//! `vael:` built-ins.
//!
//! The resolver turns a specifier into one canonical absolute path, which is
//! also the module identity. A canonical specifier listed by the engine-neutral
//! `vaeljs-builtins` crate resolves to itself instead; the loader declares the
//! matching native QuickJS module from that crate's Rust logic. The loader
//! accepts only formats that QuickJS can execute: `.js`, `.mjs`, and names
//! without an extension. The format decision comes from `vaeljs-module-policy`,
//! the same policy the V8 runtime uses.

use std::path::{Path, PathBuf};

use rquickjs::{
    Ctx, Error, Module, Result,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use url::Url;
#[cfg(feature = "manifest")]
use vaeljs_manifest::{ImportResolver, NormalizedSpecifier, ResolvedImport};
use vaeljs_module_policy::{ModuleFormat, is_path_specifier, module_format};

const BARE_SPECIFIER_REASON: &str = "bare module specifiers are not implemented in vaeljs-lite; use a relative path, an absolute path, or a file: URL";
const TYPE_ATTRIBUTE_REASON: &str =
    "import attributes are not implemented; vaeljs-lite loads ECMAScript modules only";

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

/// Resolves relative paths, absolute paths, `file:` URLs, optional `vael.jsonc`
/// import-map/workspace entries, and shared `vael:` built-ins to one canonical
/// identity.
///
/// A canonical path is the module identity, so the same file reached by two
/// spellings is evaluated once. A shared built-in resolves to its canonical
/// `vael:` specifier, so static and dynamic imports also reach one instance.
/// Bare specifiers and every other URL scheme are rejected with a reason. The
/// returned name is always valid UTF-8 because QuickJS module names are strings.
pub(super) struct FileResolver {
    #[cfg(feature = "manifest")]
    import_resolver: Option<ImportResolver>,
    node_compat: bool,
}

impl FileResolver {
    pub(super) fn new() -> Self {
        Self {
            #[cfg(feature = "manifest")]
            import_resolver: None,
            node_compat: true,
        }
    }

    #[cfg(feature = "manifest")]
    pub(super) fn with_import_resolver(resolver: ImportResolver) -> Self {
        Self {
            import_resolver: Some(resolver),
            node_compat: true,
        }
    }

    pub(super) fn with_node_compat(mut self, enabled: bool) -> Self {
        self.node_compat = enabled;
        self
    }

    #[cfg(feature = "manifest")]
    fn resolve_with_import_resolver(&self, base: &str, name: &str) -> Result<Option<String>> {
        let Some(resolver) = &self.import_resolver else {
            return Ok(None);
        };
        let resolved = resolver
            .resolve(name)
            .map_err(|error| Error::new_resolving_message(base, name, error.to_string()))?;
        match resolved {
            ResolvedImport::Builtin(_) | ResolvedImport::Unmapped => Ok(None),
            ResolvedImport::Local(path) => canonicalize_module_path(base, name, &path).map(Some),
            ResolvedImport::Specifier(normalized) => match normalized {
                NormalizedSpecifier::Path(_) => unreachable!("local paths map to Local"),
                NormalizedSpecifier::FileUrl(file_url) => {
                    let parsed = Url::parse(&file_url).map_err(|_| {
                        Error::new_resolving_message(base, name, "invalid file: URL target")
                    })?;
                    let path = parsed.to_file_path().map_err(|()| {
                        Error::new_resolving_message(
                            base,
                            name,
                            "file: URL does not name a local filesystem path",
                        )
                    })?;
                    canonicalize_module_path(base, name, &path).map(Some)
                }
                NormalizedSpecifier::Https(_)
                | NormalizedSpecifier::Gh(_)
                | NormalizedSpecifier::Jsr(_) => Err(Error::new_resolving_message(
                    base,
                    name,
                    "remote module loading is not supported in vaeljs-lite",
                )),
                NormalizedSpecifier::Npm(_) => Err(Error::new_resolving_message(
                    base,
                    name,
                    "npm: modules are not supported; remote module loading is not supported in vaeljs-lite",
                )),
                NormalizedSpecifier::Node(node) => {
                    #[cfg(feature = "node-compat")]
                    if self.node_compat
                        && let Some(canonical) = crate::node_compat::resolve_specifier(&node)
                    {
                        return Ok(Some(canonical.to_owned()));
                    }
                    let _ = &node;
                    Err(Error::new_resolving_message(
                        base,
                        name,
                        node_unsupported_reason(self.node_compat),
                    ))
                }
                NormalizedSpecifier::Bare(_) => Err(Error::new_resolving_message(
                    base,
                    name,
                    BARE_SPECIFIER_REASON,
                )),
            },
        }
    }
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
        // A shared built-in is a canonical `vael:` specifier. It is exact: the
        // shared crate does not accept a bare name, another scheme, a query, or
        // a fragment. Resolving to the same string gives the engine one module
        // identity for static and dynamic imports. With both module features
        // off the dependency is absent and this block is not compiled.
        #[cfg(feature = "node-compat")]
        if self.node_compat
            && let Some(canonical) = crate::node_compat::resolve_specifier(name)
        {
            return Ok(canonical.to_owned());
        }
        if name
            .split_once(':')
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("node"))
        {
            return Err(Error::new_resolving_message(
                base,
                name,
                node_unsupported_reason(self.node_compat),
            ));
        }
        #[cfg(feature = "vael-crypto")]
        if name == crate::crypto::SPECIFIER {
            return Ok(crate::crypto::SPECIFIER.to_owned());
        }
        #[cfg(feature = "vael-fs")]
        if name == vaeljs_fs_core::SPECIFIER {
            return Ok(vaeljs_fs_core::SPECIFIER.to_owned());
        }
        #[cfg(feature = "vael-ssh")]
        if name == vaeljs_ssh_core::SPECIFIER {
            return Ok(vaeljs_ssh_core::SPECIFIER.to_owned());
        }
        #[cfg(feature = "vael-template")]
        if name == crate::template::SPECIFIER {
            return Ok(crate::template::SPECIFIER.to_owned());
        }
        #[cfg(any(
            feature = "vael-assert",
            feature = "vael-chars",
            feature = "vael-console",
            feature = "vael-os",
            feature = "vael-path",
            feature = "vael-slices",
            feature = "vael-strings",
            feature = "vael-test",
            feature = "vael-uuid"
        ))]
        if let Some(builtin) = vaeljs_builtins::module(name) {
            return Ok(builtin.specifier.to_owned());
        }
        #[cfg(feature = "manifest")]
        if let Some(resolved) = self.resolve_with_import_resolver(base, name)? {
            return Ok(resolved);
        }
        let path = candidate_path_with_compat(base, name, self.node_compat)?;
        canonicalize_module_path(base, name, &path)
    }
}

fn canonicalize_module_path(base: &str, name: &str, path: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
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

/// Returns the path a specifier names before canonicalization.
///
/// `base` is the importing module's canonical name. A relative specifier is
/// joined to the base's parent directory, never to the process working
/// directory, so imports stay stable when the caller's current directory
/// changes. A `file:` URL is converted to its local path.
fn node_unsupported_reason(node_compat: bool) -> &'static str {
    if !cfg!(feature = "node-compat") {
        "the `node:` scheme is not implemented in vaeljs-lite; use a relative path, an absolute path, or a file: URL"
    } else if !node_compat {
        "Node.js built-in modules are disabled (--no-node-compat)"
    } else {
        "the requested `node:` module is not implemented in vaeljs-lite"
    }
}

#[cfg(test)]
fn candidate_path(base: &str, name: &str) -> Result<PathBuf> {
    candidate_path_with_compat(base, name, true)
}

fn candidate_path_with_compat(base: &str, name: &str, node_compat: bool) -> Result<PathBuf> {
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
            "node" => Err(Error::new_resolving_message(
                base,
                name,
                node_unsupported_reason(node_compat),
            )),
            scheme => Err(Error::new_resolving_message(
                base,
                name,
                format!(
                    "the `{scheme}:` scheme is not implemented in vaeljs-lite; use a relative path, an absolute path, or a file: URL"
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

/// Reads and declares an ECMAScript or erasable TypeScript module for QuickJS.
///
/// The format decision comes from `vaeljs-module-policy`. When the `typescript`
/// feature is enabled, `.ts` and `.mts` sources are stripped to ECMAScript with
/// preserved line/column positions before `Module::declare`. When `typescript`
/// is disabled, TypeScript modules are rejected with a clear message. Every
/// unsupported extension carries the shared policy's reason. Read errors name
/// the resolved module.
pub(super) struct FileLoader {
    #[cfg_attr(not(feature = "node-compat"), allow(dead_code))]
    pub(super) node_compat: bool,
}

impl Loader for FileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
        // `vael:chars`, `vael:crypto`, `vael:fs`, `vael:os`, `vael:ssh`,
        // `vael:strings`, `vael:uuid`, `vael:uuid/v4`, and `vael:uuid/v7` are native
        // QuickJS modules (`declare_def`) with no ECMAScript source;
        // `vael:assert`, `vael:console`, `vael:path`, `vael:slices`, and `vael:test`
        // declare the shared `.esm.js` re-export wrappers from `vaeljs-builtins`.
        #[cfg(feature = "node-compat")]
        if self.node_compat
            && let Some(canonical) = crate::node_compat::resolve_specifier(name)
        {
            return crate::node_compat::declare_module(ctx, canonical);
        }
        #[cfg(feature = "vael-crypto")]
        if name == crate::crypto::SPECIFIER {
            return Module::declare_def::<crate::crypto::CryptoModule, _>(ctx.clone(), name);
        }
        #[cfg(feature = "vael-fs")]
        if name == vaeljs_fs_core::SPECIFIER {
            return Module::declare_def::<crate::fs::FsModule, _>(ctx.clone(), name);
        }
        #[cfg(feature = "vael-ssh")]
        if name == vaeljs_ssh_core::SPECIFIER {
            return Module::declare_def::<crate::ssh::SshModule, _>(ctx.clone(), name);
        }
        #[cfg(feature = "vael-template")]
        if name == crate::template::SPECIFIER {
            return Module::declare_def::<crate::template::TemplateModule, _>(ctx.clone(), name);
        }
        #[cfg(any(
            feature = "vael-assert",
            feature = "vael-chars",
            feature = "vael-console",
            feature = "vael-os",
            feature = "vael-path",
            feature = "vael-slices",
            feature = "vael-strings",
            feature = "vael-test",
            feature = "vael-uuid"
        ))]
        if let Some(builtin) = vaeljs_builtins::module(name) {
            match builtin.specifier {
                #[cfg(feature = "vael-assert")]
                vaeljs_builtins::assert::SPECIFIER => {
                    return Module::declare(ctx.clone(), name, vaeljs_builtins::assert::ESM_SOURCE);
                }
                #[cfg(feature = "vael-chars")]
                vaeljs_builtins::chars::SPECIFIER => {
                    return Module::declare_def::<crate::chars::CharsModule, _>(ctx.clone(), name);
                }
                #[cfg(feature = "vael-console")]
                vaeljs_builtins::console::SPECIFIER => {
                    return Module::declare(
                        ctx.clone(),
                        name,
                        vaeljs_builtins::console::ESM_SOURCE,
                    );
                }
                #[cfg(feature = "vael-os")]
                vaeljs_builtins::os::SPECIFIER => {
                    return Module::declare_def::<crate::os::OsModule, _>(ctx.clone(), name);
                }
                #[cfg(feature = "vael-path")]
                vaeljs_builtins::path::SPECIFIER => {
                    return Module::declare(ctx.clone(), name, vaeljs_builtins::path::ESM_SOURCE);
                }
                #[cfg(feature = "vael-slices")]
                vaeljs_builtins::slices::SPECIFIER => {
                    return Module::declare(ctx.clone(), name, vaeljs_builtins::slices::ESM_SOURCE);
                }
                #[cfg(feature = "vael-strings")]
                vaeljs_builtins::strings::SPECIFIER => {
                    return Module::declare_def::<crate::strings::StringsModule, _>(
                        ctx.clone(),
                        name,
                    );
                }
                #[cfg(feature = "vael-test")]
                vaeljs_builtins::test::SPECIFIER => {
                    return Module::declare(ctx.clone(), name, vaeljs_builtins::test::ESM_SOURCE);
                }
                #[cfg(feature = "vael-uuid")]
                vaeljs_builtins::uuid::SPECIFIER => {
                    return Module::declare_def::<crate::uuid::UuidModule, _>(ctx.clone(), name);
                }
                #[cfg(feature = "vael-uuid")]
                vaeljs_builtins::uuid::V4_SPECIFIER => {
                    return Module::declare_def::<crate::uuid::UuidV4Module, _>(ctx.clone(), name);
                }
                #[cfg(feature = "vael-uuid")]
                vaeljs_builtins::uuid::V7_SPECIFIER => {
                    return Module::declare_def::<crate::uuid::UuidV7Module, _>(ctx.clone(), name);
                }
                _ => {
                    return Err(Error::new_loading_message(
                        name,
                        "the shared built-in has no native binding in this build",
                    ));
                }
            }
        }
        let path = Path::new(name);
        let format =
            module_format(path).map_err(|reason| Error::new_loading_message(name, reason))?;
        let source = match format {
            ModuleFormat::JavaScript => std::fs::read(path).map_err(|error| {
                Error::new_loading_message(name, format!("cannot read file: {error}"))
            })?,
            ModuleFormat::TypeScript => {
                #[cfg(not(feature = "typescript"))]
                {
                    return Err(Error::new_loading_message(
                        name,
                        "TypeScript is not enabled in this build; enable the `typescript` feature or use a .js or .mjs file",
                    ));
                }
                #[cfg(feature = "typescript")]
                {
                    let specifier = Url::from_file_path(path).map_err(|()| {
                        Error::new_loading_message(
                            name,
                            "the path cannot be represented as a file URL; use a UTF-8 path",
                        )
                    })?;
                    let text = std::fs::read_to_string(path).map_err(|error| {
                        Error::new_loading_message(name, format!("cannot read file: {error}"))
                    })?;
                    vaeljs_module_policy::typescript::strip(&specifier, text)
                        .map_err(|reason| Error::new_loading_message(name, reason))?
                        .into_bytes()
                }
            }
        };
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
        assert!(!is_windows_drive_path("vael:assert"));
        assert!(!is_windows_drive_path("C:"));
    }
}
