//! Safe, bounded synchronous child process execution backing `node:child_process`.
//!
//! * Never invokes a shell (`sh -c` or `cmd.exe /c`): executes `program` directly.
//! * Drains `stdout` and `stderr` concurrently on scoped reader threads capped at
//!   [`SpawnOptions::max_buffer`] so noisy children cannot OOM the host or deadlock pipes.
//! * Writes optional `stdin` concurrently on a scoped writer thread.
//! * Enforces [`SpawnOptions::timeout_ms`] via `try_wait` polling, terminating and
//!   reaping timed-out children so no zombie processes are left behind.

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::error::{NodeCoreError, map_io_error};

/// Default capture cap per output stream (`10 MiB`).
pub const DEFAULT_MAX_BUFFER: usize = 10 * 1024 * 1024;

const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(75);

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

    let mut cmd = Command::new(program);
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

    let stdin_pipe = child.stdin.take();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let max_buf = opts.max_buffer;
    let truncated = Arc::new(AtomicBool::new(false));

    let stdin_signal = stdin_pipe
        .zip(opts.stdin.as_deref())
        .map(|(mut pipe, data)| {
            let payload = data.to_vec();
            let signal = Arc::new((Mutex::new(false), Condvar::new()));
            let worker_signal = Arc::clone(&signal);
            std::thread::spawn(move || {
                let _ = pipe.write_all(&payload);
                drop(pipe);
                let (lock, cvar) = &*worker_signal;
                let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
                *done = true;
                cvar.notify_all();
            });
            signal
        });

    let stdout_state = stdout_pipe.map(|pipe| spawn_reader(pipe, max_buf, Arc::clone(&truncated)));
    let stderr_state = stderr_pipe.map(|pipe| spawn_reader(pipe, max_buf, Arc::clone(&truncated)));

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

    if let Some(signal) = stdin_signal {
        wait_for_done(&signal, grace_deadline);
    }

    let mut read_error: Option<io::Error> = None;
    let stdout = if let Some(state) = stdout_state {
        let (buf, err) = collect_stream(&state, grace_deadline);
        if read_error.is_none() {
            read_error = err;
        }
        buf
    } else {
        Vec::new()
    };
    let stderr = if let Some(state) = stderr_state {
        let (buf, err) = collect_stream(&state, grace_deadline);
        if read_error.is_none() {
            read_error = err;
        }
        buf
    } else {
        Vec::new()
    };

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
    if let Some(ms) = opts.timeout_ms {
        deadline_from_duration(Instant::now(), Duration::from_millis(ms))?;
    }
    Ok(())
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

fn spawn_reader(
    mut reader: impl std::io::Read + Send + 'static,
    max_buffer: usize,
    truncated: Arc<AtomicBool>,
) -> Arc<(Mutex<StreamState>, Condvar)> {
    let shared = Arc::new((Mutex::new(StreamState::default()), Condvar::new()));
    let worker_shared = Arc::clone(&shared);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    let (lock, cvar) = &*worker_shared;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
                Ok(n) => {
                    let (lock, _) = &*worker_shared;
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
                    let (lock, cvar) = &*worker_shared;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.err = Some(err);
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
            }
        }
    });
    shared
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
}
