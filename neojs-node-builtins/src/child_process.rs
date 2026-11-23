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
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;

#[cfg(unix)]
use rustix::event::PollFd;
#[cfg(unix)]
use rustix::event::PollFlags;
#[cfg(unix)]
use rustix::event::Timespec;
#[cfg(unix)]
use rustix::event::poll;
#[cfg(unix)]
use rustix::fs::OFlags;
#[cfg(unix)]
use rustix::fs::fcntl_getfl;
#[cfg(unix)]
use rustix::fs::fcntl_setfl;
#[cfg(unix)]
use rustix::io::Errno;
#[cfg(unix)]
use rustix::io::read;
#[cfg(unix)]
use rustix::io::write;
#[cfg(unix)]
use rustix::process::Pid;
#[cfg(unix)]
use rustix::process::Signal;
#[cfg(unix)]
use rustix::process::kill_process_group;

/// The hard cap on one serialized bridge request, before parsing.
///
/// The guest wrapper uses the same value as its code-unit cap; the V8 op checks
/// a V8 string's UTF-8 length against it before copying the string to Rust.
pub const MAX_REQUEST_BYTES: usize = 4_194_304;

/// How long a killed or exited direct child's pipes may still be drained.
///
/// A descendant that inherited a piped descriptor can keep it open after the
/// direct child exits. Unix completion polls for this grace period, then kills
/// the direct child's process group and stops. It never joins a blocked reader
/// thread, so a descendant cannot defeat the deadline.
#[cfg(unix)]
const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(50);

/// The longest single poll wait, so `try_wait` and the deadline are checked.
#[cfg(unix)]
const POLL_SLICE: Duration = Duration::from_millis(10);

#[cfg(test)]
thread_local! {
    /// Simulates a platform without the bounded Unix runner.
    static FORCE_UNSUPPORTED: Cell<bool> = const { Cell::new(false) };
    /// Forces the post-spawn non-blocking setup to fail.
    static FAIL_NONBLOCKING: Cell<bool> = const { Cell::new(false) };
    /// Makes the injected setup failure wait for a child-side readiness file.
    static FAIL_READY: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Forces the next pipe read to fail with EIO.
    static FAIL_READ: Cell<bool> = const { Cell::new(false) };
    /// Exposes the pid of the last spawned child to the cleanup tests.
    static LAST_PID: Cell<Option<u32>> = const { Cell::new(None) };
}

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
/// a permission gate, not a sandbox. The allowlist matches paths, not inodes:
/// protect the executable and every ancestor directory from untrusted writes.
/// See [`Self::allow_program`] for the required permission prerequisite.
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
    /// symlink to an allowed file reaches the same entry.
    ///
    /// # Permission prerequisite
    ///
    /// This is a path allowlist, not an object pin. It does not pin the file's
    /// inode, ownership, or contents. An attacker who can replace the executable
    /// or any ancestor directory entry after this call can change what runs
    /// under the same allowed canonical path. The embedding application must
    /// protect the executable **and every ancestor directory** with ownership
    /// and permissions that only trusted principals can modify. The
    /// canonicalisation and regular-file check reduce accidental mistakes; they
    /// cannot close the operating system race between the check and `exec`.
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
///
/// This is the typed form of the request used by an engine adapter that can
/// build it without a JSON round trip. It is still validated against the policy
/// by [`run_sync`].
#[derive(Clone, Debug)]
pub struct SyncRequest {
    /// The absolute program path.
    pub file: String,
    /// Program arguments.
    pub args: Vec<String>,
    /// Optional working directory.
    pub cwd: Option<String>,
    /// Explicit environment; the ambient environment is never inherited.
    pub env: Option<BTreeMap<String, String>>,
    /// Optional standard input as UTF-8 text.
    pub input: Option<String>,
    /// Requested timeout in milliseconds; zero selects the policy maximum.
    pub timeout_ms: Option<u64>,
    /// Requested per-stream output bound.
    pub max_buffer: Option<usize>,
}

/// A completed synchronous request.
#[derive(Debug)]
pub struct SyncResult {
    /// The direct child's process id.
    pub pid: u32,
    /// The exit code, or `None` for signal/timeout/overflow/incomplete/io.
    pub status: Option<i32>,
    /// The terminating signal name, when known.
    pub signal_name: Option<&'static str>,
    /// Captured standard output, bounded by `maxBuffer`.
    pub stdout: Vec<u8>,
    /// Captured standard error, bounded by `maxBuffer`.
    pub stderr: Vec<u8>,
    /// Whether the deadline killed the child.
    pub timed_out: bool,
    /// Whether an output stream exceeded `maxBuffer`.
    pub limit_exceeded: bool,
    /// Whether a descendant held a pipe after the direct child exited.
    pub incomplete: bool,
    /// Whether an unexpected pipe read failed.
    pub io_error: bool,
}

