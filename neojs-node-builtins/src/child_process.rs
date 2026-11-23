//! The bounded synchronous child-process runner.
//!
//! A request has already been parsed and type-checked by [`crate::protocol`].
//! This module re-validates every limit before spawning, so an adapter cannot
//! bypass a policy by constructing a request directly.
//!
//! The runner never invokes a shell and never reads the ambient environment.
//! It canonicalises the requested program and requires an exact match against
//! the policy's allowlist. The canonicalisation is a check, not a capability:
//! an allowed program can still act with the process's authority.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

/// The default maximum wall-clock time for one child process.
pub const DEFAULT_MAX_TIMEOUT: Duration = Duration::from_secs(10);
/// The default maximum bytes retained for one output stream.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1_048_576;
/// The default maximum bytes accepted for standard input.
pub const DEFAULT_MAX_INPUT_BYTES: usize = 1_048_576;
/// The default maximum number of arguments.
pub const DEFAULT_MAX_ARGS: usize = 256;
/// The default maximum total argument bytes.
pub const DEFAULT_MAX_ARG_BYTES: usize = 65_536;
/// The default maximum number of environment entries.
pub const DEFAULT_MAX_ENV_ENTRIES: usize = 128;
/// The default maximum total environment bytes.
pub const DEFAULT_MAX_ENV_BYTES: usize = 65_536;

/// Resource limits applied to every child process.
///
/// The defaults are deliberately finite. A `0` timeout is not used to mean
/// "unlimited": an explicit `timeout: 0` from JavaScript selects
/// [`Limits::max_timeout`] instead, so an embedding application always has a
/// bound. A `max_output_bytes` of `0` accepts no output; any byte written to a
/// piped stream is then an overflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum wall-clock time for one request.
    pub max_timeout: Duration,
    /// Maximum retained bytes for each of stdout and stderr.
    pub max_output_bytes: usize,
    /// Maximum bytes accepted for standard input.
    pub max_input_bytes: usize,
    /// Maximum number of arguments.
    pub max_args: usize,
    /// Maximum total bytes across all arguments.
    pub max_arg_bytes: usize,
    /// Maximum number of environment entries.
    pub max_env_entries: usize,
    /// Maximum total bytes across environment names and values.
    pub max_env_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_timeout: DEFAULT_MAX_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_args: DEFAULT_MAX_ARGS,
            max_arg_bytes: DEFAULT_MAX_ARG_BYTES,
            max_env_entries: DEFAULT_MAX_ENV_ENTRIES,
            max_env_bytes: DEFAULT_MAX_ENV_BYTES,
        }
    }
}

impl Limits {
    /// Sets the wall-clock bound for one request.
    pub fn with_max_timeout(mut self, max_timeout: Duration) -> Self {
        self.max_timeout = max_timeout;
        self
    }

    /// Sets the retained-byte bound for each output stream.
    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    /// Sets the standard-input bound.
    pub fn with_max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    /// Sets the argument-count bound.
    pub fn with_max_args(mut self, max_args: usize) -> Self {
        self.max_args = max_args;
        self
    }

    /// Sets the total argument-byte bound.
    pub fn with_max_arg_bytes(mut self, max_arg_bytes: usize) -> Self {
        self.max_arg_bytes = max_arg_bytes;
        self
    }

    /// Sets the environment-entry bound.
    pub fn with_max_env_entries(mut self, max_env_entries: usize) -> Self {
        self.max_env_entries = max_env_entries;
        self
    }

    /// Sets the total environment-byte bound.
    pub fn with_max_env_bytes(mut self, max_env_bytes: usize) -> Self {
        self.max_env_bytes = max_env_bytes;
        self
    }
}

/// A failure while accepting a program into a policy.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// The program could not be canonicalised or inspected.
    #[error("cannot allow child-process program: {source}")]
    Program {
        /// The operating-system error.
        #[source]
        source: io::Error,
    },

    /// The canonical path is not a regular file.
    #[error("the child-process program is not a regular file")]
    NotAFile,
}

