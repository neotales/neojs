//! Module resolution and loading for the local filesystem and optional HTTPS.
//!
//! Executable-extension and path-specifier decisions come from the shared
//! `neojs-module-policy` crate, which the separate QuickJS runtime also uses.
//! Engine-specific resolution and reading stay here.
//!
//! Resolution accepts relative paths, absolute paths, `file:` URLs, and the
//! built-in specifiers `age`, `neo:age`, `assert`, `neo:assert`, `chars`,
//! `neo:chars`, `console`, `neo:console`, `crypto`, `neo:crypto`, `os`,
//! `neo:os`, `path`, `neo:path`, `slices`, `neo:slices`, `ssh`, `neo:ssh`,
//! `stdio`, `neo:stdio`, `strings`, `neo:strings`, `test`, `neo:test`, `uuid`,
//! `neo:uuid`, `uuid/v4`, `neo:uuid/v4`, `uuid/v7`, and `neo:uuid/v7`
//! ([`crate::builtins`]), and — only when the runtime was created with
//! [`crate::Runtime::with_remote`] — absolute `https:` URLs. That opt-in also
//! enables `jsr:` imports through [`crate::jsr_loader`]. Every other scheme is
//! rejected with a reason that names the missing feature instead of failing
//! later inside V8.
//!
//! A relative specifier in a remote module resolves against that module's URL,
//! so it stays remote. A remote module cannot import `file:`; remote code gets
//! no implicit path into the local filesystem. When a fetch follows redirects,
//! the module map registers the module under the final URL and aliases the
//! requested URL to it. That gives the module one identity, makes its relative
//! imports resolve against the final URL, and makes `import.meta.url` the final
//! URL, as a browser does. An installed final-URL observer runs before that
//! registration; the JSR loader uses it to keep an ordinary HTTPS redirect out
//! of a verified registry identity.
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
use neojs_module_policy::is_path_specifier;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::builtins;
use crate::remote::RemoteClient;
use crate::remote::diagnostic_url;
use crate::remote::display_url;
use crate::remote::validate_url;
use crate::typescript;

pub(crate) use neojs_module_policy::{ModuleFormat, module_format};

