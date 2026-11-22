//! Module resolution and loading restricted to the local filesystem.
//!
//! Resolution accepts relative paths, absolute paths, `file:` URLs, and the
//! built-in specifiers `age`, `neo:age`, `chars`, `neo:chars`, `console`,
//! `neo:console`, `crypto`, `neo:crypto`, `path`, `neo:path`, `ssh`, `neo:ssh`,
//! `stdio`, `neo:stdio`, `strings`, and `neo:strings`
//! ([`crate::builtins`]). Every other scheme is rejected with a reason that
//! names the missing feature instead of failing later inside V8.
//!
//! Built-in specifiers resolve to one canonical `neo:` URL, so the engine caches
//! a single module instance however many spellings or importers use. They are
//! never read from disk.
//!
//! A `.ts` or `.mts` module is read as text and type-stripped before V8 sees it
//! ([`crate::typescript`]). A file that cannot be reduced to ECMAScript fails
//! with the stripper's reason, naming the module.
//!
//! Import attributes that request a module type are rejected: neojs loads
//! ECMAScript modules only, so a `with { type: ... }` clause (including on a
//! built-in or on a dynamic import) fails with a reason instead of being
//! ignored. Attributes that do not name a type are not interpreted by neojs and
//! are left to the engine.

use std::path::Path;

use deno_core::FsModuleLoader;
use deno_core::ModuleLoadOptions;
use deno_core::ModuleLoadReferrer;
use deno_core::ModuleLoadResponse;
use deno_core::ModuleLoader;
use deno_core::ModuleResolveResponse;
use deno_core::ModuleSource;
use deno_core::ModuleSourceCode;
use deno_core::ModuleType;
use deno_core::RequestedModuleType;
use deno_core::ResolutionKind;
use deno_core::url::Url;
use deno_error::JsErrorBox;

use crate::builtins;
use crate::typescript;

const JSX_REASON: &str =
    "JSX and TSX are not implemented; use .ts or .mts, or transpile the JSX first";
const CTS_REASON: &str = "CommonJS TypeScript (.cts) is not supported; use .ts or .mts instead";
const DECLARATION_REASON: &str =
    "declaration files (.d.ts, .d.mts, and .d.cts) describe types and contain no runnable code";
const COMMONJS_REASON: &str = "the .cjs extension is not supported; neojs runs ECMAScript modules";
const JSON_REASON: &str = "JSON modules are not implemented; only ECMAScript modules are loaded";
const WASM_REASON: &str =
    "WebAssembly modules are not implemented; only ECMAScript modules are loaded";
const NATIVE_REASON: &str = "native addons are not implemented";

/// How neojs executes a module file, decided by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleFormat {
    /// ECMAScript, passed to V8 unchanged.
    JavaScript,
    /// TypeScript, reduced to ECMAScript by type stripping.
    TypeScript,
}

/// Returns the format of a file, or why it is not an executable module.
///
/// `.js` and `.mjs` are JavaScript. `.ts` and `.mts` are TypeScript and are
/// type-stripped before evaluation. An absent extension is accepted and treated
/// as JavaScript. Declaration files, JSX and TSX, CommonJS, JSON, WebAssembly,
/// native addons, and unknown extensions are rejected with a reason. The check
/// is by name only; the file is not opened.
pub(crate) fn module_format(path: &Path) -> Result<ModuleFormat, String> {
    if typescript::is_declaration_file(path) {
        return Err(DECLARATION_REASON.to_string());
    }
    let Some(extension) = path.extension() else {
        return Ok(ModuleFormat::JavaScript);
    };
    let extension = match extension.to_str() {
        Some(extension) => extension.to_ascii_lowercase(),
        None => return Err("the file extension is not valid UTF-8".to_string()),
    };

    let reason = match extension.as_str() {
        "js" | "mjs" => return Ok(ModuleFormat::JavaScript),
        "ts" | "mts" => return Ok(ModuleFormat::TypeScript),
        "tsx" | "jsx" => JSX_REASON,
        "cts" => CTS_REASON,
        "cjs" => COMMONJS_REASON,
        "json" => JSON_REASON,
        "wasm" => WASM_REASON,
        "node" => NATIVE_REASON,
        other => {
            return Err(format!(
                "unknown module extension `.{other}`; neojs loads ECMAScript modules (.js or .mjs) and erasable TypeScript (.ts or .mts)"
            ));
        }
    };
    Err(reason.to_string())
}

/// Returns why a requested module type is not supported, or `None` when there
/// is no attribute.
///
/// neojs has one module type: ECMAScript. Any `with { type: ... }` attribute is
/// rejected, whether it names a known type (`json`, `text`, `bytes`), an
/// unknown one, or a built-in module. Ignoring the attribute would execute a
/// file as JavaScript when the importer asked for a different representation.
fn unsupported_module_type(requested: &RequestedModuleType) -> Option<String> {
    let kind = match requested {
        RequestedModuleType::None => return None,
        RequestedModuleType::Json => "json".to_string(),
        RequestedModuleType::Text => "text".to_string(),
        RequestedModuleType::Bytes => "bytes".to_string(),
        RequestedModuleType::Other(kind) => kind.to_string(),
    };
    Some(format!(
        "module type attributes are not implemented; neojs cannot load a module with `type: \"{kind}\"`"
    ))
}

