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

fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("neojs: {message}");
            eprintln!("Try 'neojs --help'.");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
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
        } => run(input, allow_remote),
        Command::Test {
            patterns,
            allow_remote,
        } => run_tests(&patterns, allow_remote),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run {
        input: Input,
        allow_remote: bool,
    },
    Test {
        patterns: Vec<String>,
        allow_remote: bool,
    },
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
enum Input {
    Path(PathBuf),
    Url(String),
}

fn parse_args(args: Vec<OsString>) -> Result<Command, String> {
    let mut allow_remote = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.to_str() {
            Some("--allow-remote") => {
                allow_remote = true;
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
/// Every argument is a pattern. `--allow-remote` may appear before or after
/// `test` and enables remote loading for each selected file's fresh runtime. `--`
/// ends option parsing so a pattern beginning with `-`, including one named
/// `--allow-remote`, can still be named. The command requires at least one
/// pattern; discovery defaults are deliberately absent.
fn parse_test_args(args: &[OsString], mut allow_remote: bool) -> Result<Command, String> {
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
            allow_remote = true;
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
  neojs test <file-or-glob>...
  neojs test --allow-remote <file-or-glob>...
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
module resolve against that module's URL. Only https: is accepted, remote
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
does not stop later files. A file that registers no tests fails. --allow-remote
may be given before or after `test` and enables https: imports in every selected
file; each file's fresh runtime gets its own default remote limits, while the
user-profile package cache is shared. `--` ends option parsing, so a pattern
named --allow-remote can still be selected. The runner waits up to about 100 ms
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
    use super::Command;
    use super::parse_args;
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
                allow_remote: true,
            })
        );
    }

    #[test]
    fn test_command_carries_allow_remote_from_after_the_subcommand() {
        assert_eq!(
            parse_args(args(&["test", "--allow-remote", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: true,
            })
        );
    }

    #[test]
    fn test_command_keeps_remote_disabled_without_the_flag() {
        assert_eq!(
            parse_args(args(&["test", "a.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string()],
                allow_remote: false,
            })
        );
    }

    #[test]
    fn test_command_double_dash_keeps_allow_remote_as_a_pattern() {
        assert_eq!(
            parse_args(args(&["test", "--", "--allow-remote"])),
            Ok(Command::Test {
                patterns: vec!["--allow-remote".to_string()],
                allow_remote: false,
            })
        );
    }

    #[test]
    fn test_command_rejects_unknown_options() {
        assert_eq!(
            parse_args(args(&["test", "--unknown", "a.test.js"])),
            Err("unknown option `--unknown`".to_string())
        );
    }
}
