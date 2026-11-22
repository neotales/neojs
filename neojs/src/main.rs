//! Command-line entry point for neojs.

use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use neojs_core::Error;
use neojs_core::Runtime;

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
            eprintln!("neojs: {error}");
            return ExitCode::from(EXIT_STARTUP_FAILURE);
        }
    };

    match runtime.run_file(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neojs: {error}");
            ExitCode::from(exit_code(&error))
        }
    }
}

fn exit_code(error: &Error) -> u8 {
    match error {
        Error::Core(_) | Error::TypeScript { .. } => EXIT_RUNTIME_FAILURE,
        _ => EXIT_STARTUP_FAILURE,
    }
}

fn help() -> String {
    format!(
        "\
neojs {}
Run an ECMAScript module or erasable TypeScript file on V8.

Usage:
  neojs <file>
  neojs --help
  neojs --version

The file is read as an ECMAScript module, and relative imports resolve against
it. ECMAScript modules, local .js, .mjs, .ts, and .mts imports, console output,
top-level await, and the age, assert, chars, console, crypto, os, path, slices,
ssh, and stdio built-in modules are implemented. TypeScript is run by erasing
type syntax: it is not type checked, constructs that need a transform (such as
enum and namespace) are rejected, and .tsx, .jsx, .cts, and declaration files
are not supported. The built-ins are available as Neo.getBuiltInModule(\"age\"),
Neo.getBuiltInModule(\"assert\"), Neo.getBuiltInModule(\"chars\"),
Neo.getBuiltInModule(\"console\"), Neo.getBuiltInModule(\"crypto\"),
Neo.getBuiltInModule(\"os\"), Neo.getBuiltInModule(\"path\"),
Neo.getBuiltInModule(\"slices\"), Neo.getBuiltInModule(\"ssh\"),
Neo.getBuiltInModule(\"stdio\"), and Neo.getBuiltInModule(\"strings\"), and as
the import specifiers \"age\", \"neo:age\", \"assert\", \"neo:assert\",
\"chars\", \"neo:chars\", \"console\", \"neo:console\", \"crypto\",
\"neo:crypto\", \"os\", \"neo:os\", \"path\", \"neo:path\", \"slices\",
\"neo:slices\", \"ssh\", \"neo:ssh\", \"stdio\", \"neo:stdio\", \"strings\",
and \"neo:strings\".
The stdio module writes to this process's stdout and stderr and offers bounded
in-memory byte streams; it adds no filesystem, process, environment, or stdin
access. The os module reports the Rust build target's platform and arch; it
reads no environment or operating-system file and installs no os global. The
ssh module requires an explicit host-key pin or known-hosts document (there is
no insecure default), adds outbound TCP and synchronous
command execution, authenticates with a password, an in-memory key, or an
explicit Unix agent opt-in, and reads no files or environment except
SSH_AUTH_SOCK in that agent mode. It never forwards an agent. .cjs files, other
neo: and node: imports, remote imports, module type attributes, and permissions
are not.

Exit codes:
  0  the module finished
  1  the module failed to compile, resolve imports, link, or run
  2  neojs could not start (bad arguments, unreadable or unsupported entry file)
",
        env!("CARGO_PKG_VERSION")
    )
}