/// The authority and resource limits for `node:child_process`.
///
/// The default policy allows no program. Build it with [`Self::allow_program`],
/// which canonicalises the path and rejects anything that is not a regular
/// file. A program is allowed only when the request names its canonical path
/// exactly; a relative name, a bare name, and a `PATH` search are all rejected.
///
/// Adding a program grants it the full authority of the neojs process. This is
/// a permission gate, not a sandbox.
#[derive(Clone, Debug, Default)]
pub struct ChildProcessPolicy {
    allowed_programs: BTreeSet<PathBuf>,
    limits: Limits,
}

impl ChildProcessPolicy {
    /// Creates a policy that allows no programs and uses the default limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one exact executable path to the allowlist.
    ///
    /// The path is canonicalised now and must name an existing regular file.
    /// The runner canonicalises the requested path again before comparing, so a
    /// symlink to an allowed file reaches the same entry. Two canonicalisations
    /// cannot remove the operating system race between the check and `exec`;
    /// the allowlist identifies the file that was approved, it does not pin its
    /// contents.
    pub fn allow_program(mut self, program: impl AsRef<Path>) -> Result<Self, PolicyError> {
        let canonical = std::fs::canonicalize(program.as_ref())
            .map_err(|source| PolicyError::Program { source })?;
        let metadata =
            std::fs::metadata(&canonical).map_err(|source| PolicyError::Program { source })?;
        if !metadata.is_file() {
            return Err(PolicyError::NotAFile);
        }
        self.allowed_programs.insert(canonical);
        Ok(self)
    }

    /// Replaces the resource limits.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the effective limits.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Returns whether an already-canonical path is allowed.
    pub fn allows(&self, program: &Path) -> bool {
        self.allowed_programs.contains(program)
    }
}

/// The opt-in configuration for Node compatibility in one runtime.
///
/// This supplies the process policy to a runtime constructed with Node
/// compatibility. `NodeOptions::default()` grants no process authority: the
/// `node:` resolver exists, but every execution is denied. Use
/// [`NodeOptions::new`] with a [`ChildProcessPolicy`] to allow specific
/// executables. Node modules remain absent unless the embedding application
/// constructs the runtime with these options.
#[derive(Clone, Debug, Default)]
pub struct NodeOptions {
    child_process: ChildProcessPolicy,
}

impl NodeOptions {
    /// Creates options with the given child-process policy.
    pub fn new(child_process: ChildProcessPolicy) -> Self {
        Self { child_process }
    }

    /// Returns the child-process policy.
    pub fn child_process(&self) -> &ChildProcessPolicy {
        &self.child_process
    }
}

/// A coded child-process failure.
///
/// The code is stable and safe to match on. The message describes the failure
/// without including environment values, standard input, argument values, or
/// captured output. An operating-system error is included only through its own
/// short description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildProcessError {
    code: String,
    message: String,
}

impl ChildProcessError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn invalid(message: &str) -> Self {
        Self::new("ERR_INVALID_ARG_VALUE", message.to_string())
    }

    fn limit(message: &str) -> Self {
        Self::new("ERR_NODE_LIMIT", message.to_string())
    }

    fn denied() -> Self {
        Self::new(
            "ERR_NODE_CHILD_PROCESS_DENIED",
            "child process execution is not allowed by this runtime's policy".to_string(),
        )
    }

    fn not_absolute() -> Self {
        Self::new(
            "ERR_NODE_COMMAND_NOT_ABSOLUTE",
            "the child-process program must be an absolute path; PATH lookup is not implemented"
                .to_string(),
        )
    }

    fn spawn(error: io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::NotFound => "ENOENT",
            io::ErrorKind::PermissionDenied => "EACCES",
            _ => "ERR_NODE_CHILD_PROCESS_SPAWN",
        };
        Self::new(code, error.to_string())
    }

    /// Returns the stable error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ChildProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ChildProcessError {}

/// A validated synchronous request.
pub(crate) struct RunRequest {
    pub(crate) file: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) env: Option<BTreeMap<String, String>>,
    pub(crate) input: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_buffer: Option<usize>,
}

