//! Command-line entry point for VaelJS Lite on QuickJS.

#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vaeljs_dts::Profile;
#[cfg(feature = "npm")]
use vaeljs_lite_core::NpmOptions;
#[cfg(feature = "remote")]
use vaeljs_lite_core::RemoteOptions;
use vaeljs_lite_core::{Error, ImportResolver, Runtime, RuntimeOptions, TestReport};
use vaeljs_manifest::{
    DiscoveredManifest, MAX_DISCOVERY_DEPTH, Manifest, Workspace, discover_manifest,
};

/// Default program name shown when `argv[0]` is missing or unusable.
const DEFAULT_PROG_NAME: &str = "vjs-lite";
/// Exit code for a module or test that failed while loading or running.
const EXIT_RUNTIME_FAILURE: u8 = 1;
/// Exit code for a command vjs-lite could not start.
const EXIT_STARTUP_FAILURE: u8 = 2;

/// Persistent remote-loading setting for both run and test commands.
const ALLOW_REMOTE_ENV: &str = "VAELJS_ALLOW_REMOTE";

/// Derives the CLI program name from `argv[0]`, falling back to `default`.
fn program_name_from_arg0(arg0: Option<&OsStr>, default: &str) -> String {
    let Some(raw) = arg0 else {
        return default.to_string();
    };
    let Some(file_name) = Path::new(raw).file_name().and_then(OsStr::to_str) else {
        return default.to_string();
    };
    let Some(base) = file_name.rsplit(['/', '\\']).next() else {
        return default.to_string();
    };
    let name = if base.len() >= 4 && base.as_bytes()[base.len() - 4..].eq_ignore_ascii_case(b".exe")
    {
        &base[..base.len() - 4]
    } else {
        base
    };
    if name.is_empty() || name.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        default.to_string()
    } else {
        name.to_string()
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let prog_name = program_name_from_arg0(args.next().as_deref(), DEFAULT_PROG_NAME);
    let command = match parse_args(args.collect()) {
        Ok(command) => command,
        Err(message) => return startup_failure(&prog_name, &message),
    };

    match command {
        Command::Help => {
            print!("{}", help(&prog_name));
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("vjs-lite {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Run {
            input,
            allow_remote,
            config,
            npm,
            node_compat,
        } => match resolve_allow_remote(allow_remote) {
            Ok(allow_remote) => run(&prog_name, input, allow_remote, &config, npm, node_compat),
            Err(message) => startup_failure(&prog_name, &message),
        },
        Command::Test {
            patterns,
            allow_remote,
            config,
            npm,
            node_compat,
        } => match resolve_allow_remote(allow_remote) {
            Ok(allow_remote) => run_tests(
                &prog_name,
                &patterns,
                allow_remote,
                &config,
                npm,
                node_compat,
            ),
            Err(message) => startup_failure(&prog_name, &message),
        },
        Command::Types { profile, out_dir } => run_types(&prog_name, profile, out_dir.as_deref()),
        Command::TypesHelp => {
            print!("{}", types_help(&prog_name));
            ExitCode::SUCCESS
        }
    }
}

fn startup_failure(prog_name: &str, message: &str) -> ExitCode {
    eprintln!("{prog_name}: {message}");
    eprintln!("Try '{prog_name} --help'.");
    ExitCode::from(EXIT_STARTUP_FAILURE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NpmFlags {
    enabled: bool,
    frozen_lockfile: bool,
    offline: bool,
}

impl Default for NpmFlags {
    fn default() -> Self {
        Self {
            enabled: true,
            frozen_lockfile: false,
            offline: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run {
        input: Input,
        allow_remote: AllowRemote,
        config: ConfigChoice,
        npm: NpmFlags,
        node_compat: bool,
    },
    Test {
        patterns: Vec<String>,
        allow_remote: AllowRemote,
        config: ConfigChoice,
        npm: NpmFlags,
        node_compat: bool,
    },
    Types {
        profile: Profile,
        out_dir: Option<PathBuf>,
    },
    TypesHelp,
    Help,
    Version,
}

/// The `vael.jsonc` manifest selection made on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigChoice {
    Auto,
    Disabled,
    Explicit(PathBuf),
}

impl ConfigChoice {
    fn set_explicit(&mut self, path: PathBuf) -> Result<(), String> {
        match self {
            Self::Auto => {
                *self = Self::Explicit(path);
                Ok(())
            }
            Self::Disabled => {
                Err("`--config` and `--no-config` cannot be used together".to_string())
            }
            Self::Explicit(_) => Err("`--config` cannot be specified more than once".to_string()),
        }
    }

    fn set_disabled(&mut self) -> Result<(), String> {
        match self {
            Self::Auto | Self::Disabled => {
                *self = Self::Disabled;
                Ok(())
            }
            Self::Explicit(_) => {
                Err("`--config` and `--no-config` cannot be used together".to_string())
            }
        }
    }
}

/// The remote-loading choice made on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowRemote {
    Unspecified,
    #[cfg_attr(not(feature = "remote"), allow(dead_code))]
    Enabled,
    Disabled,
}

impl AllowRemote {
    fn choose(&mut self, requested: Self) -> Result<(), String> {
        if *self != Self::Unspecified && *self != requested {
            return Err(
                "`--allow-remote` and `--no-allow-remote` cannot be used together".to_string(),
            );
        }
        *self = requested;
        Ok(())
    }
}

fn resolve_allow_remote(choice: AllowRemote) -> Result<bool, String> {
    let configured = match std::env::var_os(ALLOW_REMOTE_ENV) {
        Some(value) => parse_boolean_env(ALLOW_REMOTE_ENV, &value)?,
        None => false,
    };
    let enabled = match choice {
        AllowRemote::Unspecified => configured,
        AllowRemote::Enabled => true,
        AllowRemote::Disabled => false,
    };
    #[cfg(not(feature = "remote"))]
    if enabled {
        return Err(
            "`VAELJS_ALLOW_REMOTE` requires `vaeljs-lite` built with the `remote` feature"
                .to_string(),
        );
    }
    Ok(enabled)
}

fn parse_boolean_env(name: &str, value: &OsStr) -> Result<bool, String> {
    let Some(text) = value.to_str() else {
        return Err(boolean_env_error(name));
    };
    if text == "1" || text.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if text == "0" || text.eq_ignore_ascii_case("false") {
        return Ok(false);
    }
    Err(boolean_env_error(name))
}

fn boolean_env_error(name: &str) -> String {
    format!("invalid {name}: expected `1`, `0`, `true`, or `false`")
}

#[derive(Debug, PartialEq, Eq)]
enum Input {
    Path(PathBuf),
    Url(String),
}

fn parse_args(args: Vec<OsString>) -> Result<Command, String> {
    let mut allow_remote = AllowRemote::Unspecified;
    let mut config = ConfigChoice::Auto;
    #[cfg_attr(not(feature = "npm"), allow(unused_mut))]
    let mut npm = NpmFlags::default();
    let mut node_compat = true;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.to_str() {
            Some("--allow-remote") => {
                #[cfg(not(feature = "remote"))]
                return Err(
                    "`--allow-remote` requires `vaeljs-lite` built with the `remote` feature"
                        .to_string(),
                );
                #[cfg(feature = "remote")]
                {
                    allow_remote.choose(AllowRemote::Enabled)?;
                    index += 1;
                }
            }
            Some("--no-allow-remote") => {
                allow_remote.choose(AllowRemote::Disabled)?;
                index += 1;
            }
            Some("--no-npm") => {
                #[cfg(not(feature = "npm"))]
                return Err(
                    "`--no-npm` requires `vaeljs-lite` built with the `npm` feature".to_string(),
                );
                #[cfg(feature = "npm")]
                {
                    npm.enabled = false;
                    index += 1;
                }
            }
            Some("--frozen-lockfile") => {
                #[cfg(not(feature = "npm"))]
                return Err(
                    "`--frozen-lockfile` requires `vaeljs-lite` built with the `npm` feature"
                        .to_string(),
                );
                #[cfg(feature = "npm")]
                {
                    npm.frozen_lockfile = true;
                    index += 1;
                }
            }
            Some("--offline") => {
                #[cfg(not(feature = "npm"))]
                return Err(
                    "`--offline` requires `vaeljs-lite` built with the `npm` feature".to_string(),
                );
                #[cfg(feature = "npm")]
                {
                    npm.offline = true;
                    index += 1;
                }
            }
            Some("--no-config") => {
                config.set_disabled()?;
                index += 1;
            }
            Some("--no-node-compat") => {
                node_compat = false;
                index += 1;
            }
            Some("--config") => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "`--config` requires a path".to_string())?;
                if value.is_empty() {
                    return Err("`--config` requires a non-empty path".to_string());
                }
                config.set_explicit(PathBuf::from(value))?;
                index += 2;
            }
            Some(value) if value.starts_with("--config=") => {
                let path = &value["--config=".len()..];
                if path.is_empty() {
                    return Err("`--config` requires a non-empty path".to_string());
                }
                config.set_explicit(PathBuf::from(path))?;
                index += 1;
            }
            _ => break,
        }
    }

    let rest = &args[index..];
    let Some(first) = rest.first() else {
        return Err("no input file".to_string());
    };

    if let Some(option) = first.to_str() {
        match option {
            "-h" | "--help" if rest.len() == 1 => return Ok(Command::Help),
            "-V" | "--version" if rest.len() == 1 => return Ok(Command::Version),
            "--" => {
                if rest.len() != 2 {
                    return Err("expected exactly one input file after `--`".to_string());
                }
                return Ok(Command::Run {
                    input: classify_input(&rest[1]),
                    allow_remote,
                    config,
                    npm,
                    node_compat,
                });
            }
            "test" => return parse_test_args(&rest[1..], allow_remote, config, npm, node_compat),
            "types" => {
                if allow_remote != AllowRemote::Unspecified
                    || config != ConfigChoice::Auto
                    || !node_compat
                {
                    return Err(
                        "`types` does not accept `--allow-remote`, `--no-allow-remote`, `--no-node-compat`, `--config`, or `--no-config`"
                            .to_string(),
                    );
                }
                if npm != NpmFlags::default() {
                    return Err(
                        "`types` does not accept `--no-npm`, `--frozen-lockfile`, or `--offline`"
                            .to_string(),
                    );
                }
                return parse_types_args(&rest[1..]);
            }
            _ => {}
        }
    }

    if rest.len() > 1 {
        return Err(format!("expected one input file, got {}", rest.len()));
    }
    if first.to_str().is_some_and(|arg| arg.starts_with('-')) {
        return Err(format!("unknown option `{}`", first.to_string_lossy()));
    }
    Ok(Command::Run {
        input: classify_input(first),
        allow_remote,
        config,
        npm,
        node_compat,
    })
}

