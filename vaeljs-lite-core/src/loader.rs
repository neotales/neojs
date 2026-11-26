//! Module resolution and loading for local ECMAScript/TypeScript files,
//! shared `vael:`/`node:` built-ins, optional `vael.jsonc` import maps,
//! optional `https:`/`gh:`/`jsr:` remote modules, and optional `npm:` packages.

use std::path::{Path, PathBuf};
#[cfg(any(feature = "remote", feature = "npm"))]
use std::rc::Rc;

use rquickjs::{
    Ctx, Error, Module, Result,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use url::Url;
#[cfg(feature = "manifest")]
use vaeljs_manifest::{ImportResolver, NormalizedSpecifier, ResolvedImport};
use vaeljs_module_policy::{ModuleFormat, is_path_specifier, module_format};
#[cfg(feature = "remote")]
use vaeljs_remote::{display_url, github_url};

#[cfg(feature = "npm")]
use crate::npm_loader::{NpmLoader, NpmReferrerAction, ProjectTarget};
#[cfg(feature = "remote")]
use crate::remote_loader::RemoteState;

const BARE_SPECIFIER_REASON: &str = "bare module specifiers are not implemented in vaeljs-lite; use a relative path, an absolute path, or a file: URL";
const TYPE_ATTRIBUTE_REASON: &str =
    "import attributes are not implemented; vaeljs-lite loads ECMAScript modules only";

/// Rejects any non-empty import attribute bag.
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
            display_referrer(base),
            display_specifier(name),
            TYPE_ATTRIBUTE_REASON,
        ));
    }
    Ok(())
}

/// Sets `import.meta.url` on a newly declared QuickJS module.
pub(super) fn set_import_meta_url(
    module: &Module<'_, Declared>,
    meta_url: &str,
) -> rquickjs::Result<()> {
    let meta = module.meta()?;
    meta.set("url", meta_url)?;
    Ok(())
}

/// Computes the `import.meta.url` string for a local module identity.
pub(super) fn local_meta_url(name: &str) -> String {
    if (is_windows_drive_path(name) || Path::new(name).is_absolute())
        && let Ok(url) = Url::from_file_path(name)
    {
        return url.to_string();
    }
    name.to_owned()
}

/// Resolves a canonical `vael:` built-in specifier, matching the `vael` scheme
/// ASCII-case-insensitively (`VAEL:os` -> `vael:os`) while rejecting whitespace
/// or control characters in the module name.
pub(super) fn resolve_vael_builtin(name: &str) -> Option<&'static str> {
    if let Some(canonical) = exact_vael_builtin(name) {
        return Some(canonical);
    }
    let (scheme, path) = name.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("vael") {
        return None;
    }
    if path
        .chars()
        .any(|character| character == ' ' || character.is_ascii_control())
    {
        return None;
    }
    let normalized = format!("vael:{path}");
    exact_vael_builtin(&normalized)
}

fn exact_vael_builtin(name: &str) -> Option<&'static str> {
    #[cfg(feature = "vael-crypto")]
    if name == crate::crypto::SPECIFIER {
        return Some(crate::crypto::SPECIFIER);
    }
    #[cfg(feature = "vael-fs")]
    if name == vaeljs_fs_core::SPECIFIER {
        return Some(vaeljs_fs_core::SPECIFIER);
    }
    #[cfg(feature = "vael-ssh")]
    if name == vaeljs_ssh_core::SPECIFIER {
        return Some(vaeljs_ssh_core::SPECIFIER);
    }
    #[cfg(feature = "vael-template")]
    if name == crate::template::SPECIFIER {
        return Some(crate::template::SPECIFIER);
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
        return Some(builtin.specifier);
    }
    let _ = name;
    None
}

/// Resolves relative paths, absolute paths, `file:` URLs, optional `vael.jsonc`
/// import-map/workspace entries, optional `https:`/`gh:`/`jsr:` remote modules,
/// optional `npm:`/`package.json` modules, and shared `vael:`/`node:` built-ins.
pub(super) struct FileResolver {
    #[cfg(feature = "manifest")]
    import_resolver: Option<ImportResolver>,
    #[cfg(feature = "remote")]
    remote: Option<Rc<RemoteState>>,
    #[cfg(feature = "npm")]
    npm: Rc<NpmLoader>,
    node_compat: bool,
}