/// A completed synchronous request.
pub(crate) struct SyncResult {
    pub(crate) pid: u32,
    pub(crate) status: Option<i32>,
    pub(crate) signal_name: Option<&'static str>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) timed_out: bool,
    pub(crate) limit_exceeded: bool,
}

/// Runs one validated request under `policy`.
pub(crate) fn run(
    request: &RunRequest,
    policy: &ChildProcessPolicy,
) -> Result<SyncResult, ChildProcessError> {
    let limits = policy.limits;

    if request.file.is_empty() {
        return Err(ChildProcessError::invalid(
            "the child-process program must not be empty",
        ));
    }
    if request.file.contains('\0') {
        return Err(ChildProcessError::invalid(
            "the child-process program must not contain a NUL byte",
        ));
    }
    let requested = Path::new(&request.file);
    if !requested.is_absolute() {
        return Err(ChildProcessError::not_absolute());
    }
    let program = std::fs::canonicalize(requested).map_err(ChildProcessError::spawn)?;
    if !policy.allows(&program) {
        return Err(ChildProcessError::denied());
    }

    check_args(request, limits)?;
    check_environment(request, limits)?;
    if let Some(directory) = &request.cwd
        && directory.contains('\0')
    {
        return Err(ChildProcessError::invalid(
            "the working directory must not contain a NUL byte",
        ));
    }
    let input = request.input.as_deref().unwrap_or("").as_bytes().to_vec();
    if input.len() > limits.max_input_bytes {
        return Err(ChildProcessError::limit(
            "standard input exceeds the configured limit",
        ));
    }

    let timeout = match request.timeout_ms {
        // `timeout: 0` means no explicit deadline in Node. This runtime keeps
        // the policy's finite bound instead of disabling the guard.
        None | Some(0) => limits.max_timeout,
        Some(milliseconds) => {
            let requested = Duration::from_millis(milliseconds);
            if requested > limits.max_timeout {
                return Err(ChildProcessError::limit(
                    "the requested timeout exceeds the configured limit",
                ));
            }
            requested
        }
    };

    let max_buffer = match request.max_buffer {
        None => limits.max_output_bytes,
        Some(0) => {
            return Err(ChildProcessError::invalid(
                "maxBuffer must be a positive integer",
            ));
        }
        Some(value) if value > limits.max_output_bytes => {
            return Err(ChildProcessError::limit(
                "the requested maxBuffer exceeds the configured limit",
            ));
        }
        Some(value) => value,
    };

    let mut command = Command::new(&program);
    command.args(&request.args);
    command.env_clear();
    if let Some(environment) = &request.env {
        for (name, value) in environment {
            command.env(name, value);
        }
    }
    if let Some(directory) = &request.cwd {
        command.current_dir(directory);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(ChildProcessError::spawn)?;
    let pid = child.id();
    let overflow = Arc::new(AtomicBool::new(false));

    let stdout_reader = reader_thread(
        child.stdout.take().expect("stdout is piped"),
        max_buffer,
        Arc::clone(&overflow),
    );
    let stderr_reader = reader_thread(
        child.stderr.take().expect("stderr is piped"),
        max_buffer,
        Arc::clone(&overflow),
    );
    let writer = if input.is_empty() {
        // Dropping the handle closes standard input, which is the signal a
        // child reading until EOF expects.
        drop(child.stdin.take());
        None
    } else {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        Some(thread::spawn(move || {
            // A child that exits early closes the pipe. A failed write is not a
            // separate failure: the exit status and captured output are the
            // authoritative result.
            let _ = stdin.write_all(&input);
        }))
    };

    let started = Instant::now();
    let mut timed_out = false;
    let mut limit_exceeded = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                join_quietly(writer);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(ChildProcessError::spawn(error));
            }
        }

        if overflow.load(Ordering::SeqCst) {
            limit_exceeded = true;
        } else if started.elapsed() >= timeout {
            timed_out = true;
        } else {
            thread::sleep(Duration::from_millis(2));
            continue;
        }

        let _ = child.kill();
        break;
    }

    // Reap the direct child even when it was killed. On Unix a SIGKILL cannot be
    // ignored, so this does not wait for a cooperating process.
    let status = child.wait().ok();
    join_quietly(writer);
    let stdout = join_reader(stdout_reader);
    let stderr = join_reader(stderr_reader);
    if overflow.load(Ordering::SeqCst) {
        limit_exceeded = true;
    }

    let (status_code, signal_name) = match status {
        Some(status) if !timed_out && !limit_exceeded => (status.code(), exit_signal_name(&status)),
        Some(status) => (None, exit_signal_name(&status)),
        None => (None, None),
    };

    Ok(SyncResult {
        pid,
        status: status_code,
        signal_name,
        stdout,
        stderr,
        timed_out,
        limit_exceeded,
    })
}