/// Parses `vjs-lite test <file-or-glob>...`.
fn parse_test_args(
    args: &[OsString],
    mut allow_remote: AllowRemote,
    mut config: ConfigChoice,
    #[cfg_attr(not(feature = "npm"), allow(unused_mut))] mut npm: NpmFlags,
    mut node_compat: bool,
) -> Result<Command, String> {
    if args.len() == 1 && matches!(args[0].to_str(), Some("-h" | "--help")) {
        return Ok(Command::Help);
    }
    let mut patterns = Vec::new();
    let mut after_double_dash = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        let Some(value) = arg.to_str() else {
            return Err("test file or glob names must be valid UTF-8".to_string());
        };
        if !after_double_dash && value == "--" {
            after_double_dash = true;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--allow-remote" {
            #[cfg(not(feature = "remote"))]
            return Err(
                "`--allow-remote` requires `vaeljs-lite` built with the `remote` feature"
                    .to_string(),
            );
            #[cfg(feature = "remote")]
            {
                allow_remote.choose(AllowRemote::Enabled)?;
                index += 1;
                continue;
            }
        }
        if !after_double_dash && value == "--no-allow-remote" {
            allow_remote.choose(AllowRemote::Disabled)?;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--no-npm" {
            #[cfg(not(feature = "npm"))]
            return Err(
                "`--no-npm` requires `vaeljs-lite` built with the `npm` feature".to_string(),
            );
            #[cfg(feature = "npm")]
            {
                npm.enabled = false;
                index += 1;
                continue;
            }
        }
        if !after_double_dash && value == "--frozen-lockfile" {
            #[cfg(not(feature = "npm"))]
            return Err(
                "`--frozen-lockfile` requires `vaeljs-lite` built with the `npm` feature"
                    .to_string(),
            );
            #[cfg(feature = "npm")]
            {
                npm.frozen_lockfile = true;
                index += 1;
                continue;
            }
        }
        if !after_double_dash && value == "--offline" {
            #[cfg(not(feature = "npm"))]
            return Err(
                "`--offline` requires `vaeljs-lite` built with the `npm` feature".to_string(),
            );
            #[cfg(feature = "npm")]
            {
                npm.offline = true;
                index += 1;
                continue;
            }
        }
        if !after_double_dash && value == "--no-config" {
            config.set_disabled()?;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--no-node-compat" {
            node_compat = false;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--config" {
            let path = args
                .get(index + 1)
                .ok_or_else(|| "`--config` requires a path".to_string())?;
            if path.is_empty() {
                return Err("`--config` requires a non-empty path".to_string());
            }
            config.set_explicit(PathBuf::from(path))?;
            index += 2;
            continue;
        }
        if !after_double_dash && let Some(path) = value.strip_prefix("--config=") {
            if path.is_empty() {
                return Err("`--config` requires a non-empty path".to_string());
            }
            config.set_explicit(PathBuf::from(path))?;
            index += 1;
            continue;
        }
        if !after_double_dash && value.starts_with('-') {
            return Err(format!("unknown option `{value}`"));
        }
        patterns.push(value.to_string());
        index += 1;
    }
    if patterns.is_empty() {
        return Err("no test files or globs given".to_string());
    }
    Ok(Command::Test {
        patterns,
        allow_remote,
        config,
        npm,
        node_compat,
    })
}