/// Runs one typed request under `options` and the current policy.
///
/// This is the engine-neutral entry point for an adapter that constructs the
/// request in Rust. The JSON protocol uses the same runner.
pub fn run_sync(
    request: &SyncRequest,
    options: &NodeOptions,
) -> Result<SyncResult, ChildProcessError> {
    run(request, options.child_process())
}

/// Runs one validated request under `policy`.
fn run(
    request: &SyncRequest,
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

    // Reject before building or spawning the command. A platform without the
    // bounded pipe runner must not execute a child and then discover it cannot
    // clean it up safely.
    if !platform_supported() {
        return Err(platform_unsupported());
    }

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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Give the child its own process group so a bounded cleanup can signal
        // descendants that inherited its pipes.
        command.process_group(0);
    }

    let mut child = command.spawn().map_err(ChildProcessError::spawn)?;
    let pid = child.id();
    #[cfg(test)]
    LAST_PID.with(|last| last.set(Some(pid)));
    let mut guard = ChildGuard::new(&mut child);
    let execution = execute(&mut *guard.child, &input, timeout, max_buffer)?;
    guard.disarm();

    Ok(SyncResult {
        pid,
        status: execution.status,
        signal_name: execution.signal_name,
        stdout: execution.stdout,
        stderr: execution.stderr,
        timed_out: execution.timed_out,
        limit_exceeded: execution.limit_exceeded,
        incomplete: execution.incomplete,
        io_error: execution.io_error,
    })
}

/// The result of one bounded execution attempt.
struct Execution {
    status: Option<i32>,
    signal_name: Option<&'static str>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
    limit_exceeded: bool,
    incomplete: bool,
    io_error: bool,
}

/// Returns whether this build has the bounded Unix pipe runner.
#[cfg(unix)]
fn platform_supported() -> bool {
    #[cfg(test)]
    {
        !FORCE_UNSUPPORTED.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        true
    }
}

#[cfg(not(unix))]
fn platform_supported() -> bool {
    false
}

fn platform_unsupported() -> ChildProcessError {
    ChildProcessError::new(
        "ERR_NODE_CHILD_PROCESS_UNSUPPORTED",
        "bounded child-process completion is implemented on Unix only",
    )
}

/// Kills and reaps a spawned child if execution unwinds early.
///
/// The guard is armed immediately after `spawn` and disarmed only after the
/// bounded runner has completed and reaped the direct child. A post-spawn setup
/// failure therefore cannot leak a running process.
struct ChildGuard<'a> {
    child: &'a mut Child,
    armed: bool,
}

