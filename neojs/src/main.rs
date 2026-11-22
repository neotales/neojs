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

/// The default `--help` text, shared by `neojs --help` and `neojs test --help`.
///
/// This stays short on purpose: the built-in module surfaces, remote limits,
/// unsupported features, and test-runner limits are documented in the README,
/// which the last line points at. The options and both `--allow-remote`
/// placements must stay in step with the current dev parser.
fn help() -> String {
    format!(
        "\
neojs {}
Run an ECMAScript module or erasable TypeScript file, or run test files.

Usage:
  neojs <file>                    run one module
  neojs --allow-remote <file-or-https-url>
  neojs test <file-or-glob>...    run test files
  neojs test --allow-remote <file-or-glob>...
  neojs --allow-remote test <file-or-glob>...

Options:
  --allow-remote   allow https: entry and imports; off by default
  -h, --help       show this help; alone, or after `test`
  -V, --version    show the version; must be the only argument
  --               end option parsing, for a file or pattern beginning with `-`

Modules are ECMAScript modules, and relative imports resolve from the importing
file. .js, .mjs, .ts, and .mts run. TypeScript is stripped, not type checked:
only erasable syntax is accepted (no enum, namespace, or other transforms), and
.tsx, .jsx, .cts, and declaration files are rejected.

The age, assert, chars, console, crypto, os, path, slices,
ssh, stdio, strings, test, and uuid built-in modules are implemented, including
the uuid/v4 and uuid/v7 subpaths. Import one by bare name or with the `neo:`
prefix, or use `Neo.getBuiltInModule(\"test\")` and
`Neo.getBuiltInModule(\"uuid\")`; the README lists every specifier.

Remote loading is disabled by default. `--allow-remote` enables an https:
entry and absolute https: imports; it may appear before the module, or before or
after `test`. It is not SSRF protection and not a sandbox, and downloads are
cached under `neojs/packages` in the user cache.

`neojs test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
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