/// Validates argument shape before a child exists.
fn check_args(request: &RunRequest, limits: Limits) -> Result<(), ChildProcessError> {
    if request.args.len() > limits.max_args {
        return Err(ChildProcessError::limit(
            "the argument count exceeds the configured limit",
        ));
    }
    let mut total = 0usize;
    for argument in &request.args {
        if argument.contains('\0') {
            return Err(ChildProcessError::invalid(
                "an argument must not contain a NUL byte",
            ));
        }
        total = total.saturating_add(argument.len());
    }
    if total > limits.max_arg_bytes {
        return Err(ChildProcessError::limit(
            "the arguments exceed the configured byte limit",
        ));
    }
    Ok(())
}

/// Validates the explicit environment map before a child exists.
fn check_environment(request: &RunRequest, limits: Limits) -> Result<(), ChildProcessError> {
    let Some(environment) = &request.env else {
        return Ok(());
    };
    if environment.len() > limits.max_env_entries {
        return Err(ChildProcessError::limit(
            "the environment entry count exceeds the configured limit",
        ));
    }
    let mut total = 0usize;
    for (name, value) in environment {
        if name.is_empty() || name.contains('=') || name.contains('\0') {
            return Err(ChildProcessError::invalid(
                "an environment name is not valid",
            ));
        }
        if value.contains('\0') {
            return Err(ChildProcessError::invalid(
                "an environment value must not contain a NUL byte",
            ));
        }
        total = total.saturating_add(name.len()).saturating_add(value.len());
    }
    if total > limits.max_env_bytes {
        return Err(ChildProcessError::limit(
            "the environment exceeds the configured byte limit",
        ));
    }
    Ok(())
}

/// Reads at most `limit` bytes and records an overflow.
fn read_capped(mut reader: impl Read, limit: usize, overflow: Arc<AtomicBool>) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let remaining = limit.saturating_sub(output.len());
                if read > remaining {
                    output.extend_from_slice(&buffer[..remaining]);
                    overflow.store(true, Ordering::SeqCst);
                    break;
                }
                output.extend_from_slice(&buffer[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    output
}

fn reader_thread(
    reader: impl Read + Send + 'static,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || read_capped(reader, limit, overflow))
}

fn join_reader(handle: thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

fn join_quietly(handle: Option<thread::JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

#[cfg(unix)]
fn exit_signal_name(status: &ExitStatus) -> Option<&'static str> {
    use std::os::unix::process::ExitStatusExt;
    signal_name(status.signal()?)
}

#[cfg(not(unix))]
fn exit_signal_name(_status: &ExitStatus) -> Option<&'static str> {
    None
}

/// Names the signals this slice can report without a platform binding.
#[cfg(unix)]
fn signal_name(number: i32) -> Option<&'static str> {
    Some(match number {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        6 => "SIGABRT",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::read_capped;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    #[test]
    fn a_reader_stops_one_byte_past_the_limit_and_records_it() {
        let overflow = Arc::new(AtomicBool::new(false));
        let bytes = read_capped(&b"abcdef"[..], 3, Arc::clone(&overflow));
        assert_eq!(bytes, b"abc");
        assert!(overflow.load(Ordering::SeqCst));
    }

    #[test]
    fn a_reader_accepts_exactly_the_limit() {
        let overflow = Arc::new(AtomicBool::new(false));
        let bytes = read_capped(&b"abc"[..], 3, Arc::clone(&overflow));
        assert_eq!(bytes, b"abc");
        assert!(!overflow.load(Ordering::SeqCst));
    }
}