impl<'a> ChildGuard<'a> {
    fn new(child: &'a mut Child) -> Self {
        Self { child, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ChildGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            #[cfg(unix)]
            if let Some(pgid) = Pid::from_raw(self.child.id() as i32) {
                let _ = kill_process_group(pgid, Signal::KILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Runs a spawned child to a bounded completion on Unix.
///
/// Pipes are non-blocking and driven by `poll` on the calling thread. No reader
/// or writer thread is created, so a descendant that inherits a pipe and keeps
/// it open cannot make a join unbounded. After the direct child exits, or after
/// a timeout or output overflow, the runner drains for [`PIPE_DRAIN_GRACE`],
/// kills the child's process group, and returns. An escaped descendant in a new
/// process group may still hold a pipe, but the runner has already stopped
/// polling and drops its ends; there is no thread left waiting for it.
#[cfg(unix)]
fn execute(
    child: &mut std::process::Child,
    input: &[u8],
    timeout: Duration,
    max_buffer: usize,
) -> Result<Execution, ChildProcessError> {
    let pid = child.id();
    let pgid = Pid::from_raw(pid as i32).ok_or_else(|| {
        ChildProcessError::new("ERR_NODE_CHILD_PROCESS_SPAWN", "invalid child pid")
    })?;
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");

    if let Some(stdin) = &stdin {
        set_nonblocking(stdin)?;
    }
    set_nonblocking(&stdout)?;
    set_nonblocking(&stderr)?;

    let mut stdin_open = !input.is_empty() && stdin.is_some();
    if !stdin_open {
        // Closing the handle is what tells a child reading until EOF that the
        // input is complete.
        drop(stdin.take());
    }
    let mut stdin_offset = 0usize;
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut timed_out = false;
    let mut limit_exceeded = false;
    let mut incomplete = false;
    let mut io_error = false;
    let mut killed = false;
    let mut direct_status = None;

    let started = Instant::now();
    let hard_deadline = started + timeout;
    // A natural direct-child exit gets the same small drain window as a kill.
    let mut stop_at = hard_deadline;

    let mut read_buffer = [0u8; 8192];
    loop {
        if direct_status.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    direct_status = Some(status);
                    stdin_open = false;
                    stop_at = stop_at.min(Instant::now() + PIPE_DRAIN_GRACE);
                }
                Ok(None) => {}
                Err(error) => {
                    kill_child_group(child, pgid);
                    let _ = child.wait();
                    return Err(ChildProcessError::spawn(error));
                }
            }
        }

        let now = Instant::now();
        if direct_status.is_none() && now >= hard_deadline && !timed_out {
            timed_out = true;
            stdin_open = false;
            if !killed {
                kill_child_group(child, pgid);
                killed = true;
            }
            stop_at = now + PIPE_DRAIN_GRACE;
        }
        if now >= stop_at {
            if killed {
                break;
            }
            if direct_status.is_some() && (stdout_open || stderr_open) {
                // The direct child exited while a descendant still held its
                // piped streams. This is not a truthful success with possibly
                // truncated output; report it explicitly.
                incomplete = true;
            }
            kill_child_group(child, pgid);
            killed = true;
            stdin_open = false;
            stop_at = now + PIPE_DRAIN_GRACE;
        }

        if direct_status.is_some() && !stdout_open && !stderr_open && !stdin_open {
            break;
        }

        let wait = stop_at
            .saturating_duration_since(Instant::now())
            .min(POLL_SLICE);
        if wait.is_zero() {
            continue;
        }

        let mut fds = Vec::with_capacity(3);
        if stdin_open {
            fds.push(PollFd::new(
                stdin.as_ref().expect("open stdin has a handle"),
                PollFlags::OUT,
            ));
        }
        if stdout_open {
            fds.push(PollFd::new(&stdout, PollFlags::IN));
        }
        if stderr_open {
            fds.push(PollFd::new(&stderr, PollFlags::IN));
        }
        let timeout_spec = Timespec::try_from(wait).unwrap_or(Timespec {
            tv_sec: 0,
            tv_nsec: 1_000_000,
        });
        let ready = match poll(&mut fds, Some(&timeout_spec)) {
            Ok(ready) => ready,
            Err(error) if error == Errno::INTR => continue,
            Err(error) => {
                kill_child_group(child, pgid);
                let _ = child.wait();
                return Err(ChildProcessError::spawn(io::Error::from_raw_os_error(
                    error.raw_os_error(),
                )));
            }
        };
        if ready == 0 {
            continue;
        }

        let mut index = 0usize;
        if stdin_open {
            if fds[index]
                .revents()
                .intersects(PollFlags::OUT | PollFlags::ERR | PollFlags::HUP)
            {
                let stdin_handle = stdin.as_ref().expect("open stdin has a handle");
                match write(stdin_handle, &input[stdin_offset..]) {
                    Ok(0) => stdin_open = false,
                    Ok(written) => {
                        stdin_offset = stdin_offset.saturating_add(written);
                        if stdin_offset >= input.len() {
                            stdin_open = false;
                        }
                    }
                    Err(error) if error == Errno::AGAIN || error == Errno::INTR => {}
                    Err(_) => stdin_open = false,
                }
            }
            index += 1;
        }
        if stdout_open {
            if fds[index]
                .revents()
                .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
            {
                match read_pipe(&stdout, &mut read_buffer) {
                    Ok(0) => stdout_open = false,
                    Ok(read) => {
                        if append_capped(&mut stdout_bytes, &read_buffer[..read], max_buffer) {
                            limit_exceeded = true;
                            stdout_open = false;
                        }
                    }
                    Err(error) if error == Errno::AGAIN || error == Errno::INTR => {}
                    Err(_) => {
                        io_error = true;
                        stdout_open = false;
                    }
                }
            }
            index += 1;
        }
        if stderr_open
            && fds[index]
                .revents()
                .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
        {
            match read_pipe(&stderr, &mut read_buffer) {
                Ok(0) => stderr_open = false,
                Ok(read) => {
                    if append_capped(&mut stderr_bytes, &read_buffer[..read], max_buffer) {
                        limit_exceeded = true;
                        stderr_open = false;
                    }
                }
                Err(error) if error == Errno::AGAIN || error == Errno::INTR => {}
                Err(_) => {
                    io_error = true;
                    stderr_open = false;
                }
            }
        }
        if (limit_exceeded || io_error) && !killed {
            let now = Instant::now();
            kill_child_group(child, pgid);
            killed = true;
            stdin_open = false;
            stop_at = now + PIPE_DRAIN_GRACE;
        }
        if !stdin_open {
            drop(stdin.take());
        }
    }

    // The direct child is killed or already reaped; wait only for it. A
    // descendant that escaped the process group is not tracked.
    let status = child.wait().ok();
    let (status_code, signal_name) = match status {
        Some(status) if !timed_out && !limit_exceeded && !incomplete && !io_error => {
            (status.code(), exit_signal_name(&status))
        }
        Some(status) => (None, exit_signal_name(&status)),
        None => (None, None),
    };

    Ok(Execution {
        status: status_code,
        signal_name,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
        timed_out,
        limit_exceeded,
        incomplete,
        io_error,
    })
}

/// Non-Unix completion is refused rather than emulated with unbounded joins.
///
/// The bounded runner uses POSIX poll and process groups. A platform without
/// that implementation gets a clear failure, not a reader thread that a
/// descendant can keep blocked forever.
#[cfg(not(unix))]
fn execute(
    child: &mut std::process::Child,
    _input: &[u8],
    _timeout: Duration,
    _max_buffer: usize,
) -> Result<Execution, ChildProcessError> {
    let _ = child.kill();
    let _ = child.wait();
    Err(ChildProcessError::new(
        "ERR_NODE_CHILD_PROCESS_UNSUPPORTED",
        "bounded child-process completion is implemented on Unix only",
    ))
}

#[cfg(unix)]
fn set_nonblocking(fd: impl std::os::fd::AsFd) -> Result<(), ChildProcessError> {
    #[cfg(test)]
    if FAIL_NONBLOCKING.with(Cell::get) {
        if let Some(ready) = FAIL_READY.with(|ready| ready.borrow().clone()) {
            let started = Instant::now();
            while !ready.exists() && started.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        return Err(ChildProcessError::new(
            "ERR_NODE_TEST_NONBLOCKING",
            "injected non-blocking setup failure",
        ));
    }
    let flags = fcntl_getfl(&fd).map_err(|error| {
        ChildProcessError::spawn(io::Error::from_raw_os_error(error.raw_os_error()))
    })?;
    fcntl_setfl(&fd, flags | OFlags::NONBLOCK).map_err(|error| {
        ChildProcessError::spawn(io::Error::from_raw_os_error(error.raw_os_error()))
    })
}

#[cfg(unix)]
fn kill_child_group(child: &mut std::process::Child, pgid: Pid) {
    let _ = kill_process_group(pgid, Signal::KILL);
    let _ = child.kill();
}

#[cfg(unix)]
fn read_pipe(fd: impl std::os::fd::AsFd, buffer: &mut [u8]) -> Result<usize, Errno> {
    #[cfg(test)]
    if FAIL_READ.with(Cell::get) {
        return Err(Errno::IO);
    }
    read(fd, buffer)
}

/// Appends at most `limit` total bytes and reports whether the input overflowed.
#[cfg(any(unix, test))]
fn append_capped(output: &mut Vec<u8>, bytes: &[u8], limit: usize) -> bool {
    let remaining = limit.saturating_sub(output.len());
    if bytes.len() > remaining {
        output.extend_from_slice(&bytes[..remaining]);
        true
    } else {
        output.extend_from_slice(bytes);
        false
    }
}

/// Validates argument shape before a child exists.
fn check_args(request: &SyncRequest, limits: Limits) -> Result<(), ChildProcessError> {
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
fn check_environment(request: &SyncRequest, limits: Limits) -> Result<(), ChildProcessError> {
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
    use super::*;
    use std::path::Path;

    #[cfg(unix)]
    use rustix::io::Errno;
    #[cfg(unix)]
    use rustix::process::Pid;
    #[cfg(unix)]
    use rustix::process::test_kill_process;

    fn request(file: &str, args: &[&str]) -> SyncRequest {
        SyncRequest {
            file: file.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: None,
            env: None,
            input: None,
            timeout_ms: None,
            max_buffer: None,
        }
    }

    #[test]
    fn a_capped_append_stops_at_the_limit_and_reports_overflow() {
        let mut bytes = Vec::new();
        assert!(!append_capped(&mut bytes, b"abc", 3));
        assert_eq!(bytes, b"abc");
        assert!(append_capped(&mut bytes, b"d", 3));
        assert_eq!(bytes, b"abc");
    }

    #[test]
    fn a_capped_append_keeps_the_allowed_prefix_of_an_oversized_read() {
        let mut bytes = Vec::new();
        assert!(append_capped(&mut bytes, b"abcdef", 3));
        assert_eq!(bytes, b"abc");
    }

    #[cfg(unix)]
    #[test]
    fn an_unsupported_platform_rejects_before_spawn() {
        assert!(Path::new("/bin/sh").exists(), "test requires /bin/sh");
        let directory = tempfile::tempdir().expect("temp dir");
        let marker = directory.path().join("must-not-exist");
        let policy = ChildProcessPolicy::new()
            .allow_program("/bin/sh")
            .expect("allow sh");
        let request = request(
            "/bin/sh",
            &["-c", &format!("/bin/touch '{}'", marker.display())],
        );

        LAST_PID.with(|last| last.set(None));
        FORCE_UNSUPPORTED.with(|forced| forced.set(true));
        let result = run(&request, &policy);
        FORCE_UNSUPPORTED.with(|forced| forced.set(false));

        let error = result.expect_err("unsupported platform");
        assert_eq!(error.code(), "ERR_NODE_CHILD_PROCESS_UNSUPPORTED");
        assert!(
            !marker.exists(),
            "the unsupported platform executed the child"
        );
        assert_eq!(LAST_PID.with(Cell::get), None, "the child was spawned");
    }

    #[cfg(unix)]
    #[test]
    fn a_post_spawn_setup_failure_kills_the_child_group() {
        assert!(Path::new("/bin/sh").exists(), "test requires /bin/sh");
        assert!(Path::new("/bin/sleep").exists(), "test requires /bin/sleep");
        assert!(Path::new("/bin/touch").exists(), "test requires /bin/touch");
        let directory = tempfile::tempdir().expect("temp dir");
        let descendant_marker = directory.path().join("descendant-survived");
        let ready = directory.path().join("ready");
        let script = format!(
            "(/bin/sleep 0.2; /bin/touch '{}') & echo ready > '{}'; /bin/sleep 5",
            descendant_marker.display(),
            ready.display()
        );
        let policy = ChildProcessPolicy::new()
            .allow_program("/bin/sh")
            .expect("allow sh")
            .allow_program("/bin/sleep")
            .expect("allow sleep")
            .allow_program("/bin/touch")
            .expect("allow touch");
        let request = request("/bin/sh", &["-c", &script]);

        LAST_PID.with(|last| last.set(None));
        FAIL_READY.with(|path| *path.borrow_mut() = Some(ready));
        FAIL_NONBLOCKING.with(|failed| failed.set(true));
        let result = run(&request, &policy);
        FAIL_NONBLOCKING.with(|failed| failed.set(false));
        FAIL_READY.with(|path| *path.borrow_mut() = None);

        let error = result.expect_err("injected setup failure");
        assert_eq!(error.code(), "ERR_NODE_TEST_NONBLOCKING");
        let direct_pid = LAST_PID
            .with(Cell::get)
            .expect("the child was spawned before the failure");
        let direct_pid = Pid::from_raw(direct_pid as i32).expect("direct pid");
        assert!(
            matches!(test_kill_process(direct_pid), Err(Errno::SRCH)),
            "the failed setup left a live or unreaped direct child"
        );

        std::thread::sleep(Duration::from_millis(400));
        assert!(
            !descendant_marker.exists(),
            "the failed setup left a descendant alive"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unexpected_pipe_read_error_is_not_a_success() {
        assert!(Path::new("/bin/echo").exists(), "test requires /bin/echo");
        let policy = ChildProcessPolicy::new()
            .allow_program("/bin/echo")
            .expect("allow echo");
        let request = request("/bin/echo", &["hello"]);

        LAST_PID.with(|last| last.set(None));
        FAIL_READ.with(|failed| failed.set(true));
        let result = run(&request, &policy);
        FAIL_READ.with(|failed| failed.set(false));

        let result = result.expect("the runner returned a result");
        assert!(result.io_error, "a pipe read failure was treated as EOF");
        assert_eq!(
            result.status, None,
            "a pipe read failure kept a success status"
        );
    }
}
