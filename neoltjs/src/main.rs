//! Command-line entry point for NeoLTJS on QuickJS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neoltjs_core::{Error, Runtime};

/// Exit code for a module that failed while loading or running.
const EXIT_RUNTIME_FAILURE: u8 = 1;
/// Exit code for a command neoltjs could not start.
const EXIT_STARTUP_FAILURE: u8 = 2;

fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("neoltjs: {message}");
            eprintln!("Try 'neoltjs --help'.");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    match command {
        Command::Help => {
            print!("{}", help());
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("neoltjs {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Run(path) => run(&path),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run(PathBuf),
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

fn run(path: &Path) -> ExitCode {
    let mut runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("neoltjs: {error}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    match runtime.run_file(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoltjs: {error}");
            ExitCode::from(exit_code(&error))
        }
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
        | Error::NonConvergingJobs { .. } => EXIT_RUNTIME_FAILURE,
        Error::Init { .. } | Error::Entry { .. } | Error::UnsupportedEntry { .. } => {
            EXIT_STARTUP_FAILURE
        }
        Error::RuntimeConsumed => EXIT_STARTUP_FAILURE,
        _ => EXIT_STARTUP_FAILURE,
    }
}

fn help() -> &'static str {
    "\
Run an ECMAScript module on QuickJS.

Usage:
  neoltjs <file>
  neoltjs -- <file>

Options:
  -h, --help       Print this help
  -V, --version    Print the version

The QuickJS runtime executes .js, .mjs, and extensionless ECMAScript modules.
TypeScript and host asynchronous work are not implemented yet.
"
}
