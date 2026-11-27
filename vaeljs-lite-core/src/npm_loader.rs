//! `npm:` and `package.json` module resolution and verified CAS loading.
//!
//! Compiled only when the `npm` Cargo feature is enabled.
//!
//! Security invariants:
//! - Never creates or reads `node_modules`.
//! - Never executes install/lifecycle scripts (`preinstall`, `install`,
//!   `postinstall`, `prepare`, etc.).
//! - Offline by default: serves packages from [`vaeljs_npm::CasStore`] (and
//!   `vael.lock`), consulting the npm registry over [`vaeljs_remote::RemoteClient`]
//!   only when remote loading is enabled and `--offline` is not set.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use url::Url;
use vaeljs_module_policy::is_path_specifier;
use vaeljs_npm::{
    CasStore, DEFAULT_REGISTRY_ORIGIN, ExportsResolver, LOCKFILE_NAME, LoadedNpmModule, Lockfile,
    MAX_DISCOVERY_DEPTH, NpmLimits, NpmReferrerResolution, NpmRootRequest, NpmSpecifier,
    NpmrcConfig, PackageName, ProjectPackageResolver, ProjectResolution, RemoteNpmRegistry,
    ResolvedNpmGraph, ResolverError, TargetPlatform, VersionConstraint,
    resolve_requests_with_lockfile_and_npmrc, split_bare_package_specifier,
};
use vaeljs_remote::{RemoteClient, RemoteOptions};

use crate::loader::is_windows_drive_path;

const NPMRC_NAME: &str = ".npmrc";

/// Runtime configuration for `npm:` and `package.json` module resolution.
#[derive(Clone, Debug)]
pub struct NpmOptions {
    enabled: bool,
    frozen_lockfile: bool,
    offline: bool,
    cache_dir: Option<PathBuf>,
    registry_origin: String,
    registry_origin_overridden: bool,
    discover_project_npmrc: bool,
    allow_project_npmrc_auth: bool,
    npmrc_home_dir: Option<PathBuf>,
}

impl Default for NpmOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            frozen_lockfile: false,
            offline: false,
            cache_dir: None,
            registry_origin: DEFAULT_REGISTRY_ORIGIN.to_owned(),
            registry_origin_overridden: false,
            discover_project_npmrc: true,
            allow_project_npmrc_auth: false,
            npmrc_home_dir: None,
        }
    }
}

impl NpmOptions {
    /// Creates default npm options: enabled, unlocked, and using the default
    /// CAS cache directory (`$XDG_CACHE_HOME/vaeljs/packages` or
    /// `$HOME/.cache/vaeljs/packages`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables `npm:` and project `package.json` resolution at runtime.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// When `true`, refuses any npm resolution not already pinned in `vael.lock`
    /// or failing integrity verification.
    pub fn with_frozen_lockfile(mut self, frozen: bool) -> Self {
        self.frozen_lockfile = frozen;
        self
    }

    /// When `true`, refuses all network registry fetches and resolves strictly
    /// from the verified CAS cache (and `vael.lock`).
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
        self.registry_origin_overridden = true;
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

    /// Controls whether project-level `.npmrc` discovery is enabled (defaults
    /// to `true`; CLI `--no-config` sets this to `false`).
    pub fn with_discover_project_npmrc(mut self, discover: bool) -> Self {
        self.discover_project_npmrc = discover;
        self
    }

    /// Explicitly opts in to reading auth directives and `${ENV_VAR}`
    /// interpolation from project-level `.npmrc` files (defaults to `false`).
    ///
    /// Leave `false` for untrusted repositories so credentials and `${ENV_VAR}`
    /// expansion are only read from user-level `~/.npmrc`.
    pub fn with_allow_project_npmrc_auth(mut self, allow: bool) -> Self {
        self.allow_project_npmrc_auth = allow;
        self
    }

