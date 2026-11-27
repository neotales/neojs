//! Safe, bounded synchronous child process execution backing `node:child_process`.
//!
//! * Never invokes a shell (`sh -c` or `cmd.exe /c`): executes `program` directly.
//! * Drains `stdout` and `stderr` concurrently on scoped reader threads capped at
//!   [`SpawnOptions::max_buffer`] so noisy children cannot OOM the host or deadlock pipes.
//! * Writes optional `stdin` concurrently on a scoped writer thread.
//! * Enforces [`SpawnOptions::timeout_ms`] via `try_wait` polling, terminating and
//!   reaping timed-out children so no zombie processes are left behind.

#[cfg(any(windows, test))]
use std::ffi::OsStr;
use std::io::{self, Read as _, Write as _};
#[cfg(any(windows, test))]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{NodeCoreError, map_io_error};

/// Default capture cap per output stream (`10 MiB`).
pub const DEFAULT_MAX_BUFFER: usize = 10 * 1024 * 1024;

/// Hard upper bound accepted for [`SpawnOptions::max_buffer`] (`64 MiB`).
pub const MAX_MAX_BUFFER: usize = 64 * 1024 * 1024;

const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(75);
#[cfg(unix)]
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Configuration for [`spawn_sync`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnOptions {
    /// Working directory for the child process. Inherited when `None`.
    pub cwd: Option<PathBuf>,
    /// Environment variables to set on the child process.
    pub env: Option<Vec<(String, String)>>,
    /// When `true`, clears inherited environment variables before applying `env`.
    pub clear_env: bool,
    /// Optional bytes written to the child's standard input before closing EOF.
    pub stdin: Option<Vec<u8>>,
    /// Optional timeout in milliseconds after which the child is killed and reaped.
    pub timeout_ms: Option<u64>,
    /// Maximum bytes retained per output stream (`stdout` and `stderr`).
    pub max_buffer: usize,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            cwd: None,
            env: None,
            clear_env: false,
            stdin: None,
            timeout_ms: None,
            max_buffer: DEFAULT_MAX_BUFFER,
        }
    }
}

/// Captured result returned by [`spawn_sync`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnOutput {
    /// Process exit code, or `None` if terminated by a signal.
    pub status: Option<i32>,
    /// POSIX termination signal name (such as `"SIGKILL"`), when terminated by signal on Unix.
    pub signal: Option<String>,
    /// Captured standard output bytes (bounded by `max_buffer`).
    pub stdout: Vec<u8>,
    /// Captured standard error bytes (bounded by `max_buffer`).
    pub stderr: Vec<u8>,
    /// `true` when the child exceeded `timeout_ms` and was terminated.
    pub timed_out: bool,
    /// `true` when `stdout` or `stderr` exceeded `max_buffer` and excess bytes were discarded.
    pub truncated: bool,
}