/// Parses `vjs-lite types [--profile <lite|v8>] [--out-dir <dir>]`.
fn parse_types_args(args: &[OsString]) -> Result<Command, String> {
    if args.len() == 1 && matches!(args[0].to_str(), Some("-h" | "--help")) {
        return Ok(Command::TypesHelp);
    }

    let mut profile: Option<Profile> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        let Some(value) = arg.to_str() else {
            return Err("`types` options must be valid UTF-8".to_string());
        };
        if value == "--profile" {
            if profile.is_some() {
                return Err("`--profile` cannot be specified more than once".to_string());
            }
            let raw = args
                .get(index + 1)
                .ok_or_else(|| "`--profile` requires a value (`lite` or `v8`)".to_string())?;
            let text = raw
                .to_str()
                .ok_or_else(|| "`--profile` must be valid UTF-8".to_string())?;
            profile = Some(parse_profile(text)?);
            index += 2;
            continue;
        }
        if let Some(text) = value.strip_prefix("--profile=") {
            if profile.is_some() {
                return Err("`--profile` cannot be specified more than once".to_string());
            }
            profile = Some(parse_profile(text)?);
            index += 1;
            continue;
        }
        if value == "--out-dir" {
            if out_dir.is_some() {
                return Err("`--out-dir` cannot be specified more than once".to_string());
            }
            let dir = args
                .get(index + 1)
                .ok_or_else(|| "`--out-dir` requires a directory".to_string())?;
            if dir.is_empty() {
                return Err("`--out-dir` requires a non-empty directory".to_string());
            }
            out_dir = Some(PathBuf::from(dir));
            index += 2;
            continue;
        }
        if let Some(dir) = value.strip_prefix("--out-dir=") {
            if out_dir.is_some() {
                return Err("`--out-dir` cannot be specified more than once".to_string());
            }
            if dir.is_empty() {
                return Err("`--out-dir` requires a non-empty directory".to_string());
            }
            out_dir = Some(PathBuf::from(dir));
            index += 1;
            continue;
        }
        if value.starts_with('-') {
            return Err(format!("unknown option `{value}`"));
        }
        return Err(format!("unexpected argument `{value}`"));
    }

    Ok(Command::Types {
        profile: profile.unwrap_or(Profile::Lite),
        out_dir,
    })
}

fn parse_profile(value: &str) -> Result<Profile, String> {
    match value {
        "lite" => Ok(Profile::Lite),
        "v8" => Ok(Profile::V8),
        _ => Err(format!(
            "invalid profile `{value}`: expected `lite` or `v8`"
        )),
    }
}

