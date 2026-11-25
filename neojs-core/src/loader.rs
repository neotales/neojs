//! Module resolution and loading for the local filesystem and optional HTTPS.
//!
//! Executable-extension and path-specifier decisions come from the shared
//! `neojs-module-policy` crate, which the separate QuickJS runtime also uses.
//! Engine-specific resolution and reading stay here.
//!
//! Resolution accepts relative paths, absolute paths, `file:` URLs, the
//! canonical built-in specifiers `neo:age`, `neo:assert`, `neo:chars`,
//! `neo:console`, `neo:cron`, `neo:crypto`, `neo:fs`, `neo:os`, `neo:path`,
//! `neo:slices`, `neo:ssh`, `neo:stdio`, `neo:strings`, `neo:test`, `neo:uuid`,
//! `neo:uuid/v4`, and `neo:uuid/v7` ([`crate::builtins`]), and — only when the
//! runtime was created with [`crate::Runtime::with_remote`] — absolute `https:`
//! URLs. That opt-in also enables `jsr:` imports through
//! [`crate::jsr_loader`] and the `gh:` and `github:` GitHub specifiers. A
//! GitHub specifier is parsed and normalized to a credential-free
//! `https://raw.githubusercontent.com/owner/repo/ref/path` URL before it is
//! resolved, so it takes the same remote path as an explicit HTTPS import; see
//! [`neojs_remote::github`] for the accepted grammar. Every other scheme is rejected
//! with a reason that names the missing feature instead of failing later inside
//! V8. A bare specifier is never a built-in module, even when it matches a
//! registry name; `age` is rejected where `neo:age` resolves.
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
//! Each built-in module has exactly one canonical `neo:` specifier. The URL
//! scheme is matched case-insensitively, as URL schemes are, so `NEO:age` is
//! the same module as `neo:age`. The path is matched against the original
//! specifier, not a URL-normalized one: embedded tab/newline characters and
//! leading or trailing whitespace or control characters are rejected rather
//! than trimmed. The engine caches one instance per canonical URL. Built-in
//! modules are never read from disk.
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
use crate::typescript;
use neojs_manifest::{ImportResolver, NormalizedSpecifier, ResolvedImport};
use neojs_remote::RemoteClient;
use neojs_remote::diagnostic_url;
use neojs_remote::display_url;
use neojs_remote::github_url;
use neojs_remote::validate_url;

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
    import_resolver: Option<ImportResolver>,
    node_compat: bool,
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
    pub(crate) fn new(
        remote: Option<RemoteClient>,
        import_resolver: Option<ImportResolver>,
    ) -> Self {
        Self {
            remote,
            import_resolver,
            node_compat: true,
            entry: RefCell::new(None),
            final_url_observer: RefCell::new(None),
        }
    }

    /// Configures whether canonical `node:*` compatibility modules resolve.
    pub(crate) fn with_node_compat(mut self, enabled: bool) -> Self {
        self.node_compat = enabled;
        self
    }

    /// Resolves a `jsr:` specifier mapped by the project's `ImportResolver`
    /// when `referrer` is a local `file:` module.
    ///
    /// Returns `Ok(None)` when there is no resolver, the referrer is not a
    /// `file:` URL, or the mapping did not produce a `jsr:` target.
    pub(crate) fn resolve_jsr_from_manifest(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Result<Option<String>, JsErrorBox> {
        let Some(resolver) = &self.import_resolver else {
            return Ok(None);
        };
        if !is_file_referrer(referrer) {
            return Ok(None);
        }
        if builtins::resolve(specifier).is_some() || case_insensitive_builtin(specifier).is_some() {
            return Ok(None);
        }
        if specifier
            .split_once(':')
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("node"))
        {
            return Ok(None);
        }
        match resolver.resolve(specifier) {
            Ok(ResolvedImport::Specifier(NormalizedSpecifier::Jsr(jsr))) => {
                Ok(Some(jsr.canonical()))
            }
            Ok(_) => Ok(None),
            Err(error) => Err(JsErrorBox::generic(with_referrer(
                format!("cannot resolve `{}`: {error}", display_specifier(specifier)),
                referrer,
            ))),
        }
    }

    /// Consults `import_resolver` for a local `file:` referrer.
    ///
    /// Remote (`https:`) and non-`file:` referrers never consult the project's
    /// `ImportResolver`, so remote code cannot escape into local `file:` paths
    /// through `neo.jsonc` mappings or workspace packages.
    fn resolve_with_import_resolver(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Option<ModuleResolveResponse> {
        let resolver = self.import_resolver.as_ref()?;
        if !is_file_referrer(referrer) {
            return None;
        }
        let resolved = match resolver.resolve(specifier) {
            Ok(resolved) => resolved,
            Err(error) => {
                return Some(Err(JsErrorBox::generic(with_referrer(
                    format!("cannot resolve `{}`: {error}", display_specifier(specifier)),
                    referrer,
                ))));
            }
        };
        match resolved {
            ResolvedImport::Builtin(_) | ResolvedImport::Unmapped => None,
            ResolvedImport::Local(path) => Some(Url::from_file_path(&path).map_err(|()| {
                JsErrorBox::generic(with_referrer(
                    format!(
                        "cannot resolve `{}`: resolved local path is not a valid file: URL",
                        display_specifier(specifier)
                    ),
                    referrer,
                ))
            })),
            ResolvedImport::Specifier(normalized) => match normalized {
                NormalizedSpecifier::Path(_) => unreachable!("local paths map to Local"),
                NormalizedSpecifier::FileUrl(file_url) => {
                    let parsed = match Url::parse(&file_url) {
                        Ok(url) if url.scheme() == "file" && url.to_file_path().is_ok() => Ok(url),
                        _ => Err(JsErrorBox::generic(with_referrer(
                            format!(
                                "cannot resolve `{}`: invalid file: URL target",
                                display_specifier(specifier)
                            ),
                            referrer,
                        ))),
                    };
                    Some(parsed)
                }
                NormalizedSpecifier::Https(https_url) => match Url::parse(&https_url) {
                    Ok(url) => Some(self.resolve_https(url)),
                    Err(error) => Some(Err(JsErrorBox::generic(with_referrer(
                        format!("cannot resolve `{}`: {error}", display_specifier(specifier)),
                        referrer,
                    )))),
                },
                NormalizedSpecifier::Gh(gh) => match Url::parse(&gh.canonical()) {
                    Ok(url) => Some(self.resolve_https(url)),
                    Err(error) => Some(Err(JsErrorBox::generic(with_referrer(
                        format!("cannot resolve `{}`: {error}", display_specifier(specifier)),
                        referrer,
                    )))),
                },
                NormalizedSpecifier::Jsr(jsr) => {
                    let reason = self
                        .unsupported_specifier(&jsr.canonical(), referrer)
                        .unwrap_or_else(|| {
                            with_referrer(
                                format!(
                                    "cannot resolve `{}`: jsr: modules require remote module loading",
                                    jsr.canonical()
                                ),
                                referrer,
                            )
                        });
                    Some(Err(JsErrorBox::generic(reason)))
                }
                NormalizedSpecifier::Npm(npm) => {
                    let reason = self
                        .unsupported_specifier(&npm.canonical(), referrer)
                        .unwrap_or_else(|| {
                            with_referrer(
                                format!(
                                    "cannot resolve `{}`: npm: modules are not implemented",
                                    npm.canonical()
                                ),
                                referrer,
                            )
                        });
                    Some(Err(JsErrorBox::generic(reason)))
                }
                NormalizedSpecifier::Node(node) => {
                    #[cfg(feature = "node-compat")]
                    if self.node_compat
                        && let Some(canonical) = crate::node_compat::resolve_specifier(&node)
                    {
                        return Some(builtin_url(canonical));
                    }
                    let reason = self
                        .unsupported_specifier(&node, referrer)
                        .unwrap_or_else(|| with_referrer(self.node_unsupported_reason(), referrer));
                    Some(Err(JsErrorBox::generic(reason)))
                }
                NormalizedSpecifier::Bare(bare) => {
                    let reason = self
                        .unsupported_specifier(&bare, referrer)
                        .unwrap_or_else(|| {
                            with_referrer(
                                format!(
                                    "cannot resolve `{}`: bare import-map targets are not implemented",
                                    display_specifier(&bare)
                                ),
                                referrer,
                            )
                        });
                    Some(Err(JsErrorBox::generic(reason)))
                }
            },
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

        let specifier_url = match Url::parse(specifier) {
            Ok(url) => url,
            Err(error) => {
                // A bare name has no URL base to resolve against. Reject it
                // with the same explanation as a bare name from a local
                // importer instead of a parser message about a missing base.
                if let Some(reason) = self.unsupported_specifier(specifier, referrer) {
                    return Err(JsErrorBox::generic(reason));
                }
                return Err(JsErrorBox::generic(format!(
                    "cannot resolve `{}` from {}: {error}",
                    display_specifier(specifier),
                    display_referrer(referrer)
                )));
            }
        };
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
                self.unsupported_scheme(&specifier_url),
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
        validate_url(&url).map_err(|error| JsErrorBox::generic(error.to_string()))?;
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
        // canonical module as `neo:chars`. The path is read from the original
        // string: `Url::parse` trims whitespace and removes embedded
        // tab/newline characters, which would make `neo:ch\nars` resolve.
        if let Some(canonical) = case_insensitive_builtin(specifier) {
            return builtin_url(canonical);
        }
        #[cfg(feature = "node-compat")]
        if self.node_compat
            && let Some(canonical) = crate::node_compat::resolve_specifier(specifier)
        {
            return builtin_url(canonical);
        }
        if specifier
            .split_once(':')
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("node"))
        {
            return Err(JsErrorBox::generic(with_referrer(
                format!(
                    "cannot resolve `{}`: {}",
                    display_specifier(specifier),
                    self.node_unsupported_reason()
                ),
                referrer,
            )));
        }

        if let Some(resolved) = self.resolve_with_import_resolver(specifier, referrer) {
            return resolved;
        }

        // `gh:` and `github:` are not a transport of their own: they name one
        // credential-free raw HTTPS URL. Resolving them here, before the
        // referrer branch, makes them work from a local or a remote importer.
        if let Some(github) = github_url(specifier) {
            return match github {
                Ok(url) => self.resolve_https(url),
                // The parser's reason is fixed text. The specifier itself is
                // never echoed: a `gh:` URL is opaque to `url`, so its path can
                // hold `user:secret@host` or a query-like token that no URL
                // redaction would remove.
                Err(reason) => Err(JsErrorBox::generic(with_referrer(reason, referrer))),
            };
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
        if let Some(reason) = self.unsupported_specifier(specifier, referrer) {
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

/// Resolves a built-in specifier whose scheme differs only in ASCII case.
///
/// Only the `neo` scheme is case-insensitive; the path is matched byte for
/// byte against the registry. The specifier is split directly instead of passed
/// through [`Url::parse`], because the URL parser trims leading and trailing C0
/// controls and spaces and removes embedded tab, line feed, and carriage return
/// characters. Accepting those transformations would make a malformed spelling
/// such as `neo:ch\nars` reach `neo:chars`.
///
/// The path must contain no ASCII space or control character at all. Registry
/// names never do, so this rejects the trimmed forms even if one were added
/// later. Query and fragment text stays part of the path and cannot match.
fn case_insensitive_builtin(specifier: &str) -> Option<&'static str> {
    let (scheme, path) = specifier.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("neo") {
        return None;
    }
    if path
        .chars()
        .any(|character| character == ' ' || character.is_ascii_control())
    {
        return None;
    }
    builtins::resolve_name(path)
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

impl FileModuleLoader {
    fn node_unsupported_reason(&self) -> String {
        if !cfg!(feature = "node-compat") {
            "Node.js built-in modules are not implemented".to_string()
        } else if !self.node_compat {
            "Node.js built-in modules are disabled (--no-node-compat)".to_string()
        } else {
            "Node.js built-in modules are not implemented for this specifier".to_string()
        }
    }

    fn unsupported_specifier(&self, specifier: &str, referrer: &str) -> Option<String> {
        if let Ok(url) = Url::parse(specifier) {
            return match url.scheme() {
                "file" => None,
                _ => Some(with_referrer(self.unsupported_scheme(&url), referrer)),
            };
        }

        if is_path_specifier(specifier) {
            return None;
        }

        if builtins::is_registry_name(specifier) {
            return Some(with_referrer(
                format!(
                    "cannot resolve `{}`: Neo built-in modules must be imported by their canonical `neo:` specifier; use `neo:{}`. Bare module specifiers are not implemented (built-in modules: {})",
                    display_specifier(specifier),
                    specifier,
                    builtins::implemented_specifiers()
                ),
                referrer,
            ));
        }

        Some(with_referrer(
            format!(
                "cannot resolve `{}`: bare module specifiers are not implemented; use a relative or absolute file path (built-in modules: {})",
                display_specifier(specifier),
                builtins::implemented_specifiers()
            ),
            referrer,
        ))
    }

    fn unsupported_scheme(&self, url: &Url) -> String {
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
            "node" => self.node_unsupported_reason(),
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
}

/// Returns whether `referrer` is a local `file:` URL.
fn is_file_referrer(referrer: &str) -> bool {
    Url::parse(referrer).is_ok_and(|url| url.scheme() == "file")
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
