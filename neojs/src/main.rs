//! Command-line entry point for neojs.

mod test_command;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use neojs_core::Error;
use neojs_core::RemoteOptions;
use neojs_core::Runtime;
use neojs_core::TestReport;

/// Exit code for a module that failed while loading or running.
const EXIT_RUNTIME_FAILURE: u8 = 1;
/// Exit code for a command neojs could not start.
const EXIT_STARTUP_FAILURE: u8 = 2;

/// Persistent remote-loading setting for both run and test commands.
///
/// This is the only command-line setting: help, version, the `test` subcommand,
/// and input paths are actions rather than persisted preferences, so they have
/// no `NEOJS_` variables.
const ALLOW_REMOTE_ENV: &str = "NEOJS_ALLOW_REMOTE";

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
            println!("neojs {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Run {
            input,
            allow_remote,
        } => match resolve_allow_remote(allow_remote) {
            Ok(allow_remote) => run(input, allow_remote),
            Err(message) => startup_failure(&message),
        },
        Command::Test {
            patterns,
            allow_remote,
        } => match resolve_allow_remote(allow_remote) {
            Ok(allow_remote) => run_tests(&patterns, allow_remote),
            Err(message) => startup_failure(&message),
        },
    }
}

fn startup_failure(message: &str) -> ExitCode {
    eprintln!("neojs: {message}");
    eprintln!("Try 'neojs --help'.");
    ExitCode::from(EXIT_STARTUP_FAILURE)
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run {
        input: Input,
        allow_remote: AllowRemote,
    },
    Test {
        patterns: Vec<String>,
        allow_remote: AllowRemote,
    },
    Help,
    Version,
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
                });
            }
            "test" => return parse_test_args(&rest[1..], allow_remote),
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
    })
}

/// Parses `neojs test <file-or-glob>...`.
///
/// Every argument is a pattern. `--allow-remote` and `--no-allow-remote` may
/// appear before or after `test` and select remote loading for each selected
/// file's fresh runtime. `--` ends option parsing so a pattern beginning with
/// `-`, including one named `--allow-remote`, can still be named. The command
/// requires at least one pattern; discovery defaults are deliberately absent.
fn parse_test_args(args: &[OsString], mut allow_remote: AllowRemote) -> Result<Command, String> {
    if args.len() == 1 && matches!(args[0].to_str(), Some("-h" | "--help")) {
        return Ok(Command::Help);
    }
    let mut patterns = Vec::new();
    let mut after_double_dash = false;
    for arg in args {
        let Some(value) = arg.to_str() else {
            return Err("test file or glob names must be valid UTF-8".to_string());
        };
        if !after_double_dash && value == "--" {
            after_double_dash = true;
            continue;
        }
        if !after_double_dash && value == "--allow-remote" {
            allow_remote.choose(AllowRemote::Enabled)?;
            continue;
        }
        if !after_double_dash && value == "--no-allow-remote" {
            allow_remote.choose(AllowRemote::Disabled)?;
            continue;
        }
        if !after_double_dash && value.starts_with('-') {
            return Err(format!("unknown option `{value}`"));
        }
        patterns.push(value.to_string());
    }
    if patterns.is_empty() {
        return Err("no test files or globs given".to_string());
    }
    Ok(Command::Test {
        patterns,
        allow_remote,
    })
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

fn run(input: Input, allow_remote: bool) -> ExitCode {
    let mut runtime = match create_runtime(allow_remote) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("neojs: {error}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let result = match &input {
        Input::Path(path) => runtime.run_file(path),
        Input::Url(url) => runtime.run_url(url),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neojs: {error}");
            ExitCode::from(exit_code(&error))
        }
    }
}

