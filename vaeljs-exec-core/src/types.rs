//! Configuration, builder, signal, and result types for `vael:exec`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::ExecError;

/// Default maximum bytes captured per output stream (`16 MiB`).
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Hard upper bound accepted for [`CommandSpec::max_output_bytes`] (`64 MiB`).
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// Default grace period after sending [`CommandSpec`] `kill_signal` before
/// escalating to [`Signal::SigKill`] (`2 s`).
pub const DEFAULT_KILL_GRACE: Duration = Duration::from_secs(2);

/// Explicit process termination/control signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Signal {
    /// Interrupt (`SIGINT`).
    SigInt,
    /// Graceful termination request (`SIGTERM`).
    SigTerm,
    /// Unconditional termination (`SIGKILL`).
    SigKill,
    /// Hangup (`SIGHUP`).
    SigHup,
    /// Quit (`SIGQUIT`).
    SigQuit,
    /// User-defined signal 1 (`SIGUSR1`).
    SigUsr1,
    /// User-defined signal 2 (`SIGUSR2`).
    SigUsr2,
}

impl Signal {
    /// Parses a signal name (case-insensitive, with or without `"SIG"` prefix).
    ///
    /// Rejects empty or unrecognized signal names with [`crate::ExecErrorCode::InvalidArgument`].
    pub fn parse(name: &str) -> Result<Self, ExecError> {
        if name.is_empty() {
            return Err(ExecError::InvalidArgument {
                reason: "signal name must not be empty",
            });
        }
        let upper = name.to_ascii_uppercase();
        match upper.as_str() {
            "SIGINT" | "INT" => Ok(Self::SigInt),
            "SIGTERM" | "TERM" => Ok(Self::SigTerm),
            "SIGKILL" | "KILL" => Ok(Self::SigKill),
            "SIGHUP" | "HUP" => Ok(Self::SigHup),
            "SIGQUIT" | "QUIT" => Ok(Self::SigQuit),
            "SIGUSR1" | "USR1" => Ok(Self::SigUsr1),
            "SIGUSR2" | "USR2" => Ok(Self::SigUsr2),
            _ => Err(ExecError::InvalidArgument {
                reason: "unrecognized signal name",
            }),
        }
    }

