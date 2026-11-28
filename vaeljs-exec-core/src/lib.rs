//! Engine-neutral process execution, `PATH` lookup, and advisory file locking core for `vael:exec`.
//!
//! This crate powers both the V8 runtime (`vaeljs-core`) and the QuickJS runtime
//! (`vaeljs-lite-core`) with zero Tokio, zero Hyper, and zero JavaScript engine
//! dependencies.
//!
//! # Security and resource contract
//!
//! * **Argv-only execution**: [`CommandSpec`] never invokes a shell (`sh -c` or
//!   `cmd.exe /c`). Program names, arguments, working directories, and environment
//!   keys/values are validated against NUL bytes and invalid structure before spawn.
//! * **Safe Windows `PATH` resolution**: Bare program names on Windows are resolved
//!   against `PATH` + `PATHEXT` while skipping empty or relative `PATH` entries
//!   and never implicitly searching the current working directory.
//! * **Bounded concurrent stdio**: [`exec_sync`] drains `stdout` and `stderr` on
//!   concurrent bounded threads up to [`CommandSpec::get_max_output_bytes`] (default
//!   16 MiB, hard cap 64 MiB), failing with [`ExecErrorCode::OutputTooLarge`] on
//!   overflow and enforcing `timeout` via `kill_signal` -> `SIGKILL` escalation.
//! * **Safe advisory file locking**: [`FileLock::acquire`] rejects symbolic-link
//!   targets (`O_NOFOLLOW` on Unix) and uses safe `std::fs::File` advisory locks.
//! * **Minimal audited `unsafe`**: Crate root uses `#![deny(unsafe_code)]` with
//!   `unsafe` strictly confined to a single Unix `libc::kill(pid, signo)` wrapper
//!   that rejects `pid <= 0`.
//!
//! # Example
//!
//! ```
//! use vaeljs_exec_core::{CommandSpec, ExecErrorCode, FileLock, LockOptions};
//!
//! let err = CommandSpec::new("").unwrap_err();
//! assert_eq!(err.code(), ExecErrorCode::InvalidArgument);
//!
//! let dir = tempfile::tempdir().unwrap();
//! let lock = FileLock::acquire(&dir.path().join("app.lock"), &LockOptions::default()).unwrap();
//! lock.unlock().unwrap();
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod lock;
mod process;
mod types;
mod which;

pub use error::{ExecError, ExecErrorCode};
pub use lock::{FileLock, LockMode, LockOptions};
pub use process::{SpawnedChild, exec_sync, run_sync, spawn};
pub use types::{
    CommandOutput, CommandSpec, CommandStatus, DEFAULT_KILL_GRACE, DEFAULT_MAX_OUTPUT_BYTES,
    MAX_OUTPUT_BYTES, Signal, StdioMode,
};
pub use which::{env_var, expand_env_path, is_executable_file, which, which_all};

/// The canonical `vael:exec` import specifier and module registry key.
pub const SPECIFIER: &str = "vael:exec";

/// The named exports of the `vael:exec` JavaScript module, in property order.
pub const EXPORTS: &[&str] = &["exec", "run", "spawn", "which", "whichAll", "lockFile"];
