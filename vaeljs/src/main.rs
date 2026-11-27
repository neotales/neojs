//! Command-line entry point for vaeljs.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vaeljs_core::Error;
use vaeljs_core::ImportResolver;
#[cfg(feature = "npm")]
use vaeljs_core::NpmOptions;
use vaeljs_core::RemoteOptions;
use vaeljs_core::Runtime;
use vaeljs_core::RuntimeOptions;
use vaeljs_core::TestReport;
use vaeljs_dts::Profile;
use vaeljs_manifest::{
    DiscoveredManifest, MAX_DISCOVERY_DEPTH, Manifest, Workspace, discover_manifest,
};

/// Default program name shown when `argv[0]` is missing or unusable.
const DEFAULT_PROG_NAME: &str = "vjs";
/// Exit code for a module that failed while loading or running.
const EXIT_RUNTIME_FAILURE: u8 = 1;
/// Exit code for a command vjs could not start.
const EXIT_STARTUP_FAILURE: u8 = 2;

/// Persistent remote-loading setting for both run and test commands.
///
/// This is the only command-line setting: help, version, the `test` subcommand,
/// and input paths are actions rather than persisted preferences, so they have
/// no `VAELJS_` variables.
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
            println!("vjs {}", env!("CARGO_PKG_VERSION"));
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
///
/// `Unspecified` leaves the decision to [`ALLOW_REMOTE_ENV`], which defaults to
/// disabled. `Enabled` and `Disabled` are explicit choices and override a valid
/// environment setting; the setting is still validated so a typo cannot hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowRemote {
    Unspecified,
    Enabled,
    Disabled,
}

impl AllowRemote {
    /// Records one explicit flag, rejecting a contradiction.
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

/// Resolves the effective remote-loading setting.
///
/// A command-line flag wins over a valid environment setting. The environment
/// variable is validated either way, so a malformed persisted setting is a
/// startup failure rather than something a flag silently masks.
fn resolve_allow_remote(choice: AllowRemote) -> Result<bool, String> {
    let configured = match std::env::var_os(ALLOW_REMOTE_ENV) {
        Some(value) => parse_boolean_env(ALLOW_REMOTE_ENV, &value)?,
        None => false,
    };
    Ok(match choice {
        AllowRemote::Unspecified => configured,
        AllowRemote::Enabled => true,
        AllowRemote::Disabled => false,
    })
}

/// Parses an explicit boolean environment setting.
///
/// Only `1`, `0`, `true`, and `false` are accepted, case-insensitively for the
/// words. Anything else, including an empty or non-Unicode value, is a startup
/// error. The message names the variable and the accepted values but never
/// echoes the value, so a misplaced secret cannot leak through a typo.
fn parse_boolean_env(name: &str, value: &std::ffi::OsStr) -> Result<bool, String> {
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
                allow_remote.choose(AllowRemote::Enabled)?;
                index += 1;
            }
            Some("--no-allow-remote") => {
                allow_remote.choose(AllowRemote::Disabled)?;
                index += 1;
            }
            Some("--no-npm") => {
                #[cfg(not(feature = "npm"))]
                return Err("`--no-npm` requires `vaeljs` built with the `npm` feature".to_string());
                #[cfg(feature = "npm")]
                {
                    npm.enabled = false;
                    index += 1;
                }
            }
            Some("--frozen-lockfile") => {
                #[cfg(not(feature = "npm"))]
                return Err(
                    "`--frozen-lockfile` requires `vaeljs` built with the `npm` feature"
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
                    "`--offline` requires `vaeljs` built with the `npm` feature".to_string()
                );
                #[cfg(feature = "npm")]
                {
                    npm.offline = true;
                    index += 1;
                }
            }
            Some("--no-node-compat") => {
                node_compat = false;
                index += 1;
            }
            Some("--no-config") => {
                config.set_disabled()?;
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

/// Parses `vjs test <file-or-glob>...`.
///
/// Every argument is a pattern. `--allow-remote`, `--no-allow-remote`,
/// `--no-npm`, `--frozen-lockfile`, `--offline`, `--no-node-compat`,
/// `--config <path>`, and `--no-config` may appear before or after `test` and
/// apply to each selected file's fresh runtime. `--` ends option parsing so a
/// pattern beginning with `-`, including one named `--allow-remote`, can still
/// be named. The command requires at least one pattern; discovery defaults are
/// deliberately absent.
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
            allow_remote.choose(AllowRemote::Enabled)?;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--no-allow-remote" {
            allow_remote.choose(AllowRemote::Disabled)?;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--no-npm" {
            #[cfg(not(feature = "npm"))]
            return Err("`--no-npm` requires `vaeljs` built with the `npm` feature".to_string());
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
                "`--frozen-lockfile` requires `vaeljs` built with the `npm` feature".to_string(),
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
            return Err("`--offline` requires `vaeljs` built with the `npm` feature".to_string());
            #[cfg(feature = "npm")]
            {
                npm.offline = true;
                index += 1;
                continue;
            }
        }
        if !after_double_dash && value == "--no-node-compat" {
            node_compat = false;
            index += 1;
            continue;
        }
        if !after_double_dash && value == "--no-config" {
            config.set_disabled()?;
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

/// Parses `vjs types [--profile <v8|lite>] [--out-dir <dir>]`.
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
                .ok_or_else(|| "`--profile` requires a value (`v8` or `lite`)".to_string())?;
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
        profile: profile.unwrap_or(Profile::V8),
        out_dir,
    })
}