    /// Overrides the home directory used to locate `~/.npmrc`.
    pub fn with_npmrc_home_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.npmrc_home_dir = Some(dir.into());
        self
    }

    /// Returns whether project-level `.npmrc` discovery is enabled.
    pub fn discover_project_npmrc(&self) -> bool {
        self.discover_project_npmrc
    }

    /// Returns whether auth directives and `${ENV_VAR}` interpolation from
    /// project-level `.npmrc` files are allowed.
    pub fn allow_project_npmrc_auth(&self) -> bool {
        self.allow_project_npmrc_auth
    }

    /// Returns the overridden home directory used to locate `~/.npmrc`, if set.
    pub fn npmrc_home_dir(&self) -> Option<&Path> {
        self.npmrc_home_dir.as_deref()
    }
}

/// Outcome of resolving a bare specifier or `#imports` from a local project `package.json`.
pub(crate) enum ProjectTarget {
    LocalFile(PathBuf),
    NpmModule(Url),
}

/// Outcome of resolving an import specifier issued by an `npm://` module.
pub(crate) enum NpmReferrerAction {
    /// Resolved to another virtual `npm://` URL inside the verified graph.
    ResolvedUrl(Url),
    /// A canonical `vael:*` built-in specifier.
    DelegateVaelBuiltin(String),
    /// A canonical `node:*` built-in specifier.
    DelegateNodeBuiltin(String),
}

struct LockfileSlot {
    lock_path: PathBuf,
    persist_on_change: bool,
    lockfile: Lockfile,
}