/// Decides whether an input names a local path or a remote URL (`https:`,
/// `http:`, `gh:`, `github:`, `jsr:`).
fn classify_input(input: &OsString) -> Input {
    if let Some(text) = input.to_str() {
        let https = text
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"));
        let http = text
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"));
        let github = text
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("gh:"))
            || text
                .get(..7)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("github:"));
        let jsr = text
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("jsr:"));
        if https || http || github || jsr {
            return Input::Url(text.to_string());
        }
    }
    Input::Path(PathBuf::from(input))
}

enum LoadedConfig {
    None,
    Standalone(ImportResolver),
    Workspace(Box<Workspace>),
}

impl LoadedConfig {
    fn from_choice(choice: &ConfigChoice) -> Result<Option<Self>, String> {
        match choice {
            ConfigChoice::Disabled => Ok(Some(Self::None)),
            ConfigChoice::Explicit(path) => load_explicit_config(path).map(Some),
            ConfigChoice::Auto => Ok(None),
        }
    }

    fn resolver_for_entry(
        &self,
        entry_path: Option<&Path>,
    ) -> Result<Option<ImportResolver>, String> {
        match self {
            Self::None => Ok(None),
            Self::Standalone(resolver) => Ok(Some(resolver.clone())),
            Self::Workspace(workspace) => {
                let active = match entry_path {
                    Some(path) if std::fs::symlink_metadata(path).is_ok() => {
                        workspace.member_for_path(path).map_err(|err| {
                            format!("cannot resolve `{}` in workspace: {err}", path.display())
                        })?
                    }
                    _ => None,
                };
                let resolver = ImportResolver::from_workspace(workspace, active.map(|m| m.name()))
                    .map_err(|err| format!("cannot build workspace import resolver: {err}"))?;
                Ok(Some(resolver))
            }
        }
    }
}

fn load_explicit_config(path: &Path) -> Result<LoadedConfig, String> {
    let manifest = Manifest::from_path(path)
        .map_err(|err| format!("cannot load manifest `{}`: {err}", path.display()))?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|err| format!("cannot resolve manifest `{}`: {err}", path.display()))?;
    if !manifest.workspaces.is_empty() {
        let workspace = Workspace::load(&canonical)
            .map_err(|err| format!("cannot load workspace `{}`: {err}", path.display()))?;
        return Ok(LoadedConfig::Workspace(Box::new(workspace)));
    }
    let dir = canonical
        .parent()
        .ok_or_else(|| format!("manifest `{}` has no parent directory", path.display()))?
        .to_path_buf();
    let discovered = DiscoveredManifest {
        path: canonical,
        dir,
        manifest,
    };
    Ok(LoadedConfig::Standalone(ImportResolver::from_manifest(
        &discovered,
    )))
}

fn discover_resolver_for_entry(entry_path: &Path) -> Result<Option<ImportResolver>, String> {
    let abs_entry = std::path::absolute(entry_path)
        .map_err(|err| format!("cannot resolve `{}`: {err}", entry_path.display()))?;
    let Some(entry_dir) = abs_entry.parent() else {
        return Ok(None);
    };
    if !entry_dir.is_dir() {
        return Ok(None);
    }
    let Some(discovered) = discover_manifest(entry_dir, None)
        .map_err(|err| format!("cannot load manifest for `{}`: {err}", entry_path.display()))?
    else {
        return Ok(None);
    };
    if !discovered.manifest.workspaces.is_empty() {
        let workspace = Workspace::load(&discovered.path).map_err(|err| {
            format!(
                "cannot load workspace `{}`: {err}",
                discovered.path.display()
            )
        })?;
        let active = if std::fs::symlink_metadata(&abs_entry).is_ok() {
            workspace.member_for_path(&abs_entry).map_err(|err| {
                format!(
                    "cannot resolve `{}` in workspace: {err}",
                    entry_path.display()
                )
            })?
        } else {
            None
        };
        let resolver = ImportResolver::from_workspace(&workspace, active.map(|m| m.name()))
            .map_err(|err| {
                format!(
                    "cannot build workspace import resolver for `{}`: {err}",
                    entry_path.display()
                )
            })?;
        return Ok(Some(resolver));
    }

    let mut probe = discovered.dir.parent().map(Path::to_path_buf);
    for _ in 0..MAX_DISCOVERY_DEPTH {
        let Some(parent_dir) = probe else {
            break;
        };
        let Some(parent_discovered) = discover_manifest(&parent_dir, None)
            .map_err(|err| format!("cannot load manifest for `{}`: {err}", entry_path.display()))?
        else {
            break;
        };
        if !parent_discovered.manifest.workspaces.is_empty() {
            let workspace = Workspace::load(&parent_discovered.path).map_err(|err| {
                format!(
                    "cannot load workspace `{}`: {err}",
                    parent_discovered.path.display()
                )
            })?;
            if let Some(member) = workspace
                .members()
                .find(|member| member.manifest_path == discovered.path)
            {
                let resolver = ImportResolver::from_workspace(&workspace, Some(member.name()))
                    .map_err(|err| {
                        format!(
                            "cannot build workspace import resolver for `{}`: {err}",
                            entry_path.display()
                        )
                    })?;
                return Ok(Some(resolver));
            }
            break;
        }
        probe = parent_discovered.dir.parent().map(Path::to_path_buf);
    }

    Ok(Some(ImportResolver::from_manifest(&discovered)))
}