fn parse_profile(value: &str) -> Result<Profile, String> {
    match value {
        "v8" => Ok(Profile::V8),
        "lite" => Ok(Profile::Lite),
        _ => Err(format!(
            "invalid profile `{value}`: expected `v8` or `lite`"
        )),
    }
}

/// Decides whether an input names a local path or a remote URL.
///
/// `http:`, `https:`, `gh:`, and `github:` are treated as URLs. A Windows drive
/// path such as `C:\work\main.js` parses as a one-letter URL scheme, and every
/// other string is a path, so the existing path handling is unchanged. A `gh:`
/// or `github:` input still fails unless remote loading is enabled; the runtime
/// performs the grammar check and the opt-in check.
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
        if https || http || github {
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
                let resolver =
                    ImportResolver::from_workspace(workspace, active.map(|member| member.name()))
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
        Input::Url(url) => runtime.run_url(url),
    };
    match result {
        Ok(()) => match runtime.exit_code() {
            Some(code) => ExitCode::from((code & 0xff) as u8),
            None => ExitCode::SUCCESS,
        },
        Err(error) => {
            eprintln!("{prog_name}: {error}");
            ExitCode::from(exit_code(&error))
        }
    }
}

/// Creates a runtime, with remote and JSR loading only when explicitly enabled,
/// an optional `vael.jsonc` [`ImportResolver`], npm policy flags, and optional
/// `node:*` compat.
///
/// Each call gets its own default [`RemoteOptions`], so a fresh runtime also
/// gets a fresh remote request, module, and byte budget. The default package
/// cache is shared per user profile, as for a normal run. The JSR loader uses
/// the same client, limits, and cache for `jsr:` metadata and files.
fn create_runtime(
    allow_remote: bool,
    import_resolver: Option<ImportResolver>,
    npm: NpmFlags,
    node_compat: bool,
    discover_project_npmrc: bool,
) -> Result<Runtime, Error> {
    let mut options = RuntimeOptions::new().with_node_compat(node_compat);
    if allow_remote {
        options = options.with_remote(RemoteOptions::new());
    }
    if let Some(resolver) = import_resolver {
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

fn exit_code(error: &Error) -> u8 {
    match error {
        Error::Core(_)
        | Error::TypeScript { .. }
        | Error::RemoteTypeScript { .. }
        | Error::TestRunner(_) => EXIT_RUNTIME_FAILURE,
        _ => EXIT_STARTUP_FAILURE,
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

/// Expands the arguments, then runs each file in a fresh runtime.
///
/// A file that fails to load, fails a test, registers no tests, leaves a test
/// pending, or leaves an unhandled rejection makes the command exit 1. A later
/// file still runs, and each file gets a new isolate so globals, module state,
/// and the test queue cannot leak between files. With `allow_remote`, every
/// fresh runtime can also load `https:` modules, with its own default remote
/// limits and the shared user-profile package cache.
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
vaeljs {}
Emit TypeScript declarations (.d.ts) for VaelJS globals and `vael:*` built-in modules.

Usage:
  {prog_name} types [options]

Options:
  --profile <v8|lite>  runtime profile to emit (`v8` or `lite`, default `v8`)
  --out-dir <dir>      write the types/ directory layout instead of stdout
  -h, --help           show this help

Examples:
  {prog_name} types > vjs.d.ts
  {prog_name} types --profile lite > vjs-lite.d.ts
  {prog_name} types --out-dir ./types
",
        env!("CARGO_PKG_VERSION")
    )
}

/// The default `--help` text, shared by `<prog_name> --help` and `<prog_name> test --help`.
///
/// This stays short on purpose: the built-in module surfaces, remote limits,
/// unsupported features, and test-runner limits are documented in the README,
/// which the last line points at. The options and every remote-flag placement
/// must stay in step with the current dev parser.
fn help(prog_name: &str) -> String {
    #[cfg(feature = "npm")]
    let npm_options = "\
  --allow-remote   allow https:, gh:, and github: entry and imports, and
                   jsr:/npm: network fetches; off by default
  --no-allow-remote
                   force remote loading off, overriding VAELJS_ALLOW_REMOTE
  --no-npm         disable npm: and package.json module resolution
  --frozen-lockfile
                   refuse npm resolutions not pinned in vael.lock
  --offline        refuse network fetches; serve npm packages strictly from CAS";
    #[cfg(not(feature = "npm"))]
    let npm_options = "\
  --allow-remote   allow https:, gh:, and github: entry and imports, and
                   jsr: imports; off by default
  --no-allow-remote
                   force remote loading off, overriding VAELJS_ALLOW_REMOTE";

    #[cfg(feature = "npm")]
    let npm_summary = "\
user cache. JSR metadata and files use the same HTTPS limits and cache. `npm:`
specifiers, `package.json` dependencies/`#imports`, and `vael.lock` resolve
without `node_modules` or install scripts via the verified CAS cache
(offline by default; use `--allow-remote` to fetch missing packages,
`--frozen-lockfile` to require `vael.lock`, `--offline` to forbid network, or
`--no-npm` to disable). Local modules discover";
    #[cfg(not(feature = "npm"))]
    let npm_summary = "\
user cache. JSR metadata and files use the same HTTPS limits and cache. npm:
is not implemented (npm support was disabled at compile time). Local modules discover";

    format!(
        "\
vaeljs {}
Run an ECMAScript module or erasable TypeScript file, or run test files.

Usage:
  {prog_name} <file>                    run one module
  {prog_name} --allow-remote <file-or-url>
  {prog_name} --no-allow-remote <file>
  {prog_name} test <file-or-glob>...    run test files
  {prog_name} test --allow-remote <file-or-glob>...
  {prog_name} test --no-allow-remote <file-or-glob>...
  {prog_name} --allow-remote test <file-or-glob>...
  {prog_name} types [--profile <v8|lite>] [--out-dir <dir>]
                                  emit TypeScript declarations (.d.ts)

Options:
{npm_options}
  --no-node-compat disable built-in `node:*` compatibility modules
  --config <path>  use an explicit vael.jsonc manifest instead of discovery
  --no-config      disable automatic vael.jsonc manifest discovery
  -h, --help       show this help; alone, after `test`, or after `--allow-remote`
  -V, --version    show the version; alone, or after `--allow-remote`
  --               end option parsing, for a file or pattern beginning with `-`

Environment:
  VAELJS_ALLOW_REMOTE  1 or true enables remote loading, 0 or false keeps it off
                      (case-insensitive); an explicit flag overrides a valid
                      setting, and an invalid value is rejected at startup

Modules are ECMAScript modules, and relative imports resolve from the importing
file. .js, .mjs, .ts, and .mts run. TypeScript is stripped, not type checked:
only erasable syntax is accepted (no enum, namespace, or other transforms), and
.tsx, .jsx, .cts, and declaration files are rejected.

The age, assert, chars, console, cron, crypto, db, fs, os, path,
slices, ssh, stdio, strings, test, and uuid built-in modules are implemented,
including the uuid/v4 and uuid/v7 subpaths. Import one by its canonical `vael:`
specifier, such as `vael:age` and `vael:test`; a bare name is not a built-in
module. `Vael.getBuiltInModule` takes the same canonical specifier, such as
`Vael.getBuiltInModule(\"vael:test\")`; the README lists every specifier.
Supported `node:*` compatibility modules (`node:path`, `node:os`, `node:process`,
`node:fs`, `node:fs/promises`, `node:child_process`, `node:buffer`, `node:url`,
`node:crypto`) are available by canonical `node:` specifier unless
`--no-node-compat` is passed.

Remote loading is disabled by default. `--allow-remote` enables an https:,
gh:, or github: entry, absolute https: imports, and
jsr:@scope/name[@version][/subpath] imports from the public JSR registry;
`--no-allow-remote` forces it off. Either flag may appear before the module, or
before or after `test`. A gh:/github: specifier becomes a
raw.githubusercontent.com https: URL. It is not SSRF protection, not a host
allowlist, and not a sandbox. Downloads are cached under `vaeljs/packages` in the
{npm_summary}
`vael.jsonc` upward for `imports` and `workspaces` unless `--config` or
`--no-config` is passed.

`{prog_name} test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
Remote loading follows the flag or setting for every selected file.
Unsupported `node:` specifiers (such as `node:test`) are rejected.

Exit codes:
  0  the module finished, or every test file passed
  1  the module or a test failed, or a test file registered no tests
  2  {prog_name} could not start: bad arguments, an invalid or missing manifest,
     an unreadable or unsupported entry file, or a test file or glob that could
     not be expanded

Docs: https://github.com/neotales/vaeljs#readme
",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::ALLOW_REMOTE_ENV;
    use super::AllowRemote;
    use super::Command;
    use super::ConfigChoice;
    use super::DEFAULT_PROG_NAME;
    use super::NpmFlags;
    use super::parse_args;
    use super::parse_boolean_env;
    use super::program_name_from_arg0;
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use vaeljs_dts::Profile;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn test_command_carries_allow_remote_from_before_the_subcommand() {
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
    }

    #[test]
    fn test_command_carries_allow_remote_from_after_the_subcommand() {
        assert_eq!(
            parse_args(args(&["test", "--allow-remote", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: AllowRemote::Enabled,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
    }

    #[test]
    fn test_command_carries_no_allow_remote_from_either_side() {
        for values in [
            ["--no-allow-remote", "test", "a.test.js"].as_slice(),
            ["test", "--no-allow-remote", "a.test.js"].as_slice(),
        ] {
            assert_eq!(
                parse_args(args(values)),
                Ok(Command::Test {
                    patterns: vec!["a.test.js".to_string()],
                    allow_remote: AllowRemote::Disabled,
                    config: ConfigChoice::Auto,
                    npm: NpmFlags::default(),
                    node_compat: true,
                })
            );
        }
    }

    #[test]
    fn test_command_leaves_remote_decision_to_the_environment_without_a_flag() {
        assert_eq!(
            parse_args(args(&["test", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
    }

    #[test]
    fn test_command_double_dash_keeps_remote_flags_as_patterns() {
        assert_eq!(
            parse_args(args(&["test", "--", "--allow-remote", "--no-allow-remote"])),
            Ok(Command::Test {
                patterns: vec![
                    "--allow-remote".to_string(),
                    "--no-allow-remote".to_string()
                ],
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
    }

    #[test]
    fn test_command_rejects_contradictory_remote_flags() {
        assert_eq!(
            parse_args(args(&[
                "--allow-remote",
                "test",
                "--no-allow-remote",
                "a.test.js"
            ])),
            Err("`--allow-remote` and `--no-allow-remote` cannot be used together".to_string())
        );
    }

    #[test]
    fn test_command_rejects_unknown_options() {
        assert_eq!(
            parse_args(args(&["test", "--unknown", "a.test.js"])),
            Err("unknown option `--unknown`".to_string())
        );
    }

    #[test]
    fn run_command_carries_the_remote_choice() {
        assert_eq!(
            parse_args(args(&["--no-allow-remote", "main.js"])),
            Ok(Command::Run {
                input: super::Input::Path("main.js".into()),
                allow_remote: AllowRemote::Disabled,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
    }

    #[cfg(feature = "npm")]
    #[test]
    fn npm_flags_parse_for_run_and_test_and_reject_types() {
        assert_eq!(
            parse_args(args(&[
                "--no-npm",
                "--frozen-lockfile",
                "--offline",
                "main.js"
            ])),
            Ok(Command::Run {
                input: super::Input::Path("main.js".into()),
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
        assert_eq!(
            parse_args(args(&["--offline", "types"])),
            Err(
                "`types` does not accept `--no-npm`, `--frozen-lockfile`, or `--offline`"
                    .to_string()
            )
        );
    }

    #[cfg(not(feature = "npm"))]
    #[test]
    fn npm_flags_reject_when_compiled_without_npm_feature() {
        for flag in ["--no-npm", "--frozen-lockfile", "--offline"] {
            assert_eq!(
                parse_args(args(&[flag, "main.js"])),
                Err(format!(
                    "`{flag}` requires `vaeljs` built with the `npm` feature"
                ))
            );
            assert_eq!(
                parse_args(args(&["test", flag, "a.test.js"])),
                Err(format!(
                    "`{flag}` requires `vaeljs` built with the `npm` feature"
                ))
            );
        }
    }

    #[test]
    fn no_node_compat_parses_for_run_and_both_test_positions() {
        assert_eq!(
            parse_args(args(&["--no-node-compat", "main.js"])),
            Ok(Command::Run {
                input: super::Input::Path("main.js".into()),
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Auto,
                npm: NpmFlags::default(),
                node_compat: false,
            })
        );
        for values in [
            ["--no-node-compat", "test", "a.test.js"].as_slice(),
            ["test", "--no-node-compat", "a.test.js"].as_slice(),
        ] {
            assert_eq!(
                parse_args(args(values)),
                Ok(Command::Test {
                    patterns: vec!["a.test.js".to_string()],
                    allow_remote: AllowRemote::Unspecified,
                    config: ConfigChoice::Auto,
                    npm: NpmFlags::default(),
                    node_compat: false,
                })
            );
        }
        assert_eq!(
            parse_args(args(&["--no-node-compat", "types"])),
            Err(
                "`types` does not accept `--allow-remote`, `--no-allow-remote`, `--no-node-compat`, `--config`, or `--no-config`"
                    .to_string()
            )
        );
    }

    #[test]
    fn config_flags_parse_and_reject_conflicts() {
        assert_eq!(
            parse_args(args(&["--config", "custom.jsonc", "main.js"])),
            Ok(Command::Run {
                input: super::Input::Path("main.js".into()),
                allow_remote: AllowRemote::Unspecified,
                config: ConfigChoice::Explicit(PathBuf::from("custom.jsonc")),
                npm: NpmFlags::default(),
                node_compat: true,
            })
        );
        assert_eq!(
            parse_args(args(&["--config=custom.jsonc", "test", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
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
            parse_args(args(&["--config", "a.jsonc", "--no-config", "main.js"])),
            Err("`--config` and `--no-config` cannot be used together".to_string())
        );
        assert_eq!(
            parse_args(args(&[
                "--no-config",
                "test",
                "--config",
                "a.jsonc",
                "a.test.js"
            ])),
            Err("`--config` and `--no-config` cannot be used together".to_string())
        );
        assert_eq!(
            parse_args(args(&["--config"])),
            Err("`--config` requires a path".to_string())
        );
        assert_eq!(
            parse_args(args(&["--config=", "main.js"])),
            Err("`--config` requires a non-empty path".to_string())
        );
    }

    #[test]
    fn boolean_environment_setting_accepts_only_explicit_values() {
        for (value, expected) in [
            ("1", true),
            ("0", false),
            ("true", true),
            ("TRUE", true),
            ("False", false),
        ] {
            assert_eq!(
                parse_boolean_env(ALLOW_REMOTE_ENV, OsStr::new(value)),
                Ok(expected),
                "value {value:?}"
            );
        }
        for value in ["", "yes", "on", "2", " true", "true "] {
            assert_eq!(
                parse_boolean_env(ALLOW_REMOTE_ENV, OsStr::new(value)),
                Err(
                    "invalid VAELJS_ALLOW_REMOTE: expected `1`, `0`, `true`, or `false`"
                        .to_string()
                ),
                "value {value:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn boolean_environment_setting_rejects_non_unicode_values() {
        use std::os::unix::ffi::OsStringExt;

        let value = OsString::from_vec(vec![0xff]);
        assert_eq!(
            parse_boolean_env(ALLOW_REMOTE_ENV, &value),
            Err("invalid VAELJS_ALLOW_REMOTE: expected `1`, `0`, `true`, or `false`".to_string())
        );
    }

    #[test]
    fn types_command_parses_defaults_profiles_out_dir_and_help() {
        assert_eq!(
            parse_args(args(&["types"])),
            Ok(Command::Types {
                profile: Profile::V8,
                out_dir: None,
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--profile", "lite"])),
            Ok(Command::Types {
                profile: Profile::Lite,
                out_dir: None,
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--profile=v8", "--out-dir", "types"])),
            Ok(Command::Types {
                profile: Profile::V8,
                out_dir: Some(PathBuf::from("types")),
            })
        );
        assert_eq!(
            parse_args(args(&["types", "--out-dir=./out", "--profile=lite"])),
            Ok(Command::Types {
                profile: Profile::Lite,
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
            Err("invalid profile `unknown`: expected `v8` or `lite`".to_string())
        );
        assert_eq!(
            parse_args(args(&["types", "--out-dir="])),
            Err("`--out-dir` requires a non-empty directory".to_string())
        );
        assert_eq!(
            parse_args(args(&["--allow-remote", "types"])),
            Err(
                "`types` does not accept `--allow-remote`, `--no-allow-remote`, `--no-node-compat`, `--config`, or `--no-config`"
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
            program_name_from_arg0(Some(OsStr::new(r"C:\bin\VJS.EXE")), DEFAULT_PROG_NAME),
            "VJS"
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