impl FileResolver {
    pub(super) fn new(
        #[cfg(feature = "manifest")] import_resolver: Option<ImportResolver>,
        #[cfg(feature = "remote")] remote: Option<Rc<RemoteState>>,
        #[cfg(feature = "npm")] npm: Rc<NpmLoader>,
        node_compat: bool,
    ) -> Self {
        Self {
            #[cfg(feature = "manifest")]
            import_resolver,
            #[cfg(feature = "remote")]
            remote,
            #[cfg(feature = "npm")]
            npm,
            node_compat,
        }
    }

    #[cfg(feature = "manifest")]
    fn resolve_with_import_resolver(&self, base: &str, name: &str) -> Result<Option<String>> {
        let Some(resolver) = &self.import_resolver else {
            return Ok(None);
        };
        let resolved = resolver.resolve(name).map_err(|error| {
            Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                error.to_string(),
            )
        })?;
        match resolved {
            ResolvedImport::Builtin(_) | ResolvedImport::Unmapped => Ok(None),
            ResolvedImport::Local(path) => canonicalize_module_path(base, name, &path).map(Some),
            ResolvedImport::Specifier(normalized) => match normalized {
                NormalizedSpecifier::Path(_) => unreachable!("local paths map to Local"),
                NormalizedSpecifier::FileUrl(file_url) => {
                    let parsed = Url::parse(&file_url).map_err(|_| {
                        Error::new_resolving_message(
                            display_referrer(base),
                            display_specifier(name),
                            "invalid file: URL target",
                        )
                    })?;
                    let path = parsed.to_file_path().map_err(|()| {
                        Error::new_resolving_message(
                            display_referrer(base),
                            display_specifier(name),
                            "file: URL does not name a local filesystem path",
                        )
                    })?;
                    canonicalize_module_path(base, name, &path).map(Some)
                }
                NormalizedSpecifier::Https(https_url) => {
                    self.resolve_https_target(base, name, &https_url).map(Some)
                }
                NormalizedSpecifier::Gh(gh) => self
                    .resolve_https_target(base, name, &gh.canonical())
                    .map(Some),
                NormalizedSpecifier::Jsr(jsr) => self
                    .resolve_jsr_target(base, name, &jsr.canonical())
                    .map(Some),
                NormalizedSpecifier::Npm(npm) => self
                    .resolve_npm_target(base, name, &npm.canonical())
                    .map(Some),
                NormalizedSpecifier::Node(node) => {
                    #[cfg(feature = "node-compat")]
                    if self.node_compat
                        && let Some(canonical) = crate::node_compat::resolve_specifier(&node)
                    {
                        return Ok(Some(canonical.to_owned()));
                    }
                    let _ = &node;
                    Err(Error::new_resolving_message(
                        display_referrer(base),
                        display_specifier(name),
                        node_unsupported_reason(self.node_compat),
                    ))
                }
                NormalizedSpecifier::Bare(_) => Err(Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    BARE_SPECIFIER_REASON,
                )),
            },
        }
    }

    fn resolve_https_target(&self, base: &str, name: &str, url_str: &str) -> Result<String> {
        #[cfg(not(feature = "remote"))]
        {
            let _ = url_str;
            Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "remote module loading is not enabled in this build",
            ))
        }
        #[cfg(feature = "remote")]
        {
            let url = Url::parse(url_str).map_err(|error| {
                Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    error.to_string(),
                )
            })?;
            self.resolve_https_url(base, name, &url)
        }
    }

    #[cfg(feature = "remote")]
    fn resolve_https_url(&self, base: &str, name: &str, url: &Url) -> Result<String> {
        let Some(remote) = &self.remote else {
            return Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "remote module loading is disabled; create the Runtime with Runtime::with_remote to enable it",
            ));
        };
        remote.resolve_https(url).map_err(|reason| {
            Error::new_resolving_message(display_referrer(base), display_specifier(name), reason)
        })
    }

    fn resolve_jsr_target(&self, base: &str, name: &str, jsr_spec: &str) -> Result<String> {
        #[cfg(not(feature = "remote"))]
        {
            let _ = jsr_spec;
            Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "remote module loading is not enabled in this build",
            ))
        }
        #[cfg(feature = "remote")]
        {
            let Some(remote) = &self.remote else {
                return Err(Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    "jsr: modules require remote module loading; create the Runtime with Runtime::with_remote (vjs-lite --allow-remote) to enable them",
                ));
            };
            remote.resolve_jsr(jsr_spec).map_err(|reason| {
                Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    format!("cannot resolve `{}`: {reason}", display_specifier(jsr_spec)),
                )
            })
        }
    }

    fn resolve_npm_target(&self, base: &str, name: &str, npm_spec: &str) -> Result<String> {
        #[cfg(not(feature = "npm"))]
        {
            let _ = npm_spec;
            Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "npm: modules are not implemented",
            ))
        }
        #[cfg(feature = "npm")]
        {
            if !self.npm.is_enabled() {
                return Err(Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    "npm module resolution is disabled (--no-npm)",
                ));
            }
            self.npm
                .resolve_npm_specifier(npm_spec, base)
                .map(|url| url.to_string())
                .map_err(|reason| {
                    Error::new_resolving_message(
                        display_referrer(base),
                        display_specifier(name),
                        format!("cannot resolve `{}`: {reason}", display_specifier(npm_spec)),
                    )
                })
        }
    }

    #[cfg(feature = "npm")]
    fn resolve_from_npm_referrer(&self, base: &str, name: &str) -> Result<String> {
        let referrer_url = Url::parse(base).map_err(|error| {
            Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                error.to_string(),
            )
        })?;
        match self.npm.resolve_from_npm_referrer(&referrer_url, name) {
            Ok(NpmReferrerAction::ResolvedUrl(url)) => Ok(url.to_string()),
            Ok(NpmReferrerAction::DelegateVaelBuiltin(spec)) => resolve_vael_builtin(&spec)
                .map(str::to_owned)
                .ok_or_else(|| {
                    Error::new_resolving_message(
                        display_referrer(base),
                        display_specifier(name),
                        format!("cannot resolve `{}`", display_specifier(&spec)),
                    )
                }),
            Ok(NpmReferrerAction::DelegateNodeBuiltin(node_spec)) => {
                #[cfg(feature = "node-compat")]
                if self.node_compat
                    && let Some(canonical) = crate::node_compat::resolve_specifier(&node_spec)
                {
                    return Ok(canonical.to_owned());
                }
                #[cfg(not(feature = "node-compat"))]
                let _ = node_spec;
                Err(Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    node_unsupported_reason(self.node_compat),
                ))
            }
            Err(reason) => Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                reason,
            )),
        }
    }

    #[cfg(feature = "remote")]
    fn resolve_from_remote(&self, base: &str, name: &str, base_url: &Url) -> Result<String> {
        if self.remote.is_none() {
            return Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "remote module loading is disabled; create the Runtime with Runtime::with_remote to enable it",
            ));
        }
        if is_remote_relative_path(name) {
            let joined = base_url.join(name).map_err(|error| {
                Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    error.to_string(),
                )
            })?;
            return self.resolve_https_url(base, name, &joined);
        }
        let Ok(url) = Url::parse(name) else {
            return Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                BARE_SPECIFIER_REASON,
            ));
        };
        match url.scheme() {
            "https" => self.resolve_https_url(base, name, &url),
            "http" => Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "remote module URLs must use https:, not http:",
            )),
            "file" => Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "a remote module cannot import a file: URL; remote code may not read local files",
            )),
            scheme => Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                unsupported_scheme_reason(scheme, &url, self.node_compat),
            )),
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

        // 1. Canonical `vael:*` (case-insensitive scheme) and `node:*` built-ins.
        if let Some(canonical) = resolve_vael_builtin(name) {
            return Ok(canonical.to_owned());
        }
        if is_scheme_ci(name, "vael") {
            return Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                "the requested `vael:` built-in module is not implemented in vaeljs-lite",
            ));
        }
        #[cfg(feature = "node-compat")]
        if self.node_compat
            && let Some(canonical) = crate::node_compat::resolve_specifier(name)
        {
            return Ok(canonical.to_owned());
        }
        if is_scheme_ci(name, "node") {
            return Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                node_unsupported_reason(self.node_compat),
            ));
        }

        // 2. Virtual `npm://` referrers stay strictly inside their CAS graph.
        #[cfg(feature = "npm")]
        if is_npm_referrer(base) {
            return self.resolve_from_npm_referrer(base, name);
        }

        // 3. `jsr:` specifiers and verified JSR graph referrers.
        #[cfg(feature = "remote")]
        if is_scheme_ci(name, "jsr") {
            return self.resolve_jsr_target(base, name, name);
        }
        #[cfg(not(feature = "remote"))]
        if is_scheme_ci(name, "jsr") {
            return self.resolve_jsr_target(base, name, name);
        }

        #[cfg(feature = "remote")]
        let referrer_is_jsr = self
            .remote
            .as_ref()
            .is_some_and(|r| r.is_jsr_loaded_str(base));
        #[cfg(not(feature = "remote"))]
        let referrer_is_jsr = false;

        #[cfg(feature = "remote")]
        if referrer_is_jsr {
            let remote = self
                .remote
                .as_ref()
                .expect("JSR referrer implies remote state");
            if RemoteState::is_jsr_relative_reference(name) {
                return remote.resolve_jsr_relative(name, base).map_err(|reason| {
                    Error::new_resolving_message(
                        display_referrer(base),
                        display_specifier(name),
                        reason,
                    )
                });
            }
            if let Some(reason) = vaeljs_remote::jsr::local_file_reason(name) {
                return Err(Error::new_resolving_message(
                    display_referrer(base),
                    display_specifier(name),
                    reason,
                ));
            }
            if let Some(result) = remote.resolve_jsr_registry_https(name) {
                return result.map_err(|reason| {
                    Error::new_resolving_message(
                        display_referrer(base),
                        display_specifier(name),
                        reason,
                    )
                });
            }
        }

        // 4. Local-only resolution (`vael.jsonc`, explicit `npm:`, `package.json`).
        let local_referrer = !referrer_is_jsr && is_local_file_referrer(base);
        if local_referrer {
            #[cfg(feature = "manifest")]
            if let Some(resolved) = self.resolve_with_import_resolver(base, name)? {
                return Ok(resolved);
            }

            if is_scheme_ci(name, "npm") {
                return self.resolve_npm_target(base, name, name);
            }

            #[cfg(feature = "npm")]
            if self.npm.is_enabled() {
                match self.npm.resolve_project_specifier(name, base) {
                    Ok(Some(ProjectTarget::LocalFile(path))) => {
                        return canonicalize_module_path(base, name, &path);
                    }
                    Ok(Some(ProjectTarget::NpmModule(url))) => return Ok(url.to_string()),
                    Ok(None) => {}
                    Err(reason) => {
                        return Err(Error::new_resolving_message(
                            display_referrer(base),
                            display_specifier(name),
                            reason,
                        ));
                    }
                }
            }
        }

        // 5. `gh:` / `github:` (available from both local and remote/JSR referrers).
        #[cfg(feature = "remote")]
        if let Some(github) = github_url(name) {
            return match github {
                Ok(url) => self.resolve_https_url(base, name, &url),
                Err(reason) => Err(Error::new_resolving_message(
                    display_referrer(base),
                    "<invalid GitHub specifier>",
                    reason,
                )),
            };
        }
        #[cfg(not(feature = "remote"))]
        if is_scheme_ci(name, "gh") || is_scheme_ci(name, "github") {
            return Err(Error::new_resolving_message(
                display_referrer(base),
                "<GitHub specifier>",
                "remote module loading is not enabled in this build",
            ));
        }

        // 6. Remote `https:` / JSR referrers never consult local filesystem paths.
        #[cfg(feature = "remote")]
        if let Ok(base_url) = Url::parse(base)
            && !is_windows_drive_path(base)
            && base_url.scheme() == "https"
        {
            return self.resolve_from_remote(base, name, &base_url);
        }

        // 7. Explicit `https:` / `http:` from local referrers.
        if !is_windows_drive_path(name)
            && let Ok(url) = Url::parse(name)
        {
            match url.scheme() {
                "https" => return self.resolve_https_target(base, name, name),
                "http" => {
                    return Err(Error::new_resolving_message(
                        display_referrer(base),
                        display_specifier(name),
                        "remote module URLs must use https:, not http:",
                    ));
                }
                _ => {}
            }
        }

        // 8. Local file / `file:` URL resolution.
        let path = candidate_path_with_compat(base, name, self.node_compat)?;
        canonicalize_module_path(base, name, &path)
    }
}