/// Spawns `program` directly with `args` (never via a shell) and waits for completion.
///
/// Rejects empty or NUL-containing `program`, `args`, `cwd`, or `env` keys/values
/// before spawning. Concurrently drains `stdout` and `stderr` up to `opts.max_buffer`
/// bytes per stream, feeds `opts.stdin` if provided, and kills + reaps the child
/// if `opts.timeout_ms` elapses.
///
/// # Example
///
/// ```no_run
/// use vaeljs_node_core::child_process::{SpawnOptions, spawn_sync};
///
/// let out = spawn_sync("echo", &["hello".to_owned()], &SpawnOptions::default()).unwrap();
/// assert_eq!(out.status, Some(0));
/// ```
pub fn spawn_sync(
    program: &str,
    args: &[String],
    opts: &SpawnOptions,
) -> Result<SpawnOutput, NodeCoreError> {
    validate_spawn_inputs(program, args, opts)?;

    let resolved_program = resolve_program_for_spawn(program, opts)?;
    let mut cmd = Command::new(&resolved_program);
    cmd.args(args);

    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    if opts.clear_env {
        cmd.env_clear();
    }
    if let Some(env) = &opts.env {
        for (key, val) in env {
            cmd.env(key, val);
        }
    }

    let max_buf = opts.max_buffer;
    let truncated = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));

    #[cfg(unix)]
    let (mut child, stdin_handle, stdout_handle, stderr_handle) = {
        use std::net::Shutdown;
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let stdin_pair = if opts.stdin.is_some() {
            let (parent_in, child_in) =
                UnixStream::pair().map_err(|err| map_io_error("create stdin pipe", err))?;
            cmd.stdin(Stdio::from(OwnedFd::from(child_in)));
            Some(parent_in)
        } else {
            cmd.stdin(Stdio::null());
            None
        };

        let (parent_out, child_out) =
            UnixStream::pair().map_err(|err| map_io_error("create stdout pipe", err))?;
        cmd.stdout(Stdio::from(OwnedFd::from(child_out)));

        let (parent_err, child_err) =
            UnixStream::pair().map_err(|err| map_io_error("create stderr pipe", err))?;
        cmd.stderr(Stdio::from(OwnedFd::from(child_err)));

        let child = cmd
            .spawn()
            .map_err(|err| map_io_error("spawn child process", err))?;
        // Drop `cmd` immediately so parent never retains child-side FDs.
        drop(cmd);

        let stdin_handle = stdin_pair
            .zip(opts.stdin.as_deref())
            .map(|(pipe, data)| spawn_unix_writer(pipe, data.to_vec(), Arc::clone(&cancel)));
        let stdout_handle = spawn_unix_reader(
            parent_out,
            max_buf,
            Arc::clone(&truncated),
            Arc::clone(&cancel),
        );
        let stderr_handle = spawn_unix_reader(
            parent_err,
            max_buf,
            Arc::clone(&truncated),
            Arc::clone(&cancel),
        );

        let _ = Shutdown::Both;
        (
            child,
            stdin_handle,
            Some(stdout_handle),
            Some(stderr_handle),
        )
    };

    #[cfg(not(unix))]
    let (mut child, stdin_handle, stdout_handle, stderr_handle) = {
        if opts.stdin.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|err| map_io_error("spawn child process", err))?;
        drop(cmd);

        let stdin_pipe = child.stdin.take();
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let stdin_handle = stdin_pipe
            .zip(opts.stdin.as_deref())
            .map(|(pipe, data)| spawn_writer(pipe, data.to_vec()));
        let stdout_handle =
            stdout_pipe.map(|pipe| spawn_reader(pipe, max_buf, Arc::clone(&truncated)));
        let stderr_handle =
            stderr_pipe.map(|pipe| spawn_reader(pipe, max_buf, Arc::clone(&truncated)));
        (child, stdin_handle, stdout_handle, stderr_handle)
    };

    let mut timed_out = false;
    let mut exit_status: Option<ExitStatus> = None;
    let mut wait_error: Option<io::Error> = None;

    match wait_with_timeout(&mut child, opts.timeout_ms) {
        Ok((status, did_timeout)) => {
            exit_status = Some(status);
            timed_out = did_timeout;
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            wait_error = Some(err);
        }
    }

    let grace_deadline = if timed_out || wait_error.is_some() {
        Instant::now().checked_add(PIPE_DRAIN_GRACE)
    } else {
        None
    };

    if let Some(ref handle) = stdin_handle {
        wait_for_done(&handle.signal, grace_deadline);
    }

    let mut read_error: Option<io::Error> = None;
    let stdout = if let Some(ref handle) = stdout_handle {
        let (buf, err) = collect_stream(&handle.signal, grace_deadline);
        if read_error.is_none() {
            read_error = err;
        }
        buf
    } else {
        Vec::new()
    };
    let stderr = if let Some(ref handle) = stderr_handle {
        let (buf, err) = collect_stream(&handle.signal, grace_deadline);
        if read_error.is_none() {
            read_error = err;
        }
        buf
    } else {
        Vec::new()
    };

    // On timeout/wait failure, unblock and join Unix stdio worker threads so
    // grandchildren holding inherited FDs never leak background threads.
    if grace_deadline.is_some() {
        cancel.store(true, Ordering::Release);
    }
    if let Some(handle) = stdin_handle {
        handle.finish(grace_deadline.is_some());
    }
    if let Some(handle) = stdout_handle {
        handle.finish(grace_deadline.is_some());
    }
    if let Some(handle) = stderr_handle {
        handle.finish(grace_deadline.is_some());
    }

    if let Some(err) = wait_error {
        return Err(NodeCoreError::ProcessError {
            context: "wait for child process",
            reason: "failed while waiting for child".to_owned(),
            source: Some(err),
        });
    }
    if let Some(err) = read_error
        && !timed_out
    {
        return Err(NodeCoreError::ProcessError {
            context: "read child process output",
            reason: "failed while reading child output".to_owned(),
            source: Some(err),
        });
    }

    let exit_status = exit_status.ok_or_else(|| NodeCoreError::ProcessError {
        context: "wait for child process",
        reason: "child exit status unavailable".to_owned(),
        source: None,
    })?;

    Ok(SpawnOutput {
        status: exit_status.code(),
        signal: extract_signal(&exit_status),
        stdout,
        stderr,
        timed_out,
        truncated: truncated.load(Ordering::Relaxed),
    })
}