    /// Returns the canonical uppercase POSIX signal name (e.g. `"SIGTERM"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SigInt => "SIGINT",
            Self::SigTerm => "SIGTERM",
            Self::SigKill => "SIGKILL",
            Self::SigHup => "SIGHUP",
            Self::SigQuit => "SIGQUIT",
            Self::SigUsr1 => "SIGUSR1",
            Self::SigUsr2 => "SIGUSR2",
        }
    }

    #[cfg(unix)]
    pub(crate) const fn to_posix(self) -> libc::c_int {
        match self {
            Self::SigInt => libc::SIGINT,
            Self::SigTerm => libc::SIGTERM,
            Self::SigKill => libc::SIGKILL,
            Self::SigHup => libc::SIGHUP,
            Self::SigQuit => libc::SIGQUIT,
            Self::SigUsr1 => libc::SIGUSR1,
            Self::SigUsr2 => libc::SIGUSR2,
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Standard stream wiring mode for [`CommandSpec`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StdioMode {
    /// Inherit the parent process's standard stream descriptor.
    Inherit,
    /// Connect an OS pipe between parent and child.
    Piped,
    /// Redirect to the platform null device (`/dev/null` or `NUL`).
    Null,
    /// Feed fixed bytes into standard input and close EOF (`stdin` only).
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EnvOp {
    Set(String, String),
    Remove(String),
}

/// Argv-only command specification with fluent builder methods.
///
/// Never invokes a shell (`sh -c` or `cmd.exe /c`). All arguments, paths, and
/// environment entries are validated against NUL bytes and invalid structure
/// before spawning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) clear_env: bool,
    pub(crate) env_ops: Vec<EnvOp>,
    pub(crate) stdin: Option<StdioMode>,
    pub(crate) stdout: Option<StdioMode>,
    pub(crate) stderr: Option<StdioMode>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) kill_signal: Signal,
    pub(crate) kill_grace: Duration,
    pub(crate) max_output_bytes: usize,
}

impl CommandSpec {
    /// Creates a new [`CommandSpec`] for `program` without shell interpretation.
    ///
    /// Rejects empty strings and strings containing NUL (`\0`) bytes with
    /// [`crate::ExecErrorCode::InvalidArgument`].
    pub fn new(program: impl Into<String>) -> Result<Self, ExecError> {
        let program = program.into();
        if program.is_empty() {
            return Err(ExecError::InvalidArgument {
                reason: "program name must not be empty",
            });
        }
        if program.contains('\0') {
            return Err(ExecError::InvalidArgument {
                reason: "program name must not contain a NUL byte",
            });
        }
        Ok(Self {
            program,
            args: Vec::new(),
            cwd: None,
            clear_env: false,
            env_ops: Vec::new(),
            stdin: None,
            stdout: None,
            stderr: None,
            timeout: None,
            kill_signal: Signal::SigTerm,
            kill_grace: DEFAULT_KILL_GRACE,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        })
    }

    /// Appends a single literal argument (never shell-expanded).
    ///
    /// Rejects arguments containing NUL bytes.
    pub fn arg(mut self, arg: impl Into<String>) -> Result<Self, ExecError> {
        let arg = arg.into();
        if arg.contains('\0') {
            return Err(ExecError::InvalidArgument {
                reason: "process argument must not contain a NUL byte",
            });
        }
        self.args.push(arg);
        Ok(self)
    }

    /// Appends multiple literal arguments in order (never shell-expanded).
    ///
    /// Rejects any argument containing a NUL byte.
    pub fn args<I, S>(mut self, args: I) -> Result<Self, ExecError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for item in args {
            self = self.arg(item)?;
        }
        Ok(self)
    }

    /// Sets the child working directory.
    ///
    /// Rejects empty paths and paths containing NUL bytes.
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Result<Self, ExecError> {
        let path = path.into();
        validate_path_not_empty_or_nul(&path, "working directory")?;
        self.cwd = Some(path);
        Ok(self)
    }

    /// Sets or overrides an environment variable on the child process.
    ///
    /// Rejects empty keys, keys containing `'='` or NUL, and values containing NUL.
    pub fn env(
        mut self,
        key: impl Into<String>,
        val: impl Into<String>,
    ) -> Result<Self, ExecError> {
        let key = key.into();
        let val = val.into();
        validate_env_key(&key)?;
        if val.contains('\0') {
            return Err(ExecError::InvalidArgument {
                reason: "environment variable value must not contain a NUL byte",
            });
        }
        self.env_ops.push(EnvOp::Set(key, val));
        Ok(self)
    }

    /// Sets multiple environment variables on the child process.
    pub fn envs<I, K, V>(mut self, envs: I) -> Result<Self, ExecError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in envs {
            self = self.env(k, v)?;
        }
        Ok(self)
    }

    /// Removes an inherited or previously set environment variable.
    ///
    /// Rejects empty keys and keys containing `'='` or NUL.
    pub fn env_remove(mut self, key: impl Into<String>) -> Result<Self, ExecError> {
        let key = key.into();
        validate_env_key(&key)?;
        self.env_ops.push(EnvOp::Remove(key));
        Ok(self)
    }

    /// Clears all inherited and previously configured environment variables.
    #[must_use]
    pub fn env_clear(mut self) -> Self {
        self.clear_env = true;
        self.env_ops.clear();
        self
    }

    /// Configures how standard input is wired for the child process.
    #[must_use]
    pub fn stdin(mut self, mode: StdioMode) -> Self {
        self.stdin = Some(mode);
        self
    }

    /// Configures standard input to receive `bytes` and then close at EOF.
    #[must_use]
    pub fn stdin_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(StdioMode::Bytes(bytes.into()));
        self
    }

    /// Configures standard output (`Inherit`, `Piped`, or `Null`).
    ///
    /// Rejects [`StdioMode::Bytes`], which is only valid for `stdin`.
    pub fn stdout(mut self, mode: StdioMode) -> Result<Self, ExecError> {
        if matches!(mode, StdioMode::Bytes(_)) {
            return Err(ExecError::InvalidArgument {
                reason: "StdioMode::Bytes is only valid for stdin",
            });
        }
        self.stdout = Some(mode);
        Ok(self)
    }

    /// Configures standard error (`Inherit`, `Piped`, or `Null`).
    ///
    /// Rejects [`StdioMode::Bytes`], which is only valid for `stdin`.
    pub fn stderr(mut self, mode: StdioMode) -> Result<Self, ExecError> {
        if matches!(mode, StdioMode::Bytes(_)) {
            return Err(ExecError::InvalidArgument {
                reason: "StdioMode::Bytes is only valid for stdin",
            });
        }
        self.stderr = Some(mode);
        Ok(self)
    }

    /// Sets a finite, non-zero execution timeout.
    ///
    /// Rejects `Duration::ZERO` and durations that overflow the monotonic clock.
    pub fn timeout(mut self, duration: Duration) -> Result<Self, ExecError> {
        validate_positive_duration(duration, "timeout")?;
        self.timeout = Some(duration);
        Ok(self)
    }

    /// Sets the initial signal sent when `timeout` expires (default [`Signal::SigTerm`]).
    #[must_use]
    pub fn kill_signal(mut self, signal: Signal) -> Self {
        self.kill_signal = signal;
        self
    }

    /// Sets the grace period after `kill_signal` before escalating to [`Signal::SigKill`]
    /// (default [`DEFAULT_KILL_GRACE`], 2 seconds).
    ///
    /// A `Duration::ZERO` value escalates immediately without waiting. Durations that
    /// overflow the monotonic clock are rejected.
    pub fn kill_grace(mut self, grace: Duration) -> Result<Self, ExecError> {
        validate_finite_duration(grace, "kill_grace")?;
        self.kill_grace = grace;
        Ok(self)
    }

    /// Sets the maximum number of bytes captured per output stream (default
    /// [`DEFAULT_MAX_OUTPUT_BYTES`], max [`MAX_OUTPUT_BYTES`]).
    pub fn max_output_bytes(mut self, max_bytes: usize) -> Result<Self, ExecError> {
        if max_bytes > MAX_OUTPUT_BYTES {
            return Err(ExecError::InvalidArgument {
                reason: "max_output_bytes exceeds hard limit of 64 MiB",
            });
        }
        self.max_output_bytes = max_bytes;
        Ok(self)
    }

    /// Returns the configured program name or path.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Returns the configured literal argument slice.
    #[must_use]
    pub fn get_args(&self) -> &[String] {
        &self.args
    }

    /// Returns the configured working directory, if any.
    #[must_use]
    pub fn get_cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Returns whether inherited environment variables are cleared.
    #[must_use]
    pub const fn is_env_cleared(&self) -> bool {
        self.clear_env
    }

    /// Returns the configured execution timeout, if any.
    #[must_use]
    pub const fn get_timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the configured timeout termination signal.
    #[must_use]
    pub const fn get_kill_signal(&self) -> Signal {
        self.kill_signal
    }

    /// Returns the configured grace period before `SIGKILL` escalation.
    #[must_use]
    pub const fn get_kill_grace(&self) -> Duration {
        self.kill_grace
    }

    /// Returns the configured per-stream output capture limit in bytes.
    #[must_use]
    pub const fn get_max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }
}