fn canonicalize_module_path(base: &str, name: &str, path: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        Error::new_resolving_message(
            display_referrer(base),
            display_specifier(name),
            format!("cannot resolve `{}`: {error}", path.display()),
        )
    })?;
    canonical.to_str().map(str::to_owned).ok_or_else(|| {
        Error::new_resolving_message(
            display_referrer(base),
            display_specifier(name),
            format!("resolved path `{}` is not valid UTF-8", canonical.display()),
        )
    })
}

fn node_unsupported_reason(node_compat: bool) -> &'static str {
    if !cfg!(feature = "node-compat") {
        "the `node:` scheme is not implemented in vaeljs-lite; use a relative path, an absolute path, or a file: URL"
    } else if !node_compat {
        "Node.js built-in modules are disabled (--no-node-compat)"
    } else {
        "the requested `node:` module is not implemented in vaeljs-lite"
    }
}

fn unsupported_scheme_reason(scheme: &str, url: &Url, node_compat: bool) -> String {
    if !url.username().is_empty() || url.password().is_some() {
        return "cannot resolve a URL with a username or password; credentials are not supported"
            .to_string();
    }
    match scheme {
        "node" => node_unsupported_reason(node_compat).to_string(),
        "npm" => {
            if cfg!(feature = "npm") {
                "npm: modules are only allowed from local modules when npm resolution is enabled"
                    .to_string()
            } else {
                "npm: modules are not implemented".to_string()
            }
        }
        "jsr" => "jsr: modules require remote module loading; create the Runtime with Runtime::with_remote (vjs-lite --allow-remote) to enable them".to_string(),
        "http" => "remote module URLs must use https:, not http:".to_string(),
        "https" => "remote module loading is disabled; create the Runtime with Runtime::with_remote to enable it".to_string(),
        _ => format!(
            "the `{scheme}:` scheme is not implemented in vaeljs-lite; use a relative path, an absolute path, or a file: URL"
        ),
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
                    display_referrer(base),
                    display_specifier(name),
                    "file: URL does not name a local filesystem path",
                )
            }),
            "node" => Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                node_unsupported_reason(node_compat),
            )),
            scheme => Err(Error::new_resolving_message(
                display_referrer(base),
                display_specifier(name),
                unsupported_scheme_reason(scheme, &url, node_compat),
            )),
        };
    }

    if !is_path_specifier(name) {
        return Err(Error::new_resolving_message(
            display_referrer(base),
            display_specifier(name),
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
pub(crate) fn is_windows_drive_path(specifier: &str) -> bool {
    let bytes = specifier.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn is_scheme_ci(specifier: &str, target: &str) -> bool {
    specifier
        .split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(target))
}

#[cfg(feature = "remote")]
fn is_remote_relative_path(specifier: &str) -> bool {
    specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../")
}

#[cfg(feature = "npm")]
fn is_npm_referrer(referrer: &str) -> bool {
    !is_windows_drive_path(referrer) && Url::parse(referrer).is_ok_and(|url| url.scheme() == "npm")
}

fn is_local_file_referrer(referrer: &str) -> bool {
    if referrer.is_empty() {
        return false;
    }
    if is_windows_drive_path(referrer) {
        return true;
    }
    if let Ok(url) = Url::parse(referrer) {
        return url.scheme() == "file";
    }
    true
}

/// Formats a specifier for an error without credentials, query, or fragment.
pub(crate) fn display_specifier(specifier: &str) -> String {
    if is_windows_drive_path(specifier) {
        return specifier.to_owned();
    }
    match Url::parse(specifier) {
        Ok(url)
            if matches!(url.scheme(), "https" | "http")
                || !url.username().is_empty()
                || url.password().is_some() =>
        {
            redact_parsed_url(&url)
        }
        Ok(_) | Err(_) => specifier
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .to_owned(),
    }
}

fn display_referrer(referrer: &str) -> String {
    if is_windows_drive_path(referrer) {
        return referrer.to_owned();
    }
    match Url::parse(referrer) {
        Ok(url)
            if matches!(url.scheme(), "https" | "http")
                || !url.username().is_empty()
                || url.password().is_some() =>
        {
            redact_parsed_url(&url)
        }
        _ => referrer.to_owned(),
    }
}

fn redact_parsed_url(url: &Url) -> String {
    #[cfg(feature = "remote")]
    if matches!(url.scheme(), "https" | "http") {
        return display_url(url);
    }
    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    sanitized.set_query(None);
    sanitized.set_fragment(None);
    sanitized.to_string()
}

/// Reads and declares an ECMAScript or erasable TypeScript module for QuickJS.
pub(super) struct FileLoader {
    #[cfg(feature = "remote")]
    remote: Option<Rc<RemoteState>>,
    #[cfg(feature = "npm")]
    npm: Rc<NpmLoader>,
    #[cfg_attr(not(feature = "node-compat"), allow(dead_code))]
    pub(super) node_compat: bool,
}

impl FileLoader {
    pub(super) fn new(
        #[cfg(feature = "remote")] remote: Option<Rc<RemoteState>>,
        #[cfg(feature = "npm")] npm: Rc<NpmLoader>,
        node_compat: bool,
    ) -> Self {
        Self {
            #[cfg(feature = "remote")]
            remote,
            #[cfg(feature = "npm")]
            npm,
            node_compat,
        }
    }
}

impl Loader for FileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
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
                        display_specifier(name),
                        "the shared built-in has no native binding in this build",
                    ));
                }
            }
        }

        // Virtual `npm://` modules from verified CAS storage.
        #[cfg(feature = "npm")]
        if is_npm_referrer(name) {
            let url = Url::parse(name).map_err(|error| {
                Error::new_loading_message(display_specifier(name), error.to_string())
            })?;
            let loaded = self
                .npm
                .load_module(&url)
                .map_err(|reason| Error::new_loading_message(display_specifier(name), reason))?;
            let source = match loaded.format {
                ModuleFormat::JavaScript => loaded.bytes,
                ModuleFormat::TypeScript => {
                    #[cfg(not(feature = "typescript"))]
                    {
                        return Err(Error::new_loading_message(
                            display_specifier(name),
                            "TypeScript is not enabled in this build; enable the `typescript` feature or use a .js or .mjs file",
                        ));
                    }
                    #[cfg(feature = "typescript")]
                    {
                        let code = String::from_utf8(loaded.bytes).map_err(|_| {
                            Error::new_loading_message(
                                display_specifier(name),
                                "the TypeScript file is not valid UTF-8",
                            )
                        })?;
                        vaeljs_module_policy::typescript::strip(&url, code)
                            .map_err(|reason| {
                                Error::new_loading_message(display_specifier(name), reason)
                            })?
                            .into_bytes()
                    }
                }
            };
            let module = Module::declare(ctx.clone(), name, source)?;
            set_import_meta_url(&module, name)?;
            return Ok(module);
        }

        // Verified JSR modules and ordinary `https:` remote modules.
        #[cfg(feature = "remote")]
        if !is_windows_drive_path(name)
            && let Ok(url) = Url::parse(name)
            && url.scheme() == "https"
        {
            let Some(remote) = &self.remote else {
                return Err(Error::new_loading_message(
                    display_specifier(name),
                    "remote module loading is disabled",
                ));
            };
            let source = if remote.is_jsr_candidate_str(name) {
                remote
                    .load_jsr(name)
                    .map_err(|reason| Error::new_loading_message(display_specifier(name), reason))?
            } else if let Some(prefetched) = remote.take_prefetched_https(name) {
                prefetched
            } else {
                remote
                    .load_https_direct(&url)
                    .map_err(|reason| Error::new_loading_message(display_specifier(name), reason))?
            };
            let module = Module::declare(ctx.clone(), name, source)?;
            set_import_meta_url(&module, name)?;
            return Ok(module);
        }

        // Local filesystem modules.
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
        let module = Module::declare(ctx.clone(), name, source)?;
        set_import_meta_url(&module, &local_meta_url(name))?;
        Ok(module)
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
