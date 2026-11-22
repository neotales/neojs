//! Command-line entry point for neojs.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use neojs_core::Error;
use neojs_core::RemoteOptions;
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
        Command::Run {
            input,
            allow_remote,
        } => run(input, allow_remote),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run { input: Input, allow_remote: bool },
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
    let runtime = if allow_remote {
        Runtime::with_remote(RemoteOptions::new())
    } else {
        Runtime::new()
    };
    let mut runtime = match runtime {
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

fn exit_code(error: &Error) -> u8 {
    match error {
        Error::Core(_) | Error::TypeScript { .. } | Error::RemoteTypeScript { .. } => {
            EXIT_RUNTIME_FAILURE
        }
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
  neojs --allow-remote <file-or-https-url>
  neojs --help
  neojs --version

The file is read as an ECMAScript module, and relative imports resolve against
it. ECMAScript modules, local .js, .mjs, .ts, and .mts imports, console output,
top-level await, and the age, assert, chars, console, crypto, ssh, and stdio
built-in modules are implemented. TypeScript is run by erasing type syntax: it
is not type checked, constructs that need a transform (such as enum and
namespace) are rejected, and .tsx, .jsx, .cts, and declaration files are not
supported. The built-ins are available as Neo.getBuiltInModule(\"age\"),
Neo.getBuiltInModule(\"assert\"), Neo.getBuiltInModule(\"chars\"),
Neo.getBuiltInModule(\"console\"), Neo.getBuiltInModule(\"crypto\"),
Neo.getBuiltInModule(\"ssh\"), and Neo.getBuiltInModule(\"stdio\"), and as the
import specifiers \"age\", \"neo:age\", \"assert\", \"neo:assert\", \"chars\",
\"neo:chars\", \"console\", \"neo:console\", \"crypto\", \"neo:crypto\", \"ssh\",
\"neo:ssh\", \"stdio\", and \"neo:stdio\".
Remote modules are disabled by default. --allow-remote enables an https: entry
and explicit https: imports; relative imports in a remote module resolve against
that module's URL. Only https: is accepted, remote modules cannot import file:
URLs, redirects must stay credential-free https and within the limit, and no
credentials or proxy settings are read. After a redirect the final URL is the
module's identity and its relative-import base. Downloads are cached in
$XDG_CACHE_HOME/neojs/packages, or $HOME/.cache/neojs/packages when
XDG_CACHE_HOME is unset, and revalidated with HTTP validators. This is not SSRF
or permission protection and it is not a sandbox: remote code runs with the same
authority as local code.
The stdio module writes to this process's stdout and stderr and offers bounded
in-memory byte streams; it adds no filesystem, process, environment, or stdin
access. The ssh module requires an explicit host-key pin or known-hosts
document (there is no insecure default), adds outbound TCP and synchronous
command execution, authenticates with a password, an in-memory key, or an
explicit Unix agent opt-in, and reads no files or environment except
SSH_AUTH_SOCK in that agent mode. It never forwards an agent. .cjs files, other
neo: and node: imports, remote imports without --allow-remote, module type
attributes, and permissions are not.

Exit codes:
  0  the module finished
  1  the module failed to compile, resolve imports, link, or run
  2  neojs could not start (bad arguments, unreadable or unsupported entry file)
",
        env!("CARGO_PKG_VERSION")
    )
}
