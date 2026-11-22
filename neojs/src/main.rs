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
/// Only `http:` and `https:` are treated as URLs. A Windows drive path such as
/// `C:\work\main.js` parses as a one-letter URL scheme, and every other string
/// is a path, so the existing path handling is unchanged.
fn classify_input(input: &OsString) -> Input {
    if let Some(text) = input.to_str() {
        let https = text
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"));
        let http = text
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"));
        if https || http {
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

fn help() -> String {
    format!(
        "\
neojs {}
Run an ECMAScript module or erasable TypeScript file on V8, or run test files.

Usage:
  neojs <file>
  neojs --allow-remote <file-or-https-url>
  neojs --no-allow-remote <file>
  neojs test <file-or-glob>...
  neojs test --allow-remote <file-or-glob>...
  neojs test --no-allow-remote <file-or-glob>...
  neojs --help
  neojs --version

The file is read as an ECMAScript module, and relative imports resolve against
it. ECMAScript modules, local .js, .mjs, .ts, and .mts imports, console output,
top-level await, and the age, assert, chars, console, crypto, os, path, slices,
ssh, stdio, strings, test, and uuid built-in modules are implemented. TypeScript is run
by erasing type syntax: it is not type checked, constructs that need a transform
(such as enum and namespace) are rejected, and .tsx, .jsx, .cts, and declaration
files are not supported. The built-ins are available as
Neo.getBuiltInModule(\"age\"), Neo.getBuiltInModule(\"assert\"),
Neo.getBuiltInModule(\"chars\"), Neo.getBuiltInModule(\"console\"),
Neo.getBuiltInModule(\"crypto\"), Neo.getBuiltInModule(\"os\"),
Neo.getBuiltInModule(\"path\"), Neo.getBuiltInModule(\"slices\"),
Neo.getBuiltInModule(\"ssh\"), Neo.getBuiltInModule(\"stdio\"),
Neo.getBuiltInModule(\"strings\"), Neo.getBuiltInModule(\"test\"),
Neo.getBuiltInModule(\"uuid\"), Neo.getBuiltInModule(\"uuid/v4\"), and
Neo.getBuiltInModule(\"uuid/v7\"), and as
the import specifiers \"age\", \"neo:age\", \"assert\", \"neo:assert\",
\"chars\", \"neo:chars\", \"console\", \"neo:console\", \"crypto\",
\"neo:crypto\", \"os\", \"neo:os\", \"path\", \"neo:path\", \"slices\",
\"neo:slices\", \"ssh\", \"neo:ssh\", \"stdio\", \"neo:stdio\", \"strings\",
\"neo:strings\", \"test\", \"neo:test\", \"uuid\", \"neo:uuid\", \"uuid/v4\",
\"neo:uuid/v4\", \"uuid/v7\", and \"neo:uuid/v7\".
Remote modules are disabled by default. --allow-remote enables an https: entry
and explicit https: imports for a run, and for every selected test file when
given to the test command before or after `test`; relative imports in a remote
module resolve against that module's URL. Set NEOJS_ALLOW_REMOTE to 1 or true
to make that the default for every run and test command, or to 0 or false to
keep it off; the words are case-insensitive, and any other value is a startup
error. An explicit --allow-remote or --no-allow-remote overrides a valid
setting; an invalid value is rejected even when one is given. The setting is
security-relevant: it enables remote code execution for every invocation in
that environment. Only https: is accepted, remote
modules cannot import file: URLs, redirects must stay credential-free https and
within the limit, and no credentials or proxy settings are read. After a redirect the final URL is the
module's identity and its relative-import base; the module format follows that
final URL. Downloads are cached in $XDG_CACHE_HOME/neojs/packages, or
$HOME/.cache/neojs/packages when XDG_CACHE_HOME is unset, with body length and
SHA-256 checks and HTTP revalidation. Per-module body, aggregate byte, and
module-count limits bound a remote graph; the deadline is per module, not a
wall-clock bound on the program. This is not SSRF or permission protection and
it is not a sandbox: remote code runs with the same authority as local code.
The stdio module writes to this process's stdout and stderr and offers bounded
in-memory byte streams; it adds no filesystem, process, environment, or stdin
access. The os module reports the Rust build target's platform and arch; it
reads no environment or operating-system file and installs no os global. The
ssh module requires an explicit host-key pin or known-hosts document (there is
no insecure default), adds outbound TCP and synchronous
command execution, authenticates with a password, an in-memory key, or an
explicit Unix agent opt-in, and reads no files or environment except
SSH_AUTH_SOCK in that agent mode. It never forwards an agent. The test module
registers sync and async tests, treats a rejected Promise, a generator object,
a pending test, a per-test timeout, an unhandled rejection, and event-loop work
that remains after a bounded settle window as failures, and is not Node's
node:test. node: modules, including node:test, are rejected while no explicit
Node-module opt-in exists. .cjs files, other neo: imports, remote imports
without --allow-remote, module type attributes, and permissions are not.

The uuid module generates RFC 9562 version 4 and version 7 UUIDs as canonical
lowercase text from the operating system CSPRNG, with no insecure fallback.
Version 7 embeds a millisecond timestamp but promises no monotonic order.

The test command expands every argument as an existing file or a shell-style
glob. Globs are case-sensitive, * and ? do not cross a path separator, and a
leading dot is matched only literally. The selected files are sorted by
canonical path and deduplicated, so overlapping arguments run once. A missing
literal, a malformed glob, or a glob that selects no regular file is an error;
there is no default discovery pattern. Each file gets a fresh runtime, so
global state and the test queue do not leak between files, and a failed file
does not stop later files. A file that registers no tests fails.
--allow-remote and --no-allow-remote may be given before or after `test` and
select whether every selected file may import https: modules; each file's fresh
runtime gets its own default remote limits, while the user-profile package cache
is shared. An explicit flag overrides NEOJS_ALLOW_REMOTE. `--` ends option
parsing, so a pattern named --allow-remote or --no-allow-remote can still be
selected. The runner waits up to about 100 ms
of event-loop turns for detached timers, async ops, imports, and rejection
events; if work remains, the verdict is unsettled and fails rather than passing
on partial evidence. Test files run with this process's full filesystem
authority; there is no sandbox, watch mode, parallelism, filtering, retry,
subtests, mocks, snapshots, or Node compatibility claim.

Exit codes:
  0  the module finished, or every test file ran with no failures
  1  the module failed to compile, resolve imports, link, or run; or a test
     failed, was pending, left an unhandled rejection or unsettled event-loop
     work, or registered no tests
  2  neojs could not start (bad arguments, unreadable or unsupported entry file,
     or a test file/glob that could not be expanded)
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