/// Captured output and termination metadata from [`crate::exec_sync`] or
/// [`crate::SpawnedChild::wait_with_output`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    /// Exit status code (`128 + signo` if terminated by signal on Unix, `-1` if unavailable).
    pub code: i32,
    /// Termination signal name (such as `"SIGTERM"` or `"SIGKILL"`), when killed by signal on Unix.
    pub signal: Option<String>,
    /// `true` when the child exited with code `0` and did not time out.
    pub success: bool,
    /// Captured standard output bytes.
    pub stdout: Vec<u8>,
    /// Captured standard error bytes.
    pub stderr: Vec<u8>,
    /// `true` when execution exceeded `timeout` and the child was terminated.
    pub timed_out: bool,
    /// `true` when captured output was truncated at a byte boundary.
    pub truncated: bool,
}

impl CommandOutput {
    /// Decodes captured `stdout` as UTF-8 (replacing invalid sequences with `U+FFFD`).
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Splits decoded `stdout` into lines, stripping `\n` / `\r\n` endings.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.text().lines().map(str::to_owned).collect()
    }

    /// Decodes captured `stderr` as UTF-8 (replacing invalid sequences with `U+FFFD`).
    #[must_use]
    pub fn error_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Splits decoded `stderr` into lines, stripping `\n` / `\r\n` endings.
    #[must_use]
    pub fn error_lines(&self) -> Vec<String> {
        self.error_text().lines().map(str::to_owned).collect()
    }
}

/// Exit status metadata returned by [`crate::run_sync`] and [`crate::SpawnedChild::wait`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandStatus {
    /// Exit status code (`128 + signo` if terminated by signal on Unix, `-1` if unavailable).
    pub code: i32,
    /// Termination signal name, when killed by signal on Unix.
    pub signal: Option<String>,
    /// `true` when the child exited with code `0` and did not time out.
    pub success: bool,
    /// `true` when execution exceeded `timeout` and the child was terminated.
    pub timed_out: bool,
}

pub(crate) fn validate_env_key(key: &str) -> Result<(), ExecError> {
    if key.is_empty() || key.contains('\0') || key.contains('=') {
        return Err(ExecError::InvalidArgument {
            reason: "environment variable key must be non-empty and contain neither '=' nor NUL",
        });
    }
    Ok(())
}

pub(crate) fn validate_path_not_empty_or_nul(
    path: &Path,
    label: &'static str,
) -> Result<(), ExecError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() {
        return Err(ExecError::InvalidArgument {
            reason: match label {
                "working directory" => "working directory must not be empty",
                _ => "path must not be empty",
            },
        });
    }
    if bytes.contains(&0) {
        return Err(ExecError::InvalidArgument {
            reason: match label {
                "working directory" => "working directory must not contain a NUL byte",
                _ => "path must not contain a NUL byte",
            },
        });
    }
    Ok(())
}

pub(crate) fn validate_finite_duration(
    duration: Duration,
    label: &'static str,
) -> Result<(), ExecError> {
    if Instant::now().checked_add(duration).is_none() {
        return Err(ExecError::InvalidArgument {
            reason: match label {
                "timeout" => "timeout overflows monotonic clock",
                "kill_grace" => "kill_grace overflows monotonic clock",
                _ => "duration overflows monotonic clock",
            },
        });
    }
    Ok(())
}

pub(crate) fn validate_positive_duration(
    duration: Duration,
    label: &'static str,
) -> Result<(), ExecError> {
    if duration.is_zero() {
        return Err(ExecError::InvalidArgument {
            reason: match label {
                "timeout" => "timeout must be greater than zero",
                _ => "duration must be greater than zero",
            },
        });
    }
    validate_finite_duration(duration, label)
}