fn create_runtime(
    allow_remote: bool,
    resolver: Option<ImportResolver>,
    npm: NpmFlags,
    node_compat: bool,
    discover_project_npmrc: bool,
) -> Result<Runtime, Error> {
    let mut options = RuntimeOptions::new().with_node_compat(node_compat);
    #[cfg(feature = "remote")]
    if allow_remote {
        options = options.with_remote(RemoteOptions::new());
    }
    #[cfg(not(feature = "remote"))]
    let _ = allow_remote;
    if let Some(resolver) = resolver {
        options = options.with_import_resolver(resolver);
    }
    #[cfg(feature = "npm")]
    {
        let npm_options = NpmOptions::new()
            .with_enabled(npm.enabled)
            .with_frozen_lockfile(npm.frozen_lockfile)
            .with_offline(npm.offline)
            .with_discover_project_npmrc(discover_project_npmrc);
        options = options.with_npm(npm_options);
    }
    #[cfg(not(feature = "npm"))]
    let _ = (npm, discover_project_npmrc);
    Runtime::with_options(options)
}

fn run(
    prog_name: &str,
    input: Input,
    allow_remote: bool,
    config: &ConfigChoice,
    npm: NpmFlags,
    node_compat: bool,
) -> ExitCode {
    let loaded = match LoadedConfig::from_choice(config) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("{prog_name}: {message}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };
    let resolver = match (&loaded, &input) {
        (Some(loaded), Input::Path(path)) => loaded.resolver_for_entry(Some(path)),
        (Some(loaded), Input::Url(_)) => loaded.resolver_for_entry(None),
        (None, Input::Path(path)) => discover_resolver_for_entry(path),
        (None, Input::Url(_)) => Ok(None),
    };
    let resolver = match resolver {
        Ok(resolver) => resolver,
        Err(message) => {
            eprintln!("{prog_name}: {message}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let mut runtime = match create_runtime(
        allow_remote,
        resolver,
        npm,
        node_compat,
        !matches!(config, ConfigChoice::Disabled),
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{prog_name}: {error}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let result = match &input {
        Input::Path(path) => runtime.run_file(path),
        Input::Url(url) => {
            #[cfg(feature = "remote")]
            {
                runtime.run_url(url)
            }
            #[cfg(not(feature = "remote"))]
            {
                let _ = url;
                eprintln!(
                    "{prog_name}: remote module loading requires `vaeljs-lite` built with the `remote` feature"
                );
                return ExitCode::from(EXIT_STARTUP_FAILURE);
            }
        }
    };
    match result {
        Ok(()) => {
            if let Some(code) = runtime.exit_code() {
                ExitCode::from((code & 0xff) as u8)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("{prog_name}: {error}");
            ExitCode::from(exit_code(&error))
        }
    }
}

/// Cumulative counters printed after every selected test file has run.
#[derive(Default)]
struct TestTotals {
    files: usize,
    total: u32,
    passed: u32,
    failed: u32,
    pending: u32,
    unhandled: u32,
    unsettled: u32,
}

impl TestTotals {
    fn add(&mut self, report: &TestReport) {
        self.files += 1;
        self.total += report.total;
        self.passed += report.passed;
        self.failed += report.failed;
        self.pending += report.pending;
        self.unhandled += report.unhandled;
        self.unsettled += report.unsettled;
    }

    fn all_passed(&self) -> bool {
        self.failed == 0 && self.pending == 0 && self.unhandled == 0 && self.unsettled == 0
    }
}

/// Expands the arguments, then runs each file in a fresh QuickJS runtime.
fn run_tests(
    prog_name: &str,
    patterns: &[String],
    allow_remote: bool,
    config: &ConfigChoice,
    npm: NpmFlags,
    node_compat: bool,
) -> ExitCode {
    let loaded = match LoadedConfig::from_choice(config) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("{prog_name}: {message}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let files = match vaeljs_module_policy::test_glob::expand(patterns) {
        Ok(files) => files,
        Err(message) => return startup_failure(prog_name, &message),
    };

    let mut resolvers = Vec::with_capacity(files.len());
    for file in &files {
        let resolver = match &loaded {
            Some(loaded) => loaded.resolver_for_entry(Some(file)),
            None => discover_resolver_for_entry(file),
        };
        match resolver {
            Ok(resolver) => resolvers.push(resolver),
            Err(message) => {
                eprintln!("{prog_name}: {message}");
                return ExitCode::from(EXIT_STARTUP_FAILURE);
            }
        }
    }

    let mut totals = TestTotals::default();
    let mut failed_files = 0usize;
    for (file, resolver) in files.iter().zip(resolvers) {
        println!("test {}:", file.display());
        let mut runtime = match create_runtime(
            allow_remote,
            resolver,
            npm,
            node_compat,
            !matches!(config, ConfigChoice::Disabled),
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("{prog_name}: {error}");
                return ExitCode::from(EXIT_STARTUP_FAILURE);
            }
        };
        if let Err(error) = runtime.run_file(file) {
            eprintln!("{prog_name}: {}: {error}", file.display());
            failed_files += 1;
            continue;
        }
        if let Some(code) = runtime.exit_code()
            && code != 0
        {
            eprintln!("{prog_name}: {}: exited with code {code}", file.display());
            failed_files += 1;
            continue;
        }
        match runtime.run_tests() {
            Ok(report) => {
                let no_tests = report.total == 0 && report.unhandled == 0;
                totals.add(&report);
                if no_tests {
                    eprintln!("{prog_name}: {}: no tests registered", file.display());
                    failed_files += 1;
                } else if !report.all_passed() {
                    failed_files += 1;
                }
            }
            Err(error) => {
                eprintln!("{prog_name}: {}: {error}", file.display());
                failed_files += 1;
            }
        }
    }

    println!(
        "# {} selected file(s): {} test(s): {} passed, {} failed, {} pending, {} unhandled, {} unsettled; {} file(s) failed",
        files.len(),
        totals.total,
        totals.passed,
        totals.failed,
        totals.pending,
        totals.unhandled,
        totals.unsettled,
        failed_files
    );
    if failed_files > 0 || !totals.all_passed() {
        ExitCode::from(EXIT_RUNTIME_FAILURE)
    } else {
        ExitCode::SUCCESS
    }
}

/// Maps an error to the exit code the V8 `vaeljs` command uses.
fn exit_code(error: &Error) -> u8 {
    match error {
        Error::Module { .. }
        | Error::TypeScript { .. }
        | Error::RemoteTypeScript { .. }
        | Error::UnhandledRejection { .. }
        | Error::NonConvergingJobs { .. }
        | Error::TestRunner(_) => EXIT_RUNTIME_FAILURE,
        Error::Init { .. }
        | Error::Entry { .. }
        | Error::UnsupportedEntry { .. }
        | Error::RemoteDisabled
        | Error::RemoteEntry { .. }
        | Error::RuntimeConsumed => EXIT_STARTUP_FAILURE,
        _ => EXIT_STARTUP_FAILURE,
    }
}

fn run_types(prog_name: &str, profile: Profile, out_dir: Option<&Path>) -> ExitCode {
    match out_dir {
        Some(dir) => match vaeljs_dts::write_types_dir(profile, dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!(
                    "{prog_name}: cannot write types to `{}`: {error}",
                    dir.display()
                );
                ExitCode::from(EXIT_STARTUP_FAILURE)
            }
        },
        None => {
            use std::io::{ErrorKind, Write};
            let bundle = vaeljs_dts::render_bundle(profile);
            let mut stdout = std::io::stdout().lock();
            match stdout
                .write_all(bundle.as_bytes())
                .and_then(|()| stdout.flush())
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) if error.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{prog_name}: cannot write types to stdout: {error}");
                    ExitCode::from(EXIT_STARTUP_FAILURE)
                }
            }
        }
    }
}