/// Manages `package.json` discovery, `vael.lock` verification/updates, and
/// `CasStore`-backed virtual `npm://` module loading for one [`crate::Runtime`].
pub(crate) struct NpmLoader {
    options: NpmOptions,
    remote: Option<RemoteClient>,
    cas_root: Option<PathBuf>,
    manifest_root: Option<PathBuf>,
    graph: RefCell<Option<ResolvedNpmGraph>>,
    projects: RefCell<BTreeMap<PathBuf, Option<ProjectPackageResolver>>>,
    lockfiles: RefCell<BTreeMap<PathBuf, LockfileSlot>>,
    npmrcs: RefCell<BTreeMap<PathBuf, NpmrcConfig>>,
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
            npmrcs: RefCell::new(BTreeMap::new()),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.options.enabled
    }

    /// Resolves an explicit `npm:...` specifier from a local referrer.
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

        let referrer_dir = local_referrer_dir(referrer)?;
        let project = self.discover_project_for_dir(&referrer_dir)?;
        let project_dir = project.as_ref().map(|proj| proj.root_dir().to_path_buf());
        let lock_dir = self.ensure_lockfile_slot(&referrer_dir, project_dir.as_deref())?;

        self.ensure_request_resolved(&req, &referrer_dir, &lock_dir, project.as_ref())?;
        let graph_borrow = self.graph.borrow();
        let graph = graph_borrow
            .as_ref()
            .ok_or_else(|| "internal error: npm graph not initialized".to_owned())?;
        graph
            .resolve_root_specifier(&spec)
            .map_err(|err| err.to_string())
    }

    /// Resolves a bare package specifier or `#...` import against the nearest
    /// project `package.json` for a local referrer.
    ///
    /// Returns `Ok(None)` if no `package.json` was discovered or if the specifier
    /// is a path / scheme that must not be intercepted.
    pub(crate) fn resolve_project_specifier(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Result<Option<ProjectTarget>, String> {
        if !self.options.enabled {
            return Ok(None);
        }
        if is_path_specifier(specifier) || specifier.contains(':') {
            return Ok(None);
        }
        let referrer_dir = local_referrer_dir(referrer)?;
        let Some(project) = self.discover_project_for_dir(&referrer_dir)? else {
            return Ok(None);
        };

        // Resolve local `#imports` and package self-references directly without
        // touching CasStore (works even when XDG_CACHE_HOME/HOME are unset).
        if let Some(local_res) = project
            .resolve_local_specifier(specifier)
            .map_err(|err| err.to_string())?
        {
            return match local_res {
                ProjectResolution::NotIntercepted => Ok(None),
                ProjectResolution::LocalFile { path, .. } => {
                    Ok(Some(ProjectTarget::LocalFile(path)))
                }
                ProjectResolution::NpmModule(url) => Ok(Some(ProjectTarget::NpmModule(url))),
            };
        }

        let target_name = if specifier.starts_with('#') {
            let proj_name = project
                .package_json()
                .name
                .as_deref()
                .unwrap_or("<project>");
            match ExportsResolver::resolve_project_import(
                project.package_json(),
                project.root_dir(),
                proj_name,
                specifier,
            )
            .map_err(|err| err.to_string())?
            {
                vaeljs_npm::PackageImportTarget::ExternalPackage { name, .. } => name,
                vaeljs_npm::PackageImportTarget::InternalFile(_) => {
                    unreachable!("InternalFile is handled by resolve_local_specifier")
                }
            }
        } else {
            split_bare_package_specifier(specifier)
                .map_err(|err| err.to_string())?
                .0
        };

        let Some((range_str, optional)) = project.dependency_constraint(&target_name) else {
            return Err(format!(
                "package `{target_name}` is not declared in project package.json dependencies"
            ));
        };
        let constraint = VersionConstraint::parse(range_str).map_err(|err| err.to_string())?;
        let req = NpmRootRequest {
            name: target_name,
            constraint,
            optional,
        };
        let lock_dir = self.ensure_lockfile_slot(&referrer_dir, Some(project.root_dir()))?;
        self.ensure_request_resolved(&req, &referrer_dir, &lock_dir, Some(&project))?;

        let graph_borrow = self.graph.borrow();
        let graph = graph_borrow
            .as_ref()
            .ok_or_else(|| "internal error: npm graph not initialized".to_owned())?;
        match project
            .resolve_specifier(specifier, graph)
            .map_err(|err| err.to_string())?
        {
            ProjectResolution::NotIntercepted => Ok(None),
            ProjectResolution::NpmModule(url) => Ok(Some(ProjectTarget::NpmModule(url))),
            ProjectResolution::LocalFile { path, .. } => Ok(Some(ProjectTarget::LocalFile(path))),
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
            NpmReferrerResolution::VaelBuiltin(s) => Ok(NpmReferrerAction::DelegateVaelBuiltin(s)),
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
                "vael.lock not found at `{}` (required by --frozen-lockfile)",
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

    fn discover_npmrc_dir(&self, referrer_dir: &Path, project_dir: Option<&Path>) -> PathBuf {
        if let Some(dir) = project_dir {
            return dir.to_path_buf();
        }
        if let Some(dir) = &self.manifest_root {
            return dir.clone();
        }
        if let Some(dir) = find_existing_npmrc_dir(referrer_dir) {
            return dir;
        }
        referrer_dir.to_path_buf()
    }

    fn load_npmrc_for_dir(&self, project_dir: Option<&Path>) -> Result<NpmrcConfig, String> {
        let key = project_dir
            .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()))
            .unwrap_or_default();
        if let Some(cached) = self.npmrcs.borrow().get(&key) {
            return Ok(cached.clone());
        }
        let home_dir = self.options.npmrc_home_dir.clone().or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .filter(|val| !val.is_empty())
                .map(PathBuf::from)
        });
        let config = NpmrcConfig::load_for_project_with_options(
            project_dir.map(|_| key.as_path()),
            home_dir.as_deref(),
            &|name| std::env::var(name).ok(),
            self.options.allow_project_npmrc_auth,
        )
        .map_err(|err| err.to_string())?;
        self.npmrcs.borrow_mut().insert(key, config.clone());
        Ok(config)
    }

    fn ensure_request_resolved(
        &self,
        req: &NpmRootRequest,
        referrer_dir: &Path,
        lock_dir: &Path,
        project: Option<&ProjectPackageResolver>,
    ) -> Result<(), String> {
        let constraint_str = req.constraint.to_string();
        if self
            .graph
            .borrow()
            .as_ref()
            .is_some_and(|g| g.root_instance_id(&req.name, &constraint_str).is_some())
        {
            return Ok(());
        }

        let npmrc_dir = if self.options.discover_project_npmrc {
            Some(self.discover_npmrc_dir(referrer_dir, project.map(|p| p.root_dir())))
        } else {
            None
        };
        let npmrc = self.load_npmrc_for_dir(npmrc_dir.as_deref())?;
        let effective_origin =
            if !self.options.registry_origin_overridden && npmrc.has_explicit_default_registry() {
                npmrc.default_registry_origin().to_owned()
            } else {
                self.options.registry_origin.clone()
            };

        let cas = self.cas_store()?;
        let registry = if !self.options.offline {
            self.remote.as_ref().map(|client| {
                RemoteNpmRegistry::with_client_and_npmrc(
                    &effective_origin,
                    client.clone(),
                    npmrc.clone(),
                    self.options.registry_origin_overridden,
                )
            })
        } else {
            None
        };
        let registry_ref = registry.as_ref().map(|r| r as &dyn vaeljs_npm::NpmRegistry);

        let mut lockfiles = self.lockfiles.borrow_mut();
        let slot = lockfiles
            .get_mut(lock_dir)
            .ok_or_else(|| "internal error: missing lockfile slot".to_owned())?;

        let mut requests = vec![req.clone()];
        let mut seen_peers = BTreeSet::from([req.name.clone()]);
        let max_attempts = 1 + project.map_or(0, |p| {
            p.package_json().dependencies.len()
                + p.package_json().dev_dependencies.len()
                + p.package_json().optional_dependencies.len()
        });

        let (sub_graph, modified) = loop {
            match resolve_requests_with_lockfile_and_npmrc(
                &requests,
                &mut slot.lockfile,
                &cas,
                registry_ref,
                &effective_origin,
                self.options.frozen_lockfile,
                self.options.offline,
                NpmLimits::default(),
                &TargetPlatform::current(),
                Some(&npmrc),
                self.options.registry_origin_overridden,
            ) {
                Ok(res) => break res,
                Err(ResolverError::MissingPeerDependency { ref peer, .. })
                    if requests.len() < max_attempts =>
                {
                    let Some(proj) = project else {
                        return Err(format!(
                            "package `{}` requires peer `{peer}`, which is missing",
                            req.name
                        ));
                    };
                    let peer_name = PackageName::parse(peer).map_err(|e| e.to_string())?;
                    if !seen_peers.insert(peer_name.clone()) {
                        return Err(format!("missing peer dependency `{peer}`"));
                    }
                    let Some((range_str, optional)) = proj.dependency_constraint(&peer_name) else {
                        return Err(format!(
                            "package `{}` requires peer `{peer}`, which is not declared in project package.json",
                            req.name
                        ));
                    };
                    let constraint =
                        VersionConstraint::parse(range_str).map_err(|e| e.to_string())?;
                    requests.push(NpmRootRequest {
                        name: peer_name,
                        constraint,
                        optional,
                    });
                }
                Err(err) => return Err(err.to_string()),
            }
        };

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

fn local_referrer_dir(referrer: &str) -> Result<PathBuf, String> {
    if referrer.is_empty() {
        return Err("cannot resolve npm dependency from empty referrer".to_owned());
    }
    let path = if is_windows_drive_path(referrer) {
        PathBuf::from(referrer)
    } else if let Ok(url) = Url::parse(referrer) {
        if url.scheme() != "file" {
            return Err("cannot resolve npm dependency from non-file referrer".to_owned());
        }
        url.to_file_path()
            .map_err(|()| "cannot resolve npm dependency from non-file referrer".to_owned())?
    } else {
        PathBuf::from(referrer)
    };
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.to_path_buf()),
        _ => Ok(PathBuf::from(".")),
    }
}

fn find_existing_lockfile_dir(start_dir: &Path) -> Option<PathBuf> {
    find_existing_file_dir(start_dir, LOCKFILE_NAME)
}

fn find_existing_npmrc_dir(start_dir: &Path) -> Option<PathBuf> {
    find_existing_file_dir(start_dir, NPMRC_NAME)
}

fn find_existing_file_dir(start_dir: &Path, file_name: &str) -> Option<PathBuf> {
    let mut current = std::fs::canonicalize(start_dir).ok()?;
    for _ in 0..MAX_DISCOVERY_DEPTH {
        let candidate = current.join(file_name);
        if std::fs::symlink_metadata(&candidate).is_ok() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}
