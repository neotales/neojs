//! Command-line entry point for NeoJS Lite on QuickJS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neojs_lite_core::{Error, Runtime, TestReport};

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
        Command::Run(path) => run(&path),
        Command::Test { patterns } => run_tests(&patterns),
    }
}

fn startup_failure(message: &str) -> ExitCode {
    eprintln!("neojs-lite: {message}");
    eprintln!("Try 'neojs-lite --help'.");
    ExitCode::from(EXIT_STARTUP_FAILURE)
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run(PathBuf),
    Test { patterns: Vec<String> },
    Help,
    Version,
}

fn parse_args(args: Vec<OsString>) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Err("no input file".to_string());
    };

    if let Some(option) = first.to_str() {
        match option {
            "-h" | "--help" if args.len() == 1 => return Ok(Command::Help),
            "-V" | "--version" if args.len() == 1 => return Ok(Command::Version),
            "--" => {
                if args.len() != 2 {
                    return Err("expected exactly one input file after `--`".to_string());
                }
                return Ok(Command::Run(PathBuf::from(&args[1])));
            }
            "test" => return parse_test_args(&args[1..]),
            _ => {}
        }
    }

    if args.len() > 1 {
        return Err(format!("expected one input file, got {}", args.len()));
    }
    if first.to_str().is_some_and(|arg| arg.starts_with('-')) {
        return Err(format!("unknown option `{}`", first.to_string_lossy()));
    }
    Ok(Command::Run(PathBuf::from(first)))
}

/// Parses `neojs-lite test <file-or-glob>...`.
///
/// `--` ends option parsing so a pattern beginning with `-` can still be named.
/// Requires at least one pattern; discovery defaults are deliberately absent.
fn parse_test_args(args: &[OsString]) -> Result<Command, String> {
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
        if !after_double_dash && value.starts_with('-') {
            return Err(format!("unknown option `{value}`"));
        }
        patterns.push(value.to_string());
    }
    if patterns.is_empty() {
        return Err("no test files or globs given".to_string());
    }
    Ok(Command::Test { patterns })
}

fn run(path: &Path) -> ExitCode {
    let mut runtime = match Runtime::new() {
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
fn run_tests(patterns: &[String]) -> ExitCode {
    let files = match neojs_module_policy::test_glob::expand(patterns) {
        Ok(files) => files,
        Err(message) => return startup_failure(&message),
    };

    let mut totals = TestTotals::default();
    let mut failed_files = 0usize;
    for file in &files {
        println!("test {}:", file.display());
        let mut runtime = match Runtime::new() {
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
Run an ECMAScript module on QuickJS, or run test files.

Usage:
  neojs-lite <file>                    run one module
  neojs-lite -- <file>
  neojs-lite test <file-or-glob>...    run test files

Options:
  -h, --help       Print this help (alone or after `test`)
  -V, --version    Print the version
  --               End option parsing, for a file or pattern beginning with `-`

Built-in modules:
  neo:assert, neo:chars, neo:os, neo:strings, neo:test, neo:uuid, neo:uuid/v4, neo:uuid/v7

`neojs-lite test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
TypeScript and host timers are not implemented; `test(..., {{ timeout }})` is
rejected.

Exit codes:
  0  the module finished, or every test file passed
  1  the module or a test failed, or a test file registered no tests
  2  neojs-lite could not start: bad arguments, an unreadable or unsupported
     entry file, or a test file or glob that could not be expanded
",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_args};
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_test_subcommand_and_patterns() {
        assert_eq!(
            parse_args(args(&["test", "a.test.js", "b/*.test.js"])),
            Ok(Command::Test {
                patterns: vec!["a.test.js".to_string(), "b/*.test.js".to_string()],
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
}