/// A loader for local ECMAScript and TypeScript modules.
///
/// It uses [`FsModuleLoader`] for resolution and reads module files itself, so an
/// I/O failure carries the operating-system error. Specifiers and file types that
/// neojs does not implement are rejected with a message that names the missing
/// feature. A TypeScript module that cannot be reduced to ECMAScript is rejected
/// with the stripper's reason.
pub(crate) struct FileModuleLoader;

impl ModuleLoader for FileModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        if let Some(canonical) = builtins::resolve(specifier) {
            return builtin_url(canonical);
        }
        // A URL scheme is case-insensitive, so `NEO:chars` reaches the same
        // built-in as `neo:chars`.
        if let Ok(url) = Url::parse(specifier)
            && url.scheme() == "neo"
            && url.query().is_none()
            && url.fragment().is_none()
            && let Some(canonical) = builtins::resolve_name(url.path())
        {
            return builtin_url(canonical);
        }
        if let Some(reason) = unsupported_specifier(specifier, referrer) {
            return Err(JsErrorBox::generic(reason));
        }
        FsModuleLoader.resolve(specifier, referrer, kind)
    }

    fn load(
        &self,
        module_specifier: &deno_core::ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        if let Some(reason) = unsupported_module_type(&options.requested_module_type) {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {module_specifier}: {reason}"
            ))));
        }
        if let Some(source) = builtins::source(module_specifier) {
            return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(source.to_string().into()),
                module_specifier,
                None,
            )));
        }
        let Ok(path) = module_specifier.to_file_path() else {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {module_specifier}: only file: URLs are supported"
            ))));
        };
        let format = match module_format(&path) {
            Ok(format) => format,
            Err(reason) => {
                return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                    "cannot load {module_specifier}: {reason}"
                ))));
            }
        };
        let source = match format {
            ModuleFormat::JavaScript => match std::fs::read(&path) {
                Ok(code) => ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                Err(source) => {
                    return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                        "cannot load {module_specifier}: {source}"
                    ))));
                }
            },
            ModuleFormat::TypeScript => {
                let code = match std::fs::read_to_string(&path) {
                    Ok(code) => code,
                    Err(source) => {
                        return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                            "cannot load {module_specifier}: {source}"
                        ))));
                    }
                };
                match typescript::strip(module_specifier, code) {
                    Ok(code) => ModuleSourceCode::String(code.into()),
                    Err(reason) => {
                        return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                            "cannot load {module_specifier}: {reason}"
                        ))));
                    }
                }
            }
        };
        ModuleLoadResponse::Sync(Ok(ModuleSource::new(
            ModuleType::JavaScript,
            source,
            module_specifier,
            None,
        )))
    }
}

/// Resolves a built-in specifier to its canonical URL.
///
/// The canonical specifiers are compile-time constants, so a parse failure means
/// the constant is malformed rather than that the importer wrote something bad.
fn builtin_url(canonical: &'static str) -> ModuleResolveResponse {
    Url::parse(canonical).map_err(|error| {
        JsErrorBox::generic(format!(
            "cannot resolve built-in module `{canonical}`: {error}"
        ))
    })
}

fn unsupported_specifier(specifier: &str, referrer: &str) -> Option<String> {
    if let Ok(url) = Url::parse(specifier) {
        return match url.scheme() {
            "file" => None,
            _ => Some(with_referrer(unsupported_scheme(&url), referrer)),
        };
    }

    if is_path_specifier(specifier) {
        return None;
    }

    Some(with_referrer(
        format!(
            "cannot resolve `{specifier}`: bare module specifiers other than the built-in modules are not implemented; use a relative or absolute file path (built-in modules: {})",
            builtins::implemented_specifiers()
        ),
        referrer,
    ))
}

fn unsupported_scheme(url: &Url) -> String {
    let specifier = url.as_str();
    let reason = match url.scheme() {
        "neo" => {
            let name = url.path();
            if name.is_empty() {
                format!(
                    "a neo: specifier must name a module; neojs implements {}",
                    builtins::implemented_specifiers()
                )
            } else if builtins::resolve_name(name).is_some() {
                format!("the specifier must be exactly `neo:{name}`, with no query or fragment")
            } else {
                format!(
                    "neo module `{name}` is not implemented; neojs implements {}",
                    builtins::implemented_specifiers()
                )
            }
        }
        "node" => "Node.js built-in modules are not implemented".to_string(),
        "npm" => "npm: modules are not implemented".to_string(),
        "jsr" => "jsr: modules are not implemented".to_string(),
        "http" | "https" => "remote modules are not implemented".to_string(),
        "data" | "blob" => "data: and blob: modules are not implemented".to_string(),
        scheme if scheme.len() == 1 && scheme.as_bytes()[0].is_ascii_alphabetic() => {
            return format!(
                "cannot resolve `{specifier}`: Windows drive paths must be written as file: URLs"
            );
        }
        scheme => {
            return format!(
                "cannot resolve `{specifier}`: unsupported URL scheme `{scheme}`; only file: URLs are supported"
            );
        }
    };
    format!("cannot resolve `{specifier}`: {reason}")
}

fn is_path_specifier(specifier: &str) -> bool {
    specifier.starts_with('/')
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with(".\\")
        || specifier.starts_with("..\\")
        || specifier.starts_with("\\\\")
}

fn with_referrer(message: String, referrer: &str) -> String {
    if referrer.is_empty() {
        message
    } else {
        format!("{message} (imported from {referrer})")
    }
}