fn validate_spawn_inputs(
    program: &str,
    args: &[String],
    opts: &SpawnOptions,
) -> Result<(), NodeCoreError> {
    if program.is_empty() {
        return Err(NodeCoreError::InvalidInput {
            reason: "program name must not be empty",
        });
    }
    if program.contains('\0') {
        return Err(NodeCoreError::InvalidInput {
            reason: "program name must not contain a NUL byte",
        });
    }
    for arg in args {
        if arg.contains('\0') {
            return Err(NodeCoreError::InvalidInput {
                reason: "process argument must not contain a NUL byte",
            });
        }
    }
    if let Some(cwd) = &opts.cwd {
        let bytes = cwd.as_os_str().as_encoded_bytes();
        if bytes.is_empty() {
            return Err(NodeCoreError::InvalidPath {
                reason: "child working directory must not be empty",
            });
        }
        if bytes.contains(&0) {
            return Err(NodeCoreError::InvalidPath {
                reason: "child working directory must not contain a NUL byte",
            });
        }
    }
    if let Some(env) = &opts.env {
        for (key, val) in env {
            if key.is_empty() || key.contains('\0') || key.contains('=') {
                return Err(NodeCoreError::InvalidInput {
                    reason: "environment variable key must be non-empty and contain neither '=' nor NUL",
                });
            }
            if val.contains('\0') {
                return Err(NodeCoreError::InvalidInput {
                    reason: "environment variable value must not contain a NUL byte",
                });
            }
        }
    }
    if opts.max_buffer > MAX_MAX_BUFFER {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_MAX_BUFFER,
            actual: Some(opts.max_buffer),
        });
    }
    if let Some(ms) = opts.timeout_ms {
        deadline_from_duration(Instant::now(), Duration::from_millis(ms))?;
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn is_bare_program_name(program: &str) -> bool {
    let bytes = program.as_bytes();
    let has_drive = bytes.get(1) == Some(&b':');
    !program.contains('/') && !program.contains('\\') && !has_drive
}

#[cfg(any(windows, test))]
fn resolve_bare_program_in_path(
    program: &str,
    path_var: Option<&OsStr>,
    pathext_var: Option<&OsStr>,
    path_sep: char,
) -> Option<PathBuf> {
    let path_str = path_var?.to_str()?;
    let has_ext = Path::new(program).extension().is_some();
    let exts: Vec<&str> = if has_ext {
        vec![""]
    } else {
        let raw = pathext_var
            .and_then(|v| v.to_str())
            .unwrap_or(".COM;.EXE;.BAT;.CMD");
        let parsed: Vec<&str> = raw.split(';').filter(|s| !s.is_empty()).collect();
        if parsed.is_empty() {
            vec![".COM", ".EXE", ".BAT", ".CMD"]
        } else {
            parsed
        }
    };

    for dir_raw in path_str.split(path_sep) {
        if dir_raw.is_empty() {
            continue;
        }
        let dir = Path::new(dir_raw);
        if !dir.is_absolute() {
            continue;
        }
        for ext in &exts {
            let candidate = if ext.is_empty() {
                dir.join(program)
            } else {
                dir.join(format!("{program}{ext}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_program_for_spawn(program: &str, opts: &SpawnOptions) -> Result<PathBuf, NodeCoreError> {
    #[cfg(windows)]
    {
        if !is_bare_program_name(program) {
            return Ok(PathBuf::from(program));
        }
        let env_lookup = |target: &str| -> Option<std::ffi::OsString> {
            if let Some(env) = &opts.env {
                let mut found = None;
                for (k, v) in env {
                    if k.eq_ignore_ascii_case(target) {
                        found = Some(std::ffi::OsString::from(v));
                    }
                }
                if found.is_some() {
                    return found;
                }
            }
            if opts.clear_env {
                None
            } else {
                std::env::var_os(target)
            }
        };
        let path_var = env_lookup("PATH");
        let pathext_var = env_lookup("PATHEXT");
        resolve_bare_program_in_path(program, path_var.as_deref(), pathext_var.as_deref(), ';')
            .ok_or_else(|| NodeCoreError::NotFound {
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    "executable not found on absolute PATH entries",
                ),
            })
    }
    #[cfg(not(windows))]
    {
        let _ = opts;
        Ok(PathBuf::from(program))
    }
}

fn deadline_from_duration(now: Instant, duration: Duration) -> Result<Instant, NodeCoreError> {
    now.checked_add(duration)
        .ok_or(NodeCoreError::InvalidInput {
            reason: "timeout_ms overflows monotonic clock",
        })
}

#[derive(Default)]
struct StreamState {
    buf: Vec<u8>,
    done: bool,
    err: Option<io::Error>,
}

struct WorkerHandle<T> {
    signal: Arc<(Mutex<T>, Condvar)>,
    #[cfg(unix)]
    shutdown_stream: Option<std::os::unix::net::UnixStream>,
    join: Option<JoinHandle<()>>,
}

impl<T> WorkerHandle<T> {
    fn finish(self, should_join: bool) {
        #[cfg(unix)]
        if let Some(stream) = self.shutdown_stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(join) = self.join {
            #[cfg(unix)]
            {
                let _ = should_join;
                let _ = join.join();
            }
            #[cfg(not(unix))]
            if !should_join {
                let _ = join.join();
            }
        }
    }
}

#[cfg(unix)]
fn spawn_unix_writer(
    mut stream: std::os::unix::net::UnixStream,
    payload: Vec<u8>,
    cancel: Arc<AtomicBool>,
) -> WorkerHandle<bool> {
    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_signal = Arc::clone(&signal);
    let shutdown_stream = stream.try_clone().ok();
    let _ = stream.set_write_timeout(Some(STREAM_POLL_INTERVAL));
    let join = std::thread::spawn(move || {
        let mut offset = 0;
        while offset < payload.len() && !cancel.load(Ordering::Acquire) {
            match stream.write(&payload[offset..]) {
                Ok(0) => break,
                Ok(n) => offset += n,
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(_) => break,
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Write);
        drop(stream);
        let (lock, cvar) = &*worker_signal;
        let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
        *done = true;
        cvar.notify_all();
    });
    WorkerHandle {
        signal,
        shutdown_stream,
        join: Some(join),
    }
}

#[cfg(not(unix))]
fn spawn_writer(mut pipe: impl Write + Send + 'static, payload: Vec<u8>) -> WorkerHandle<bool> {
    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_signal = Arc::clone(&signal);
    let join = std::thread::spawn(move || {
        let _ = pipe.write_all(&payload);
        drop(pipe);
        let (lock, cvar) = &*worker_signal;
        let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
        *done = true;
        cvar.notify_all();
    });
    WorkerHandle {
        signal,
        join: Some(join),
    }
}

#[cfg(unix)]
fn spawn_unix_reader(
    mut stream: std::os::unix::net::UnixStream,
    max_buffer: usize,
    truncated: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> WorkerHandle<StreamState> {
    let signal = Arc::new((Mutex::new(StreamState::default()), Condvar::new()));
    let worker_signal = Arc::clone(&signal);
    let shutdown_stream = stream.try_clone().ok();
    let _ = stream.set_read_timeout(Some(STREAM_POLL_INTERVAL));
    let join = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            if cancel.load(Ordering::Acquire) {
                let (lock, cvar) = &*worker_signal;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                state.done = true;
                cvar.notify_all();
                break;
            }
            match stream.read(&mut chunk) {
                Ok(0) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
                Ok(n) => {
                    let (lock, _) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let remaining = max_buffer.saturating_sub(state.buf.len());
                    if remaining > 0 {
                        let take = n.min(remaining);
                        state.buf.extend_from_slice(&chunk[..take]);
                        if take < n {
                            truncated.store(true, Ordering::Relaxed);
                        }
                    } else {
                        truncated.store(true, Ordering::Relaxed);
                    }
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(err) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.err = Some(err);
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
            }
        }
    });
    WorkerHandle {
        signal,
        shutdown_stream,
        join: Some(join),
    }
}

#[cfg(not(unix))]
fn spawn_reader(
    mut reader: impl io::Read + Send + 'static,
    max_buffer: usize,
    truncated: Arc<AtomicBool>,
) -> WorkerHandle<StreamState> {
    let signal = Arc::new((Mutex::new(StreamState::default()), Condvar::new()));
    let worker_signal = Arc::clone(&signal);
    let join = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
                Ok(n) => {
                    let (lock, _) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let remaining = max_buffer.saturating_sub(state.buf.len());
                    if remaining > 0 {
                        let take = n.min(remaining);
                        state.buf.extend_from_slice(&chunk[..take]);
                        if take < n {
                            truncated.store(true, Ordering::Relaxed);
                        }
                    } else {
                        truncated.store(true, Ordering::Relaxed);
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.err = Some(err);
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
            }
        }
    });
    WorkerHandle {
        signal,
        join: Some(join),
    }
}

fn wait_for_done(signal: &(Mutex<bool>, Condvar), deadline: Option<Instant>) {
    let (lock, cvar) = signal;
    let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    match deadline {
        None => {
            let _guard = cvar.wait_while(guard, |done| !*done);
        }
        Some(dl) => {
            let remaining = dl.saturating_duration_since(Instant::now());
            if !*guard && !remaining.is_zero() {
                let _guard = cvar.wait_timeout_while(guard, remaining, |done| !*done);
            }
        }
    }
}

fn collect_stream(
    shared: &(Mutex<StreamState>, Condvar),
    deadline: Option<Instant>,
) -> (Vec<u8>, Option<io::Error>) {
    let (lock, cvar) = shared;
    let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = match deadline {
        None => cvar
            .wait_while(guard, |s| !s.done)
            .unwrap_or_else(|e| e.into_inner()),
        Some(dl) => {
            let remaining = dl.saturating_duration_since(Instant::now());
            if !s_done(&guard) && !remaining.is_zero() {
                cvar.wait_timeout_while(guard, remaining, |s| !s.done)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0)
            } else {
                guard
            }
        }
    };
    (std::mem::take(&mut state.buf), state.err.take())
}

fn s_done(state: &StreamState) -> bool {
    state.done
}

fn wait_with_timeout(child: &mut Child, timeout_ms: Option<u64>) -> io::Result<(ExitStatus, bool)> {
    let Some(ms) = timeout_ms else {
        return Ok((child.wait()?, false));
    };

    let deadline = Instant::now()
        .checked_add(Duration::from_millis(ms))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "timeout_ms overflows monotonic clock",
            )
        })?;
    let mut sleep_dur = Duration::from_millis(1);

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }

        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let status = child.wait()?;
            return Ok((status, true));
        }

        let remaining = deadline.saturating_duration_since(now);
        std::thread::sleep(sleep_dur.min(remaining));
        sleep_dur = (sleep_dur * 2).min(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn extract_signal(status: &ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt as _;

    status.signal().map(signal_name)
}

#[cfg(not(unix))]
fn extract_signal(_status: &ExitStatus) -> Option<String> {
    None
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        1 => "SIGHUP".to_owned(),
        2 => "SIGINT".to_owned(),
        3 => "SIGQUIT".to_owned(),
        4 => "SIGILL".to_owned(),
        5 => "SIGTRAP".to_owned(),
        6 => "SIGABRT".to_owned(),
        8 => "SIGFPE".to_owned(),
        9 => "SIGKILL".to_owned(),
        11 => "SIGSEGV".to_owned(),
        13 => "SIGPIPE".to_owned(),
        14 => "SIGALRM".to_owned(),
        15 => "SIGTERM".to_owned(),
        #[cfg(target_os = "linux")]
        10 => "SIGUSR1".to_owned(),
        #[cfg(target_os = "linux")]
        12 => "SIGUSR2".to_owned(),
        #[cfg(target_os = "macos")]
        30 => "SIGUSR1".to_owned(),
        #[cfg(target_os = "macos")]
        31 => "SIGUSR2".to_owned(),
        other => format!("SIG{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NodeCoreErrorCode;

    #[test]
    fn validates_program_args_and_env() {
        assert_eq!(
            spawn_sync("", &[], &SpawnOptions::default())
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidInput
        );
        assert_eq!(
            spawn_sync("echo\0bad", &[], &SpawnOptions::default())
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidInput
        );
        assert_eq!(
            spawn_sync("echo", &["a\0b".to_owned()], &SpawnOptions::default())
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidInput
        );
        let opts = SpawnOptions {
            env: Some(vec![("BAD=KEY".to_owned(), "v".to_owned())]),
            ..SpawnOptions::default()
        };
        assert_eq!(
            spawn_sync("echo", &[], &opts).unwrap_err().code(),
            NodeCoreErrorCode::InvalidInput
        );
    }

    #[test]
    fn missing_executable_returns_not_found() {
        let err = spawn_sync(
            "vaeljs_nonexistent_binary_999999",
            &[],
            &SpawnOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), NodeCoreErrorCode::NotFound);
    }

    #[test]
    fn huge_timeout_ms_returns_invalid_input_instead_of_panicking() {
        let err = deadline_from_duration(Instant::now(), Duration::MAX).unwrap_err();
        assert_eq!(err.code(), NodeCoreErrorCode::InvalidInput);

        let opts = SpawnOptions {
            timeout_ms: Some(u64::MAX),
            ..SpawnOptions::default()
        };
        if let Err(err) = spawn_sync("vaeljs_nonexistent_binary_999999", &[], &opts) {
            assert!(matches!(
                err.code(),
                NodeCoreErrorCode::InvalidInput | NodeCoreErrorCode::NotFound
            ));
        }
    }

    #[test]
    fn max_buffer_above_cap_is_rejected() {
        let opts = SpawnOptions {
            max_buffer: MAX_MAX_BUFFER + 1,
            ..SpawnOptions::default()
        };
        let err = spawn_sync("echo", &[], &opts).expect_err("oversized max_buffer must fail");
        assert_eq!(err.code(), NodeCoreErrorCode::InputTooLarge);
    }

    #[test]
    fn resolve_bare_program_in_path_skips_relative_and_cwd_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let abs_bin = tmp.path().join("bin");
        std::fs::create_dir_all(&abs_bin).unwrap();
        let tool_exe = abs_bin.join("mytool.exe");
        std::fs::write(&tool_exe, b"stub").unwrap();

        let pathext = OsStr::new(".com;.exe");
        let rel_path = OsStr::new(".;relative_dir");
        assert_eq!(
            resolve_bare_program_in_path("mytool", Some(rel_path), Some(pathext), ';'),
            None
        );

        let mixed = format!(".;{};{}", "relative_dir", abs_bin.display());
        let resolved =
            resolve_bare_program_in_path("mytool", Some(OsStr::new(&mixed)), Some(pathext), ';')
                .expect("should resolve from absolute PATH entry");
        assert_eq!(resolved, tool_exe);

        let with_ext = resolve_bare_program_in_path(
            "mytool.exe",
            Some(OsStr::new(&mixed)),
            Some(pathext),
            ';',
        )
        .expect("should resolve bare name with explicit extension");
        assert_eq!(with_ext, tool_exe);
        assert!(is_bare_program_name("mytool.exe"));
        assert!(!is_bare_program_name("./mytool.exe"));
        assert!(!is_bare_program_name(r"C:\bin\mytool.exe"));
    }
}
