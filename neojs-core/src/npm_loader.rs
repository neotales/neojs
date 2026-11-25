//! `npm:` and `package.json` module resolution and verified CAS loading.
//!
//! Compiled only when the `npm` Cargo feature is enabled.
//!
//! Security invariants:
//! - Never creates or reads `node_modules`.
//! - Never executes install/lifecycle scripts (`preinstall`, `install`,
//!   `postinstall`, `prepare`, etc.).
//! - Offline by default: serves packages from [`neojs_npm::CasStore`] (and
//!   `neo.lock`), consulting the npm registry over [`neojs_remote::RemoteClient`]
//!   only when remote loading is enabled and `--offline` is not set.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use deno_core::url::Url;
use neojs_module_policy::is_path_specifier;
use neojs_npm::{
    CasStore, DEFAULT_REGISTRY_ORIGIN, ExportsResolver, LOCKFILE_NAME, LoadedNpmModule, Lockfile,
    MAX_DISCOVERY_DEPTH, NpmLimits, NpmReferrerResolution, NpmRootRequest, NpmSpecifier,
    ProjectPackageResolver, ProjectResolution, RemoteNpmRegistry, ResolvedNpmGraph, TargetPlatform,
    VersionConstraint, resolve_requests_with_lockfile, split_bare_package_specifier,
};
use neojs_remote::{RemoteClient, RemoteOptions};

/// Runtime configuration for `npm:` and `package.json` module resolution.
#[derive(Clone, Debug)]
pub struct NpmOptions {
    enabled: bool,
    frozen_lockfile: bool,
    offline: bool,
    cache_dir: Option<PathBuf>,
    registry_origin: String,
}

impl Default for NpmOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            frozen_lockfile: false,
            offline: false,
            cache_dir: None,
            registry_origin: DEFAULT_REGISTRY_ORIGIN.to_owned(),
        }
    }
}

impl NpmOptions {
    /// Creates default npm options: enabled, unlocked, and using the default
    /// CAS cache directory (`$XDG_CACHE_HOME/neojs/packages` or
    /// `$HOME/.cache/neojs/packages`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables `npm:` and project `package.json` resolution at runtime.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// When `true`, refuses any npm resolution not already pinned in `neo.lock`
    /// or failing integrity verification.
    pub fn with_frozen_lockfile(mut self, frozen: bool) -> Self {
        self.frozen_lockfile = frozen;
        self
    }

    /// When `true`, refuses all network registry fetches and resolves strictly
    /// from the verified CAS cache (and `neo.lock`).
    pub fn with_offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    /// Overrides the CAS cache root directory used for npm packages.
    pub fn with_cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(dir.into());
        self
    }

    /// Overrides the npm registry origin URL (defaults to `https://registry.npmjs.org`).
    pub fn with_registry_origin(mut self, origin: impl Into<String>) -> Self {
        self.registry_origin = origin.into().trim_end_matches('/').to_owned();
        self
    }

    /// Returns whether `npm:` and `package.json` resolution is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns whether `--frozen-lockfile` mode is enabled.
    pub fn is_frozen_lockfile(&self) -> bool {
        self.frozen_lockfile
    }

    /// Returns whether `--offline` mode is enabled.
    pub fn is_offline(&self) -> bool {
        self.offline
    }

    /// Returns the explicit CAS cache directory, if configured.
    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }

    /// Returns the configured npm registry origin URL.
    pub fn registry_origin(&self) -> &str {
        &self.registry_origin
    }
}

/// Outcome of resolving an import specifier issued by an `npm://` module.
pub(crate) enum NpmReferrerAction {
    /// Resolved to another virtual `npm://` URL inside the verified graph.
    ResolvedUrl(Url),
    /// A canonical `neo:*` built-in specifier.
    DelegateNeoBuiltin(String),
    /// A canonical `node:*` built-in specifier.
    DelegateNodeBuiltin(String),
}

struct LockfileSlot {
    lock_path: PathBuf,
    persist_on_change: bool,
    lockfile: Lockfile,
}