/// Returns why a requested module type is not supported, or `None` when there
/// is no attribute.
///
/// neojs has one module type: ECMAScript. Any `with { type: ... }` attribute is
/// rejected, whether it names a known type (`json`, `text`, `bytes`), an
/// unknown one, or a built-in module. Ignoring the attribute would execute a
/// file as JavaScript when the importer asked for a different representation.
pub(crate) fn unsupported_module_type(requested: &RequestedModuleType) -> Option<String> {
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

/// A callback that sees a remote module's requested and final URLs before
/// registration.
///
/// An error refuses the module. The JSR loader installs one to track ordinary
/// HTTPS identities and to protect verified registry identities.
pub(crate) type FinalUrlObserver = Rc<dyn Fn(&Url, &Url) -> Result<(), String>>;

/// A loader for local and, when enabled, remote ECMAScript and TypeScript
/// modules.
///
/// It uses [`FsModuleLoader`] for local resolution and reads local module files
/// itself, so an I/O failure carries the operating-system error. Specifiers and
/// file types that neojs does not implement are rejected with a message that
/// names the missing feature. A TypeScript module that cannot be reduced to
/// ECMAScript is rejected with the stripper's reason.
///
/// With a remote client, `https:` imports are fetched over HTTPS. A relative
/// specifier in a remote module resolves against the importing module's URL, so
/// it stays remote. A remote module cannot import `file:`; that transition is
/// rejected instead of silently reading the local filesystem.
pub(crate) struct FileModuleLoader {
    remote: Option<RemoteClient>,
    /// A remote entry module prepared before loading starts.
    ///
    /// The runtime fetches and type-strips an entry before consuming itself, so
    /// a preflight failure leaves it unused. The loader serves that prepared
    /// source to the module map, which registers the module under its final
    /// URL and aliases the requested URL to it.
    entry: RefCell<Option<PreparedEntry>>,
    /// Observes the requested and final URLs of a remote module before it is
    /// registered.
    ///
    /// The JSR loader installs this to record an ordinary HTTPS module's
    /// requested and final redirect URLs, and to refuse a redirect onto an
    /// already verified JSR identity before V8 can register a second module
    /// there.
    final_url_observer: RefCell<Option<FinalUrlObserver>>,
}

/// A remote entry module fetched and stripped before V8 sees it.
pub(crate) struct PreparedEntry {
    /// The URL the caller requested, which becomes an alias for `found`.
    pub(crate) requested: Url,
    /// The final URL after redirects.
    pub(crate) found: Url,
    /// ECMAScript source, already type-stripped when needed.
    pub(crate) code: String,
}

impl FileModuleLoader {
    /// Creates a loader. `remote` is `None` when remote loading is disabled.
    pub(crate) fn new(remote: Option<RemoteClient>) -> Self {
        Self {
            remote,
            entry: RefCell::new(None),
            final_url_observer: RefCell::new(None),
        }
    }

    /// Installs a URL observer.
    ///
    /// The observer runs after a remote module is fetched and prepared, and
    /// before the `ModuleSource` that registers it is returned. It receives the
    /// requested URL and the final redirect URL. An error prevents registration.
    pub(crate) fn set_final_url_observer(&self, observer: FinalUrlObserver) {
        *self.final_url_observer.borrow_mut() = Some(observer);
    }

    /// Runs the final-URL observer, when one is installed.
    fn observe_final_url(&self, requested: &Url, final_url: &Url) -> Result<(), String> {
        let observer = self.final_url_observer.borrow();
        match observer.as_ref() {
            Some(observer) => observer(requested, final_url),
            None => Ok(()),
        }
    }

    /// Stores the prepared remote entry for the next load.
    pub(crate) fn set_entry(&self, entry: PreparedEntry) {
        *self.entry.borrow_mut() = Some(entry);
    }

    /// Takes the prepared entry when `specifier` is the requested entry URL.
    fn take_entry(&self, specifier: &Url) -> Option<PreparedEntry> {
        let entry = self.entry.borrow_mut().take();
        match entry {
            Some(entry) if entry.requested == *specifier => Some(entry),
            other => {
                *self.entry.borrow_mut() = other;
                None
            }
        }
    }

    /// Resolves a specifier from a remote referrer.
    fn resolve_from_remote(
        &self,
        specifier: &str,
        referrer: &str,
        base: &Url,
    ) -> ModuleResolveResponse {
        if is_relative_path(specifier) {
            let joined = base.join(specifier).map_err(|error| {
                JsErrorBox::generic(format!(
                    "cannot resolve `{}` from {}: {error}",
                    display_specifier(specifier),
                    display_referrer(referrer)
                ))
            })?;
            return self.resolve_https(joined);
        }

        let specifier_url = Url::parse(specifier).map_err(|error| {
            JsErrorBox::generic(format!(
                "cannot resolve `{}` from {}: {error}",
                display_specifier(specifier),
                display_referrer(referrer)
            ))
        })?;
        match specifier_url.scheme() {
            "https" => self.resolve_https(specifier_url),
            "http" => Err(JsErrorBox::generic(with_referrer(
                "remote module URLs must use https:, not http:".to_string(),
                referrer,
            ))),
            "file" => Err(JsErrorBox::generic(with_referrer(
                "a remote module cannot import a file: URL; remote code may not read local files"
                    .to_string(),
                referrer,
            ))),
            _ => Err(JsErrorBox::generic(with_referrer(
                unsupported_scheme(&specifier_url),
                referrer,
            ))),
        }
    }

    /// Accepts an HTTPS module URL when remote loading is enabled.
    fn resolve_https(&self, url: Url) -> ModuleResolveResponse {
        if self.remote.is_none() {
            return Err(JsErrorBox::generic(
                "remote module loading is disabled; create the Runtime with remote loading enabled"
                    .to_string(),
            ));
        }
        validate_url(&url).map_err(JsErrorBox::generic)?;
        Ok(url)
    }
}

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

        if let Ok(base) = Url::parse(referrer)
            && base.scheme() == "https"
        {
            if self.remote.is_none() {
                return Err(JsErrorBox::generic(
                    "remote module loading is disabled; create the Runtime with remote loading enabled"
                        .to_string(),
                ));
            }
            return self.resolve_from_remote(specifier, referrer, &base);
        }

        if let Ok(url) = Url::parse(specifier) {
            match url.scheme() {
                "https" => return self.resolve_https(url),
                "http" => {
                    return Err(JsErrorBox::generic(with_referrer(
                        "remote module URLs must use https:, not http:".to_string(),
                        referrer,
                    )));
                }
                _ => {}
            }
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
        let display = display_url(module_specifier);
        if let Some(reason) = unsupported_module_type(&options.requested_module_type) {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {display}: {reason}"
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
        if module_specifier.scheme() == "https" {
            if let Some(entry) = self.take_entry(module_specifier) {
                if let Err(reason) = self.observe_final_url(module_specifier, &entry.found) {
                    return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                        "cannot load {display}: {reason}"
                    ))));
                }
                return ModuleLoadResponse::Sync(Ok(ModuleSource::new_with_redirect(
                    ModuleType::JavaScript,
                    ModuleSourceCode::String(entry.code.into()),
                    module_specifier,
                    &entry.found,
                    None,
                )));
            }
            return self.load_remote(module_specifier);
        }
        let Ok(path) = module_specifier.to_file_path() else {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {display}: only file: and https: URLs are supported"
            ))));
        };
        let format = match module_format(&path) {
            Ok(format) => format,
            Err(reason) => {
                return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                    "cannot load {display}: {reason}"
                ))));
            }
        };
        let source = match format {
            ModuleFormat::JavaScript => match std::fs::read(&path) {
                Ok(code) => ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                Err(source) => {
                    return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                        "cannot load {display}: {source}"
                    ))));
                }
            },
            ModuleFormat::TypeScript => {
                let code = match std::fs::read_to_string(&path) {
                    Ok(code) => code,
                    Err(source) => {
                        return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                            "cannot load {display}: {source}"
                        ))));
                    }
                };
                match typescript::strip(module_specifier, code) {
                    Ok(code) => ModuleSourceCode::String(code.into()),
                    Err(reason) => {
                        return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                            "cannot load {display}: {reason}"
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

impl FileModuleLoader {
    /// Loads an HTTPS module with the configured remote client.
    fn load_remote(&self, module_specifier: &Url) -> ModuleLoadResponse {
        let display = display_url(module_specifier);
        let Some(remote) = &self.remote else {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {display}: remote module loading is disabled"
            ))));
        };
        if let Err(reason) = validate_url(module_specifier) {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {display}: {reason}"
            ))));
        }
        let fetched = match remote.fetch(module_specifier) {
            Ok(fetched) => fetched,
            Err(reason) => {
                return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                    "cannot load {display}: {reason}"
                ))));
            }
        };
        // A redirect can change the extension. The format follows the final
        // URL, just as the module identity and relative-import base do.
        let format = match module_format(Path::new(fetched.final_url.path())) {
            Ok(format) => format,
            Err(reason) => {
                return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                    "cannot load {display}: {reason}"
                ))));
            }
        };
        let code = match String::from_utf8(fetched.body) {
            Ok(code) => code,
            Err(_) => {
                return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                    "cannot load {display}: the module is not valid UTF-8"
                ))));
            }
        };
        let source = match format {
            ModuleFormat::JavaScript => ModuleSourceCode::String(code.into()),
            ModuleFormat::TypeScript => {
                match typescript::strip(&diagnostic_url(&fetched.final_url), code) {
                    Ok(code) => ModuleSourceCode::String(code.into()),
                    Err(reason) => {
                        return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                            "cannot load {display}: {reason}"
                        ))));
                    }
                }
            }
        };
        if let Err(reason) = self.observe_final_url(module_specifier, &fetched.final_url) {
            return ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "cannot load {display}: {reason}"
            ))));
        }
        ModuleLoadResponse::Sync(Ok(ModuleSource::new_with_redirect(
            ModuleType::JavaScript,
            source,
            module_specifier,
            &fetched.final_url,
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
            "cannot resolve `{}`: bare module specifiers other than the built-in modules are not implemented; use a relative or absolute file path (built-in modules: {})",
            display_specifier(specifier),
            builtins::implemented_specifiers()
        ),
        referrer,
    ))
}

