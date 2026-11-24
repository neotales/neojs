//! Command-line entry point for NeoJS Lite on QuickJS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neojs_lite_core::{Error, ImportResolver, Runtime, TestReport};
use neojs_manifest::{
    DiscoveredManifest, MAX_DISCOVERY_DEPTH, Manifest, Workspace, discover_manifest,
};

/// Exit code for a module or test that failed while loading or running.
const EXIT_RUNTIME_FAILURE: u8 = 1;
/// Exit code for a command neojs-lite could not start.
const EXIT_STARTUP_FAILURE: u8 = 2;

fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(command) => command,
        Err(message) => return startup_failure(&message),
    };

    match command {
        Command::Help => {
            print!("{}", help());
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("neojs-lite {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Run { path, config } => run(&path, &config),
        Command::Test { patterns, config } => run_tests(&patterns, &config),
    }
}

fn startup_failure(message: &str) -> ExitCode {
    eprintln!("neojs-lite: {message}");
    eprintln!("Try 'neojs-lite --help'.");
    ExitCode::from(EXIT_STARTUP_FAILURE)
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run {
        path: PathBuf,
        config: ConfigChoice,
    },
    Test {
        patterns: Vec<String>,
        config: ConfigChoice,
    },
    Help,
    Version,
}

/// The `neo.jsonc` manifest selection made on the command line.
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

fn parse_args(args: Vec<OsString>) -> Result<Command, String> {
    let mut config = ConfigChoice::Auto;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.to_str() {
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
                    path: PathBuf::from(&rest[1]),
                    config,
                });
            }
            "test" => return parse_test_args(&rest[1..], config),
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
        path: PathBuf::from(first),
        config,
    })
}