fn types_help(prog_name: &str) -> String {
    format!(
        "\
vaeljs-lite {}
Emit TypeScript declarations (.d.ts) for VaelJS globals and `vael:*` built-in modules.

Usage:
  {prog_name} types [options]

Options:
  --profile <lite|v8>  runtime profile to emit (`lite` or `v8`, default `lite`)
  --out-dir <dir>      write the types/ directory layout instead of stdout
  -h, --help           show this help

Examples:
  {prog_name} types > vjs-lite.d.ts
  {prog_name} types --profile v8 > vjs.d.ts
  {prog_name} types --out-dir ./types
",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(feature = "vael-template")]
const VAEL_MODULES_HELP: &str = "  vael:assert, vael:chars, vael:console, vael:crypto, vael:fs, vael:os, vael:path, vael:slices, vael:ssh, vael:strings, vael:template, vael:test, vael:uuid, vael:uuid/v4, vael:uuid/v7";
#[cfg(not(feature = "vael-template"))]
const VAEL_MODULES_HELP: &str = "  vael:assert, vael:chars, vael:console, vael:crypto, vael:fs, vael:os, vael:path, vael:slices, vael:ssh, vael:strings, vael:test, vael:uuid, vael:uuid/v4, vael:uuid/v7";

#[cfg(feature = "node-compat")]
const NODE_MODULES_HELP: &str = "\n  node:buffer, node:child_process, node:crypto, node:fs, node:os, node:path, node:path/posix, node:path/win32, node:process, node:url (unless --no-node-compat)";
#[cfg(not(feature = "node-compat"))]
const NODE_MODULES_HELP: &str = "";

fn help(prog_name: &str) -> String {
    #[cfg(all(feature = "remote", feature = "npm"))]
    let remote_npm_options = "\
  --allow-remote   allow https:, gh:, github:, and jsr: entry and imports, plus
                   npm: network fetches; off by default
  --no-allow-remote
                   force remote loading off, overriding VAELJS_ALLOW_REMOTE
  --no-npm         disable npm: and package.json module resolution
  --frozen-lockfile
                   refuse npm resolutions not pinned in vael.lock
  --offline        refuse network fetches; serve npm packages strictly from CAS";
    #[cfg(all(feature = "remote", not(feature = "npm")))]
    let remote_npm_options = "\
  --allow-remote   allow https:, gh:, github:, and jsr: entry and imports;
                   off by default
  --no-allow-remote
                   force remote loading off, overriding VAELJS_ALLOW_REMOTE";
    #[cfg(not(feature = "remote"))]
    let remote_npm_options = "\
  (remote and npm features disabled at compile time)";

    #[cfg(all(feature = "remote", feature = "npm"))]
    let remote_npm_summary = "\
Remote loading is disabled by default. `--allow-remote` enables https:, gh:,
github:, and jsr:@scope/name[@version][/subpath] entries and imports;
`--no-allow-remote` forces it off. Downloads are cached under `vaeljs/packages`
in the user cache. `npm:` specifiers, `package.json` dependencies/`#imports`,
and `vael.lock` resolve without `node_modules` or install scripts via the
verified CAS cache (offline by default; use `--allow-remote` to fetch missing
packages, `--frozen-lockfile` to require `vael.lock`, `--offline` to forbid
network, or `--no-npm` to disable). Local modules discover `vael.jsonc` upward
for `imports` and `workspaces` unless `--config` or `--no-config` is passed.";
    #[cfg(all(feature = "remote", not(feature = "npm")))]
    let remote_npm_summary = "\
Remote loading is disabled by default. `--allow-remote` enables https:, gh:,
github:, and jsr:@scope/name[@version][/subpath] entries and imports;
`--no-allow-remote` forces it off. Downloads are cached under `vaeljs/packages`
in the user cache. `npm:` is disabled at compile time. Local modules discover
`vael.jsonc` upward for `imports` and `workspaces` unless `--config` or
`--no-config` is passed.";
    #[cfg(not(feature = "remote"))]
    let remote_npm_summary = "\
Local modules discover `vael.jsonc` upward for `imports` and `workspaces`
unless `--config` or `--no-config` is passed. Remote (`https:`, `gh:`, `jsr:`)
and `npm:` features were disabled at compile time.";

    #[cfg(feature = "remote")]
    let usage_remote = format!(
        "  {prog_name} --allow-remote <file-or-url>\n  {prog_name} --no-allow-remote <file>\n"
    );
    #[cfg(not(feature = "remote"))]
    let usage_remote = String::new();

    #[cfg(feature = "remote")]
    let usage_test_remote = format!("  {prog_name} test --allow-remote <file-or-glob>...\n");
    #[cfg(not(feature = "remote"))]
    let usage_test_remote = String::new();

    #[cfg(feature = "remote")]
    let env_section = "\
\nEnvironment:
  VAELJS_ALLOW_REMOTE  1 or true enables remote loading, 0 or false keeps it off
                       (case-insensitive); an explicit flag overrides a valid
                       setting, and an invalid value is rejected at startup\n";
    #[cfg(not(feature = "remote"))]
    let env_section = "";

    format!(
        "\
vaeljs-lite {}
Run an ECMAScript module or erasable TypeScript file on QuickJS, or run test files.

Usage:
  {prog_name} <file>                    run one module
{usage_remote}  {prog_name} --config <path> <file>
  {prog_name} --no-config <file>
  {prog_name} -- <file>
  {prog_name} test <file-or-glob>...    run test files
{usage_test_remote}  {prog_name} types [--profile <lite|v8>] [--out-dir <dir>]
                                       emit TypeScript declarations (.d.ts)

Options:
{remote_npm_options}
  --config <path>   Use an explicit vael.jsonc manifest instead of discovery
  --no-config       Disable vael.jsonc and project .npmrc discovery
  --no-node-compat  Disable node:* built-in compatibility modules
  -h, --help        Print this help (alone or after `test`)
  -V, --version     Print the version
  --                End option parsing, for a file or pattern beginning with `-`
{env_section}
Built-in modules:
{}{}

Modules are ECMAScript modules, and relative imports resolve from the importing
file. .js, .mjs, .ts, and .mts run. TypeScript is stripped, not type checked:
only erasable syntax is accepted (no enum, namespace, or other transforms), and
.tsx, .jsx, .cts, and declaration files are rejected.

{remote_npm_summary}
Unsupported `node:*` imports (such as `node:fs/promises`) are rejected.

`{prog_name} test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
Host timers are not implemented; `test(..., {{ timeout }})` is rejected.

Exit codes:
  0  the module finished, or every test file passed
  1  the module or a test failed, or a test file registered no tests
  2  {prog_name} could not start: bad arguments, an invalid or missing manifest,
     an unreadable or unsupported entry file, or a test file or glob that could
     not be expanded
",
        env!("CARGO_PKG_VERSION"),
        VAEL_MODULES_HELP,
        NODE_MODULES_HELP
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AllowRemote, Command, ConfigChoice, DEFAULT_PROG_NAME, Input, NpmFlags, parse_args,
        program_name_from_arg0,
    };
    #[cfg(feature = "remote")]
    use super::{classify_input, parse_boolean_env};
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;
    use vaeljs_dts::Profile;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_test_subcommand_and_patterns() {
        assert_eq!(
            parse_args(args(&["test", "a.test.js", "b/*.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string(), "b/*.test.js".to_string()],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
    }

    #[test]
    fn test_subcommand_supports_help_and_double_dash() {
        assert_eq!(parse_args(args(&["test", "--help"])), Ok(Command::Help));
        assert_eq!(
            parse_args(args(&["test", "--", "-weird.test.js"])),
            Ok(Command::Test {
                patterns: vec!["-weird.test.js".to_string()],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
    }

    #[test]
    fn test_subcommand_rejects_empty_and_unknown_options() {
        assert_eq!(
            parse_args(args(&["test"])),
            Err("no test files or globs given".to_string())
        );
        assert_eq!(
            parse_args(args(&["test", "--unknown", "a.test.js"])),
            Err("unknown option `--unknown`".to_string())
        );
    }

    #[test]
    fn config_flags_parse_and_reject_conflicts() {
        assert_eq!(
            parse_args(args(&["--config", "custom.jsonc", "main.js"])),
            Ok(Command::Run {
                input: Input::Path(PathBuf::from("main.js")),
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Explicit(PathBuf::from("custom.jsonc")),
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
        assert_eq!(
            parse_args(args(&["test", "--no-config", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Disabled,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
        assert_eq!(
            parse_args(args(&["--no-node-compat", "main.js"])),
            Ok(Command::Run {
                input: Input::Path(PathBuf::from("main.js")),
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: false,
            })
        );
        assert_eq!(
            parse_args(args(&["test", "--no-node-compat", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: false,
            })
        );
        assert_eq!(
            parse_args(args(&["--config", "a.jsonc", "--no-config", "main.js"])),
            Err("`--config` and `--no-config` cannot be used together".to_string())
        );
        assert_eq!(
            parse_args(args(&["--config"])),
            Err("`--config` requires a path".to_string())
        );
    }

    #[test]
    #[cfg(feature = "remote")]
    fn allow_remote_and_url_inputs_parse_cleanly() {
        assert_eq!(
            parse_args(args(&["--allow-remote", "https://example.test/mod.ts"])),
            Ok(Command::Run {
                input: Input::Url("https://example.test/mod.ts".to_string()),
                allow_remote: AllowRemote::Enabled,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
        assert_eq!(
            parse_args(args(&["--allow-remote", "test", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: AllowRemote::Enabled,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
        assert_eq!(
            parse_args(args(&["--allow-remote", "--no-allow-remote", "main.js"])),
            Err("`--allow-remote` and `--no-allow-remote` cannot be used together".to_string())
        );
        assert_eq!(
            classify_input(&OsString::from("jsr:@scope/pkg@1.0.0")),
            Input::Url("jsr:@scope/pkg@1.0.0".to_string())
        );
        assert_eq!(
            classify_input(&OsString::from("gh:owner/repo@v1/mod.ts")),
            Input::Url("gh:owner/repo@v1/mod.ts".to_string())
        );
        assert_eq!(
            classify_input(&OsString::from("github:owner/repo@v1/mod.ts")),
            Input::Url("github:owner/repo@v1/mod.ts".to_string())
        );
        assert_eq!(
            classify_input(&OsString::from("C:\\work\\main.js")),
            Input::Path(PathBuf::from("C:\\work\\main.js"))
        );
        assert!(parse_boolean_env("VAELJS_ALLOW_REMOTE", OsStr::new("1")).unwrap());
        assert!(!parse_boolean_env("VAELJS_ALLOW_REMOTE", OsStr::new("false")).unwrap());
        assert!(parse_boolean_env("VAELJS_ALLOW_REMOTE", OsStr::new("maybe")).is_err());
    }

    #[test]
    #[cfg(feature = "npm")]
    fn npm_flags_parse_for_run_and_test() {
        assert_eq!(
            parse_args(args(&[
                "--no-npm",
                "--frozen-lockfile",
                "--offline",
                "main.js"
            ])),
            Ok(Command::Run {
                input: Input::Path(PathBuf::from("main.js")),
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags {
                    enabled: false,
                    frozen_lockfile: true,
                    offline: true,
                },
                node_compat: true,
            })
        );
        assert_eq!(
            parse_args(args(&[
                "test",
                "--frozen-lockfile",
                "--offline",
                "a.test.js"
            ])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags {
                    enabled: true,
                    frozen_lockfile: true,
                    offline: true,
                },
                node_compat: true,
            })
        );
    }

    #[test]
    fn types_command_parses_defaults_profiles_out_dir_and_help() {
        assert_eq!(
            parse_args(args(&["types"])),
            Ok(Command::Types {
                profile: Profile::Lite,
                out_dir: None,
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--profile", "v8"])),
            Ok(Command::Types {
                profile: Profile::V8,
                out_dir: None,
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--profile=lite", "--out-dir", "types"])),
            Ok(Command::Types {
                profile: Profile::Lite,
                out_dir: Some(PathBuf::from("types")),
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--out-dir=./out", "--profile=v8"])),
            Ok(Command::Types {
                profile: Profile::V8,
                out_dir: Some(PathBuf::from("./out")),
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--help"])),
            Ok(Command::TypesHelp)
        );
        assert_eq!(parse_args(args(&["types", "-h"])), Ok(Command::TypesHelp));
        assert_eq!(
            parse_args(args(&["types", "--profile", "unknown"])),
            Err("invalid profile `unknown`: expected `lite` or `v8`".to_string())
        );
        assert_eq!(
            parse_args(args(&["types", "--out-dir="])),
            Err("`--out-dir` requires a non-empty directory".to_string())
        );
        assert_eq!(
            parse_args(args(&["--no-config", "types"])),
            Err(
                "`types` does not accept `--allow-remote`, `--no-allow-remote`, `--no-node-compat`, `--config`, or `--no-config`"
                    .to_string()
            )
        );
        assert_eq!(
            parse_args(args(&["--no-node-compat", "types"])),
            Err(
                "`types` does not accept `--allow-remote`, `--no-allow-remote`, `--no-node-compat`, `--config`, or `--no-config`"
                    .to_string()
            )
        );
        #[cfg(feature = "npm")]
        assert_eq!(
            parse_args(args(&["--offline", "types"])),
            Err(
                "`types` does not accept `--no-npm`, `--frozen-lockfile`, or `--offline`"
                    .to_string()
            )
        );
    }

    #[test]
    fn program_name_from_arg0_extracts_basename_and_falls_back() {
        assert_eq!(
            program_name_from_arg0(Some(OsStr::new("/usr/local/bin/vjs")), DEFAULT_PROG_NAME),
            "vjs"
        );
        assert_eq!(
            program_name_from_arg0(Some(OsStr::new("./foo/my-vjs")), DEFAULT_PROG_NAME),
            "my-vjs"
        );
        assert_eq!(
            program_name_from_arg0(Some(OsStr::new("vael")), DEFAULT_PROG_NAME),
            "vael"
        );
        assert_eq!(
            program_name_from_arg0(Some(OsStr::new(r"C:\bin\vjs.exe")), DEFAULT_PROG_NAME),
            "vjs"
        );
        assert_eq!(
            program_name_from_arg0(Some(OsStr::new(r"C:\bin\VJS-LITE.EXE")), DEFAULT_PROG_NAME),
            "VJS-LITE"
        );
        assert_eq!(
            program_name_from_arg0(None, DEFAULT_PROG_NAME),
            DEFAULT_PROG_NAME
        );
        for invalid in ["", "/", r"C:\", ".exe", "my vjs", "vjs\n", "vjs\r", "vjs\0"] {
            assert_eq!(
                program_name_from_arg0(Some(OsStr::new(invalid)), DEFAULT_PROG_NAME),
                DEFAULT_PROG_NAME,
                "input: {invalid:?}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                program_name_from_arg0(
                    Some(OsStr::from_bytes(b"/tmp/\xff\xfe")),
                    DEFAULT_PROG_NAME
                ),
                DEFAULT_PROG_NAME
            );
        }
    }
}