/// Manages `package.json` discovery, `neo.lock` verification/updates, and
/// `CasStore`-backed virtual `npm://` module loading for one [`crate::Runtime`].
pub(crate) struct NpmLoader {
    options: NpmOptions,
    remote: Option<RemoteClient>,
    cas_root: Option<PathBuf>,
    manifest_root: Option<PathBuf>,
    graph: RefCell<Option<ResolvedNpmGraph>>,
    projects: RefCell<BTreeMap<PathBuf, Option<ProjectPackageResolver>>>,
    lockfiles: RefCell<BTreeMap<PathBuf, LockfileSlot>>,
}

impl NpmLoader {
    pub(crate) fn new(
        options: NpmOptions,
        remote: Option<RemoteClient>,
        remote_cache_dir: Option<PathBuf>,
        manifest_root: Option<PathBuf>,
    ) -> Self {
        let cas_root = options
            .cache_dir
            .clone()
            .or(remote_cache_dir)
            .or_else(|| RemoteOptions::new().cache_dir().map(Path::to_path_buf));
        Self {
            options,
            remote,
            cas_root,
            manifest_root,
            graph: RefCell::new(None),
            projects: RefCell::new(BTreeMap::new()),
            lockfiles: RefCell::new(BTreeMap::new()),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.options.enabled
    }

    /// Resolves an explicit `npm:...` specifier from a local `file:` referrer.
    pub(crate) fn resolve_npm_specifier(
        &self,
        specifier_str: &str,
        referrer: &str,
    ) -> Result<Url, String> {
        if !self.options.enabled {
            return Err("npm module resolution is disabled (--no-npm)".to_owned());
        }
        let spec = NpmSpecifier::parse(specifier_str).map_err(|err| err.to_string())?;
        let req = NpmRootRequest::from_specifier(&spec).map_err(|err| err.to_string())?;

        let referrer_dir = file_referrer_dir(referrer)?;
        let project_dir = self
            .discover_project_for_dir(&referrer_dir)?
            .map(|proj| proj.root_dir().to_path_buf());
        let lock_dir = self.ensure_lockfile_slot(&referrer_dir, project_dir.as_deref())?;

        self.ensure_request_resolved(&req, &lock_dir)?;
        let graph_borrow = self.graph.borrow();
        let graph = graph_borrow
            .as_ref()
            .ok_or_else(|| "internal error: npm graph not initialized".to_owned())?;
        graph
            .resolve_root_specifier(&spec)
            .map_err(|err| err.to_string())
    }

    /// Resolves a bare package specifier or `#...` import against the nearest
    /// project `package.json` for a local `file:` referrer.
    ///
    /// Returns `Ok(None)` if no `package.json` was discovered or if the specifier
    /// is a path / scheme that must not be intercepted.
    pub(crate) fn resolve_project_specifier(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Result<Option<Url>, String> {
        if !self.options.enabled {
            return Ok(None);
        }
        if is_path_specifier(specifier) || specifier.contains(':') {
            return Ok(None);
        }
        let referrer_dir = file_referrer_dir(referrer)?;
        let Some(project) = self.discover_project_for_dir(&referrer_dir)? else {
            return Ok(None);
        };

        if specifier.starts_with('#') {
            let proj_name = project
                .package_json()
                .name
                .as_deref()
                .unwrap_or("<project>");
            let target = ExportsResolver::resolve_project_import(
                project.package_json(),
                project.root_dir(),
                proj_name,
                specifier,
            )
            .map_err(|err| err.to_string())?;
            match target {
                neojs_npm::PackageImportTarget::InternalFile(_) => {
                    let empty_graph = self.empty_or_existing_graph()?;
                    match project
                        .resolve_specifier(specifier, &empty_graph)
                        .map_err(|err| err.to_string())?
                    {
                        ProjectResolution::LocalFile { path, .. } => {
                            let url = Url::from_file_path(&path).map_err(|()| {
                                format!(
                                    "resolved path `{}` is not a valid file: URL",
                                    path.display()
                                )
                            })?;
                            return Ok(Some(url));
                        }
                        ProjectResolution::NotIntercepted => return Ok(None),
                        ProjectResolution::NpmModule(url) => return Ok(Some(url)),
                    }
                }
                neojs_npm::PackageImportTarget::ExternalPackage { name, .. } => {
                    let Some((range_str, optional)) = project.dependency_constraint(&name) else {
                        return Err(format!(
                            "package `{name}` is not declared in project package.json dependencies"
                        ));
                    };
                    let constraint =
                        VersionConstraint::parse(range_str).map_err(|err| err.to_string())?;
                    let req = NpmRootRequest {
                        name,
                        constraint,
                        optional,
                    };
                    let lock_dir =
                        self.ensure_lockfile_slot(&referrer_dir, Some(project.root_dir()))?;
                    self.ensure_request_resolved(&req, &lock_dir)?;
                }
            }
        } else {
            let (pkg_name, _) =
                split_bare_package_specifier(specifier).map_err(|err| err.to_string())?;
            let is_self_ref = project.package_json().name.as_deref() == Some(pkg_name.as_str())
                && project.package_json().exports.is_some();
            if !is_self_ref {
                let Some((range_str, optional)) = project.dependency_constraint(&pkg_name) else {
                    return Err(format!(
                        "package `{pkg_name}` is not declared in project package.json dependencies"
                    ));
                };
                let constraint =
                    VersionConstraint::parse(range_str).map_err(|err| err.to_string())?;
                let req = NpmRootRequest {
                    name: pkg_name,
                    constraint,
                    optional,
                };
                let lock_dir =
                    self.ensure_lockfile_slot(&referrer_dir, Some(project.root_dir()))?;
                self.ensure_request_resolved(&req, &lock_dir)?;
            }
        }

        let graph = self.empty_or_existing_graph()?;
        match project
            .resolve_specifier(specifier, &graph)
            .map_err(|err| err.to_string())?
        {
            ProjectResolution::NotIntercepted => Ok(None),
            ProjectResolution::NpmModule(url) => Ok(Some(url)),
            ProjectResolution::LocalFile { path, .. } => {
                Url::from_file_path(&path).map(Some).map_err(|()| {
                    format!(
                        "resolved path `{}` is not a valid file: URL",
                        path.display()
                    )
                })
            }
        }
    }

    /// Resolves an import specifier issued from inside an already-resolved
    /// virtual `npm://` referrer module.
    pub(crate) fn resolve_from_npm_referrer(
        &self,
        referrer: &Url,
        specifier: &str,
    ) -> Result<NpmReferrerAction, String> {
        if !self.options.enabled {
            return Err("npm module resolution is disabled (--no-npm)".to_owned());
        }
        let graph_borrow = self.graph.borrow();
        let graph = graph_borrow
            .as_ref()
            .ok_or_else(|| format!("npm module `{referrer}` is not in the resolved graph"))?;
        match graph
            .resolve_from_npm_referrer(referrer, specifier)
            .map_err(|err| err.to_string())?
        {
            NpmReferrerResolution::NpmUrl(url) => Ok(NpmReferrerAction::ResolvedUrl(url)),
            NpmReferrerResolution::NeoBuiltin(s) => Ok(NpmReferrerAction::DelegateNeoBuiltin(s)),
            NpmReferrerResolution::NodeBuiltin(s) => Ok(NpmReferrerAction::DelegateNodeBuiltin(s)),
        }
    }

    /// Loads and content-verifies a virtual `npm://` module from [`CasStore`].
    pub(crate) fn load_module(&self, url: &Url) -> Result<LoadedNpmModule, String> {
        if !self.options.enabled {
            return Err("npm module loading is disabled (--no-npm)".to_owned());
        }
        let graph_borrow = self.graph.borrow();
        let graph = graph_borrow
            .as_ref()
            .ok_or_else(|| format!("npm module `{url}` is not in the resolved graph"))?;
        graph
            .load_module_verified(url)
            .map_err(|err| err.to_string())
    }

    fn cas_store(&self) -> Result<CasStore, String> {
        let root = self.cas_root.as_deref().ok_or_else(|| {
            "no CAS cache directory is configured; set XDG_CACHE_HOME, HOME, or NpmOptions::with_cache_dir"
                .to_owned()
        })?;
        Ok(CasStore::new(root))
    }

    fn empty_or_existing_graph(&self) -> Result<ResolvedNpmGraph, String> {
        if let Some(existing) = self.graph.borrow().as_ref() {
            return Ok(existing.clone());
        }
        let cas = self.cas_store()?;
        ResolvedNpmGraph::new(&self.options.registry_origin, cas).map_err(|err| err.to_string())
    }

    fn discover_project_for_dir(
        &self,
        dir: &Path,
    ) -> Result<Option<ProjectPackageResolver>, String> {
        let canon = match std::fs::canonicalize(dir) {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        if let Some(cached) = self.projects.borrow().get(&canon) {
            return Ok(cached.clone());
        }
        let discovered = ProjectPackageResolver::discover(&canon).map_err(|err| err.to_string())?;
        self.projects.borrow_mut().insert(canon, discovered.clone());
        Ok(discovered)
    }

    fn ensure_lockfile_slot(
        &self,
        referrer_dir: &Path,
        project_dir: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let (lock_dir, persist_on_change) = if let Some(dir) = project_dir {
            (dir.to_path_buf(), true)
        } else if let Some(dir) = &self.manifest_root {
            (dir.clone(), true)
        } else if let Some(existing_dir) = find_existing_lockfile_dir(referrer_dir) {
            (existing_dir, true)
        } else {
            (referrer_dir.to_path_buf(), false)
        };

        if self.lockfiles.borrow().contains_key(&lock_dir) {
            return Ok(lock_dir);
        }

        let lock_path = lock_dir.join(LOCKFILE_NAME);
        let slot = if std::fs::symlink_metadata(&lock_path).is_ok() {
            let lockfile = Lockfile::read_from_path(&lock_path).map_err(|err| err.to_string())?;
            LockfileSlot {
                lock_path,
                persist_on_change: true,
                lockfile,
            }
        } else if self.options.frozen_lockfile {
            return Err(format!(
                "neo.lock not found at `{}` (required by --frozen-lockfile)",
                lock_path.display()
            ));
        } else {
            LockfileSlot {
                lock_path,
                persist_on_change,
                lockfile: Lockfile::new(),
            }
        };

        self.lockfiles.borrow_mut().insert(lock_dir.clone(), slot);
        Ok(lock_dir)
    }

    fn ensure_request_resolved(&self, req: &NpmRootRequest, lock_dir: &Path) -> Result<(), String> {
        let constraint_str = req.constraint.to_string();
        if self
            .graph
            .borrow()
            .as_ref()
            .is_some_and(|g| g.root_instance_id(&req.name, &constraint_str).is_some())
        {
            return Ok(());
        }

        let cas = self.cas_store()?;
        let registry = if !self.options.offline {
            self.remote.as_ref().map(|client| {
                RemoteNpmRegistry::with_client(&self.options.registry_origin, client.clone())
            })
        } else {
            None
        };
        let registry_ref = registry.as_ref().map(|r| r as &dyn neojs_npm::NpmRegistry);

        let mut lockfiles = self.lockfiles.borrow_mut();
        let slot = lockfiles
            .get_mut(lock_dir)
            .ok_or_else(|| "internal error: missing lockfile slot".to_owned())?;

        let (sub_graph, modified) = resolve_requests_with_lockfile(
            std::slice::from_ref(req),
            &mut slot.lockfile,
            &cas,
            registry_ref,
            &self.options.registry_origin,
            self.options.frozen_lockfile,
            self.options.offline,
            NpmLimits::default(),
            &TargetPlatform::current(),
        )
        .map_err(|err| err.to_string())?;

        if modified
            && slot.persist_on_change
            && !self.options.frozen_lockfile
            && !self.options.offline
        {
            slot.lockfile
                .write_atomic(&slot.lock_path)
                .map_err(|err| err.to_string())?;
        }

        let mut graph_slot = self.graph.borrow_mut();
        match graph_slot.as_mut() {
            Some(existing) => existing.merge(sub_graph),
            None => *graph_slot = Some(sub_graph),
        }
        Ok(())
    }
}

fn file_referrer_dir(referrer: &str) -> Result<PathBuf, String> {
    let url = Url::parse(referrer)
        .map_err(|err| format!("cannot resolve npm dependency from non-URL referrer: {err}"))?;
    let path = url
        .to_file_path()
        .map_err(|()| "cannot resolve npm dependency from non-file referrer".to_owned())?;
    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "referrer file path has no parent directory".to_owned())
}

fn find_existing_lockfile_dir(start_dir: &Path) -> Option<PathBuf> {
    let mut current = std::fs::canonicalize(start_dir).ok()?;
    for _ in 0..MAX_DISCOVERY_DEPTH {
        let candidate = current.join(LOCKFILE_NAME);
        if std::fs::symlink_metadata(&candidate).is_ok() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}