fn unsupported_scheme(url: &Url) -> String {
    if !url.username().is_empty() || url.password().is_some() {
        return "cannot resolve a URL with a username or password; credentials are not supported"
            .to_string();
    }
    let specifier = display_url(url);
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
        "jsr" => {
            "jsr: modules require remote module loading; create the Runtime with Runtime::with_remote (neojs --allow-remote) to enable them"
                .to_string()
        }
        "http" => "only https: module URLs are supported, not http:".to_string(),
        "https" => {
            "remote module loading is disabled; create the Runtime with Runtime::with_remote"
                .to_string()
        }
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

/// Returns whether a specifier is a URL path that resolves against a remote
/// module's URL.
///
/// Only forward-slash forms are accepted. A backslash is a Windows path
/// separator, not a URL separator, and a remote module has no local filesystem
/// base for it.
fn is_relative_path(specifier: &str) -> bool {
    specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../")
}

/// Formats a specifier for an error without credentials, a query, or a
/// fragment.
///
/// A query can carry a signed token, so only the URL's origin and path (or a
/// bare specifier's name) reach a message.
fn display_specifier(specifier: &str) -> String {
    match Url::parse(specifier) {
        Ok(url) => display_url(&url),
        Err(_) => specifier
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .to_string(),
    }
}

/// Formats a referrer for an error with the same redaction as a specifier.
fn display_referrer(referrer: &str) -> String {
    match Url::parse(referrer) {
        Ok(url) => display_url(&url),
        Err(_) => "<referrer>".to_string(),
    }
}

fn with_referrer(message: String, referrer: &str) -> String {
    if referrer.is_empty() {
        message
    } else {
        format!("{message} (imported from {})", display_referrer(referrer))
    }
}