/// Creates a runtime, with remote loading only when explicitly enabled.
///
/// Each call gets its own default [`RemoteOptions`], so a fresh runtime also
/// gets a fresh remote request, module, and byte budget. The default package
/// cache is shared per user profile, as for a normal run.
fn create_runtime(allow_remote: bool) -> Result<Runtime, Error> {
    if allow_remote {
        Runtime::with_remote(RemoteOptions::new())
    } else {
        Runtime::new()
    }
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
fn run_tests(patterns: &[String], allow_remote: bool) -> ExitCode {
    let files = match test_command::expand(patterns) {
        Ok(files) => files,
        Err(message) => {
            eprintln!("neojs: {message}");
            eprintln!("Try 'neojs --help'.");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    let mut totals = TestTotals::default();
    let mut failed_files = 0usize;
    for file in &files {
        println!("test {}:", file.display());
        let mut runtime = match create_runtime(allow_remote) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("neojs: {error}");
                return ExitCode::from(EXIT_STARTUP_FAILURE);
            }
        };
        if let Err(error) = runtime.run_file(file) {
            eprintln!("neojs: {}: {error}", file.display());
            failed_files += 1;
            continue;
        }
        match runtime.run_tests() {
            Ok(report) => {
                let no_tests = report.total == 0 && report.unhandled == 0;
                totals.add(&report);
                if no_tests {
                    eprintln!("neojs: {}: no tests registered", file.display());
                    failed_files += 1;
                } else if !report.all_passed() {
                    failed_files += 1;
                }
            }
            Err(error) => {
                eprintln!("neojs: {}: {error}", file.display());
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

/// The default `--help` text, shared by `neojs --help` and `neojs test --help`.
///
/// This stays short on purpose: the built-in module surfaces, remote limits,
/// unsupported features, and test-runner limits are documented in the README,
/// which the last line points at. The options and every remote-flag placement
/// must stay in step with the current dev parser.
fn help() -> String {
    format!(
        "\
neojs {}
Run an ECMAScript module or erasable TypeScript file, or run test files.

Usage:
  neojs <file>                    run one module
  neojs --allow-remote <file-or-url>
  neojs --no-allow-remote <file>
  neojs test <file-or-glob>...    run test files
  neojs test --allow-remote <file-or-glob>...
  neojs test --no-allow-remote <file-or-glob>...
  neojs --allow-remote test <file-or-glob>...

Options:
  --allow-remote   allow https:, gh:, and github: entry and imports; off by default
  --no-allow-remote
                   force remote loading off, overriding NEOJS_ALLOW_REMOTE
  -h, --help       show this help; alone, after `test`, or after `--allow-remote`
  -V, --version    show the version; alone, or after `--allow-remote`
  --               end option parsing, for a file or pattern beginning with `-`

Environment:
  NEOJS_ALLOW_REMOTE  1 or true enables remote loading, 0 or false keeps it off
                      (case-insensitive); an explicit flag overrides a valid
                      setting, and an invalid value is rejected at startup

Modules are ECMAScript modules, and relative imports resolve from the importing
file. .js, .mjs, .ts, and .mts run. TypeScript is stripped, not type checked:
only erasable syntax is accepted (no enum, namespace, or other transforms), and
.tsx, .jsx, .cts, and declaration files are rejected.

The age, assert, chars, console, cron, crypto, os, path,
slices, ssh, stdio, strings, test, and uuid built-in modules are implemented,
including the uuid/v4 and uuid/v7 subpaths. Import one by its canonical `neo:`
specifier, such as `neo:age` and `neo:test`; a bare name is not a built-in
module. `Neo.getBuiltInModule` takes the same canonical specifier, such as
`Neo.getBuiltInModule(\"neo:test\")`; the README lists every specifier.

Remote loading is disabled by default. `--allow-remote` enables an https:,
gh:, or github: entry and those imports; `--no-allow-remote` forces it off.
Either flag may appear before the module, or before or after `test`. A
gh:/github: specifier becomes a raw.githubusercontent.com https: URL. It is
not SSRF protection, not a host allowlist, and not a sandbox. Downloads are
cached under `neojs/packages` in the user cache.

`neojs test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
Remote loading follows the flag or setting for every selected file.
Node modules (`node:`) are rejected; there is no Node-module opt-in.

Exit codes:
  0  the module finished, or every test file passed
  1  the module or a test failed, or a test file registered no tests
  2  neojs could not start: bad arguments, an unreadable or unsupported entry
     file, or a test file or glob that could not be expanded

Docs: https://github.com/neotales/neojs#readme
",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::ALLOW_REMOTE_ENV;
    use super::AllowRemote;
    use super::Command;
    use super::parse_args;
    use super::parse_boolean_env;
    use std::ffi::OsStr;
    use std::ffi::OsString;

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
            })
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
                    "invalid NEOJS_ALLOW_REMOTE: expected `1`, `0`, `true`, or `false`".to_string()
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
            Err("invalid NEOJS_ALLOW_REMOTE: expected `1`, `0`, `true`, or `false`".to_string())
        );
    }
}