/// Parses `neojs-lite test <file-or-glob>...`.
///
/// `--config <path>` and `--no-config` may appear before or after `test`.
/// `--` ends option parsing so a pattern beginning with `-` can still be named.
/// Requires at least one pattern; discovery defaults are deliberately absent.
fn parse_test_args(args: &[OsString], mut config: ConfigChoice) -> Result<Command, String> {
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
    Ok(Command::Test { patterns, config })
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

    fn resolver_for_entry(&self, entry_path: &Path) -> Result<Option<ImportResolver>, String> {
        match self {
            Self::None => Ok(None),
            Self::Standalone(resolver) => Ok(Some(resolver.clone())),
            Self::Workspace(workspace) => {
                let active = if std::fs::symlink_metadata(entry_path).is_ok() {
                    workspace.member_for_path(entry_path).map_err(|err| {
                        format!(
                            "cannot resolve `{}` in workspace: {err}",
                            entry_path.display()
                        )
                    })?
                } else {
                    None
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

fn create_runtime(resolver: Option<ImportResolver>) -> Result<Runtime, Error> {
    match resolver {
        Some(resolver) => Runtime::with_import_resolver(resolver),
        None => Runtime::new(),
    }
}

fn run(path: &Path, config: &ConfigChoice) -> ExitCode {
    let loaded = match LoadedConfig::from_choice(config) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("neojs-lite: {message}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };
    let resolver = match &loaded {
        Some(loaded) => loaded.resolver_for_entry(path),
        None => discover_resolver_for_entry(path),
    };
    let resolver = match resolver {
        Ok(resolver) => resolver,
        Err(message) => {
            eprintln!("neojs-lite: {message}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let mut runtime = match create_runtime(resolver) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("neojs-lite: {error}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    match runtime.run_file(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neojs-lite: {error}");
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
///
/// A file that fails to load, fails a test, registers no tests, leaves a test
/// pending, leaves an unhandled rejection, or leaves unsettled job work makes
/// the command exit 1. Later files still run in fresh isolated runtimes.
fn run_tests(patterns: &[String], config: &ConfigChoice) -> ExitCode {
    let loaded = match LoadedConfig::from_choice(config) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("neojs-lite: {message}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let files = match neojs_module_policy::test_glob::expand(patterns) {
        Ok(files) => files,
        Err(message) => return startup_failure(&message),
    };

    let mut resolvers = Vec::with_capacity(files.len());
    for file in &files {
        let resolver = match &loaded {
            Some(loaded) => loaded.resolver_for_entry(file),
            None => discover_resolver_for_entry(file),
        };
        match resolver {
            Ok(resolver) => resolvers.push(resolver),
            Err(message) => {
                eprintln!("neojs-lite: {message}");
                return ExitCode::from(EXIT_STARTUP_FAILURE);
            }
        }
    }

    let mut totals = TestTotals::default();
    let mut failed_files = 0usize;
    for (file, resolver) in files.iter().zip(resolvers) {
        println!("test {}:", file.display());
        let mut runtime = match create_runtime(resolver) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("neojs-lite: {error}");
                return ExitCode::from(EXIT_STARTUP_FAILURE);
            }
        };
        if let Err(error) = runtime.run_file(file) {
            eprintln!("neojs-lite: {}: {error}", file.display());
            failed_files += 1;
            continue;
        }
        match runtime.run_tests() {
            Ok(report) => {
                let no_tests = report.total == 0 && report.unhandled == 0;
                totals.add(&report);
                if no_tests {
                    eprintln!("neojs-lite: {}: no tests registered", file.display());
                    failed_files += 1;
                } else if !report.all_passed() {
                    failed_files += 1;
                }
            }
            Err(error) => {
                eprintln!("neojs-lite: {}: {error}", file.display());
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

/// Maps an error to the exit code the V8 `neojs` command uses.
///
/// A module that started and failed is a runtime failure. A preflight problem
/// (unreadable or unsupported entry, or failure to create the runtime) is a
/// startup failure. Keeping this mapping aligned makes shared fixtures behave
/// the same across the two commands even though the prefixes differ.
fn exit_code(error: &Error) -> u8 {
    match error {
        Error::Module { .. }
        | Error::TypeScript { .. }
        | Error::UnhandledRejection { .. }
        | Error::NonConvergingJobs { .. }
        | Error::TestRunner(_) => EXIT_RUNTIME_FAILURE,
        Error::Init { .. } | Error::Entry { .. } | Error::UnsupportedEntry { .. } => {
            EXIT_STARTUP_FAILURE
        }
        Error::RuntimeConsumed => EXIT_STARTUP_FAILURE,
        _ => EXIT_STARTUP_FAILURE,
    }
}

fn help() -> String {
    format!(
        "\
neojs-lite {}
Run an ECMAScript module or erasable TypeScript file on QuickJS, or run test files.

Usage:
  neojs-lite <file>                    run one module
  neojs-lite --config <path> <file>
  neojs-lite --no-config <file>
  neojs-lite -- <file>
  neojs-lite test <file-or-glob>...    run test files

Options:
  --config <path>  Use an explicit neo.jsonc manifest instead of discovery
  --no-config      Disable automatic neo.jsonc manifest discovery
  -h, --help       Print this help (alone or after `test`)
  -V, --version    Print the version
  --               End option parsing, for a file or pattern beginning with `-`

Built-in modules:
  neo:assert, neo:chars, neo:fs, neo:os, neo:strings, neo:test, neo:uuid, neo:uuid/v4, neo:uuid/v7

Modules are ECMAScript modules, and relative imports resolve from the importing
file. .js, .mjs, .ts, and .mts run. TypeScript is stripped, not type checked:
only erasable syntax is accepted (no enum, namespace, or other transforms), and
.tsx, .jsx, .cts, and declaration files are rejected.

Local modules discover `neo.jsonc` upward for `imports` and `workspaces`
unless `--config` or `--no-config` is passed. Remote (`https:`, `gh:`, `jsr:`,
`npm:`) and `node:` imports are not supported in `neojs-lite`.

`neojs-lite test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
Host timers are not implemented; `test(..., {{ timeout }})` is rejected.

Exit codes:
  0  the module finished, or every test file passed
  1  the module or a test failed, or a test file registered no tests
  2  neojs-lite could not start: bad arguments, an invalid or missing manifest,
     an unreadable or unsupported entry file, or a test file or glob that could
     not be expanded
",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::{Command, ConfigChoice, parse_args};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_test_subcommand_and_patterns() {
        assert_eq!(
            parse_args(args(&["test", "a.test.js", "b/*.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string(), "b/*.test.js".to_string()],
                config: ConfigChoice::Auto,
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
                config: ConfigChoice::Auto,
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
                path: PathBuf::from("main.js"),
                config: ConfigChoice::Explicit(PathBuf::from("custom.jsonc")),
            })
        );
        assert_eq!(
            parse_args(args(&["test", "--no-config", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                config: ConfigChoice::Disabled,
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
}
