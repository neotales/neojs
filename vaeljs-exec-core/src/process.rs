//! Argv-only synchronous and streaming child process execution for `vael:exec`.
//!
//! * Never invokes a shell (`sh -c` or `cmd.exe /c`).
//! * On Windows, resolves bare program names via `PATH` + `PATHEXT` while skipping
//!   relative entries and never implicitly searching the current working directory.
//! * Drains `stdout` and `stderr` concurrently on bounded threads up to
//!   `max_output_bytes` with zero pipe deadlock.
//! * Enforces `timeout` via graceful `kill_signal` -> `kill_grace` -> `SIGKILL`
//!   escalation and unblocks/joins reader threads cleanly on Unix via `UnixStream`
//!   shutdown so grandchildren inheriting FDs never leak background threads.

use std::io::{self, Read as _, Write as _};
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{ExecError, map_io_error};
use crate::types::{
    CommandOutput, CommandSpec, CommandStatus, EnvOp, MAX_OUTPUT_BYTES, Signal, StdioMode,
    validate_positive_duration,
};

const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(75);
#[cfg(unix)]
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(25);
const READ_CHUNK_CAP: usize = 64 * 1024;

/// Executes `spec` synchronously, capturing `stdout` and `stderr` concurrently.
///
/// Defaults unspecified `stdout`/`stderr` to [`StdioMode::Piped`] and unspecified
/// `stdin` to [`StdioMode::Null`]. Enforces `max_output_bytes` (failing with
/// [`crate::ExecErrorCode::OutputTooLarge`] if exceeded) and `timeout` with
/// graceful `kill_signal` -> `SigKill` escalation.
pub fn exec_sync(spec: &CommandSpec) -> Result<CommandOutput, ExecError> {
    let stdin_mode = spec.stdin.as_ref().unwrap_or(&StdioMode::Null);
    let stdout_mode = spec.stdout.as_ref().unwrap_or(&StdioMode::Piped);
    let stderr_mode = spec.stderr.as_ref().unwrap_or(&StdioMode::Piped);

    execute_sync_inner(spec, stdin_mode, stdout_mode, stderr_mode)
}

/// Executes `spec` synchronously with inherited standard streams by default.
///
/// Defaults unspecified `stdin`, `stdout`, and `stderr` to [`StdioMode::Inherit`].
/// Enforces `timeout` with graceful `kill_signal` -> `SigKill` escalation and
/// returns [`CommandStatus`].
pub fn run_sync(spec: &CommandSpec) -> Result<CommandStatus, ExecError> {
    let stdin_mode = spec.stdin.as_ref().unwrap_or(&StdioMode::Inherit);
    let stdout_mode = spec.stdout.as_ref().unwrap_or(&StdioMode::Inherit);
    let stderr_mode = spec.stderr.as_ref().unwrap_or(&StdioMode::Inherit);

    let out = execute_sync_inner(spec, stdin_mode, stdout_mode, stderr_mode)?;
    Ok(CommandStatus {
        code: out.code,
        signal: out.signal,
        success: out.success,
        timed_out: out.timed_out,
    })
}

/// Spawns `spec` as a managed [`SpawnedChild`] handle for streaming stdio and signal control.
///
/// Defaults unspecified `stdin`, `stdout`, and `stderr` to [`StdioMode::Piped`].
pub fn spawn(spec: &CommandSpec) -> Result<SpawnedChild, ExecError> {
    let mut cmd = build_command(spec)?;

    let stdin_mode = spec.stdin.as_ref().unwrap_or(&StdioMode::Piped);
    let stdout_mode = spec.stdout.as_ref().unwrap_or(&StdioMode::Piped);
    let stderr_mode = spec.stderr.as_ref().unwrap_or(&StdioMode::Piped);

    #[cfg(unix)]
    {
        let (stdin_pair, piped_stdin) = match stdin_mode {
            StdioMode::Inherit => {
                cmd.stdin(Stdio::inherit());
                (None, false)
            }
            StdioMode::Null => {
                cmd.stdin(Stdio::null());
                (None, false)
            }
            StdioMode::Piped => {
                let (parent_in, child_in) =
                    UnixStream::pair().map_err(|err| map_io_error("create stdin pipe", err))?;
                cmd.stdin(Stdio::from(OwnedFd::from(child_in)));
                (Some((parent_in, None)), true)
            }
            StdioMode::Bytes(bytes) => {
                let (parent_in, child_in) =
                    UnixStream::pair().map_err(|err| map_io_error("create stdin pipe", err))?;
                cmd.stdin(Stdio::from(OwnedFd::from(child_in)));
                (Some((parent_in, Some(bytes.clone()))), false)
            }
        };

        let stdout_stream = configure_unix_output_stdio(&mut cmd, stdout_mode, true)?;
        let stderr_stream = configure_unix_output_stdio(&mut cmd, stderr_mode, false)?;

        let child = cmd
            .spawn()
            .map_err(|err| map_io_error("spawn child process", err))?;
        drop(cmd);

        let stdin_cancel = Arc::new(AtomicBool::new(false));
        let (stdin_pipe, stdin_writer) = match stdin_pair {
            Some((stream, None)) if piped_stdin => (Some(stream), None),
            Some((stream, Some(bytes))) => (
                None,
                Some(spawn_unix_writer(stream, bytes, Arc::clone(&stdin_cancel))),
            ),
            _ => (None, None),
        };

        Ok(SpawnedChild {
            child,
            stdin_pipe,
            stdout_pipe: stdout_stream,
            stderr_pipe: stderr_stream,
            stdin_writer,
            stdin_cancel,
            kill_signal: spec.kill_signal,
            kill_grace: spec.kill_grace,
            status: None,
        })
    }

    #[cfg(not(unix))]
    {
        let stdin_bytes = match stdin_mode {
            StdioMode::Inherit => {
                cmd.stdin(Stdio::inherit());
                None
            }
            StdioMode::Piped => {
                cmd.stdin(Stdio::piped());
                None
            }
            StdioMode::Null => {
                cmd.stdin(Stdio::null());
                None
            }
            StdioMode::Bytes(bytes) => {
                cmd.stdin(Stdio::piped());
                Some(bytes.clone())
            }
        };

        configure_output_stdio(&mut cmd, stdout_mode, true)?;
        configure_output_stdio(&mut cmd, stderr_mode, false)?;

        let mut child = cmd
            .spawn()
            .map_err(|err| map_io_error("spawn child process", err))?;
        drop(cmd);

        let stdin_cancel = Arc::new(AtomicBool::new(false));
        let stdin_writer = if let Some(bytes) = stdin_bytes {
            child
                .stdin
                .take()
                .map(|pipe| spawn_generic_writer(pipe, bytes))
        } else {
            None
        };

        Ok(SpawnedChild {
            child,
            stdin_writer,
            stdin_cancel,
            kill_signal: spec.kill_signal,
            kill_grace: spec.kill_grace,
            status: None,
        })
    }
}

/// Managed child process handle returned by [`spawn`].
#[derive(Debug)]
pub struct SpawnedChild {
    child: Child,
    #[cfg(unix)]
    stdin_pipe: Option<UnixStream>,
    #[cfg(unix)]
    stdout_pipe: Option<UnixStream>,
    #[cfg(unix)]
    stderr_pipe: Option<UnixStream>,
    stdin_writer: Option<WorkerHandle<bool>>,
    stdin_cancel: Arc<AtomicBool>,
    kill_signal: Signal,
    kill_grace: Duration,
    status: Option<CommandStatus>,
}

impl Drop for SpawnedChild {
    fn drop(&mut self) {
        self.stdin_cancel.store(true, Ordering::Release);
        if let Some(writer) = self.stdin_writer.take() {
            writer.finish(true);
        }
    }
}

impl SpawnedChild {
    /// Returns the OS-assigned process identifier for this child.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Takes ownership of the child's piped standard input handle, if still present.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        #[cfg(unix)]
        {
            self.stdin_pipe
                .take()
                .map(|s| ChildStdin::from(OwnedFd::from(s)))
        }
        #[cfg(not(unix))]
        {
            self.child.stdin.take()
        }
    }

    /// Takes ownership of the child's piped standard output handle, if still present.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        #[cfg(unix)]
        {
            self.stdout_pipe
                .take()
                .map(|s| ChildStdout::from(OwnedFd::from(s)))
        }
        #[cfg(not(unix))]
        {
            self.child.stdout.take()
        }
    }

    /// Takes ownership of the child's piped standard error handle, if still present.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        #[cfg(unix)]
        {
            self.stderr_pipe
                .take()
                .map(|s| ChildStderr::from(OwnedFd::from(s)))
        }
        #[cfg(not(unix))]
        {
            self.child.stderr.take()
        }
    }

    /// Writes `bytes` to the child's piped standard input and flushes it.
    pub fn write_stdin(&mut self, bytes: &[u8]) -> Result<(), ExecError> {
        #[cfg(unix)]
        let stdin = self.stdin_pipe.as_mut().ok_or(ExecError::InvalidArgument {
            reason: "child stdin is not piped or already closed",
        })?;
        #[cfg(not(unix))]
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or(ExecError::InvalidArgument {
                reason: "child stdin is not piped or already closed",
            })?;
        stdin
            .write_all(bytes)
            .map_err(|err| map_io_error("write child stdin", err))?;
        stdin
            .flush()
            .map_err(|err| map_io_error("flush child stdin", err))
    }

    /// Closes the parent's handle to the child's standard input (sending EOF).
    pub fn close_stdin(&mut self) {
        #[cfg(unix)]
        if let Some(stream) = self.stdin_pipe.take() {
            let _ = stream.shutdown(std::net::Shutdown::Write);
            drop(stream);
        }
        #[cfg(not(unix))]
        drop(self.child.stdin.take());
    }

    /// Reads up to `max_bytes` currently available bytes from the child's piped `stdout`.
    ///
    /// Returns an empty `Vec` on EOF. Fails with [`crate::ExecErrorCode::InvalidArgument`]
    /// if `stdout` is not piped/already taken or if `max_bytes` exceeds [`MAX_OUTPUT_BYTES`].
    pub fn read_stdout(&mut self, max_bytes: usize) -> Result<Vec<u8>, ExecError> {
        if max_bytes > MAX_OUTPUT_BYTES {
            return Err(ExecError::InvalidArgument {
                reason: "max_bytes exceeds hard limit of 64 MiB",
            });
        }
        #[cfg(unix)]
        let stdout = self
            .stdout_pipe
            .as_mut()
            .ok_or(ExecError::InvalidArgument {
                reason: "child stdout is not piped or already taken",
            })?;
        #[cfg(not(unix))]
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or(ExecError::InvalidArgument {
                reason: "child stdout is not piped or already taken",
            })?;
        read_chunk(stdout, max_bytes, "read child stdout")
    }

    /// Reads up to `max_bytes` currently available bytes from the child's piped `stderr`.
    ///
    /// Returns an empty `Vec` on EOF. Fails with [`crate::ExecErrorCode::InvalidArgument`]
    /// if `stderr` is not piped/already taken or if `max_bytes` exceeds [`MAX_OUTPUT_BYTES`].
    pub fn read_stderr(&mut self, max_bytes: usize) -> Result<Vec<u8>, ExecError> {
        if max_bytes > MAX_OUTPUT_BYTES {
            return Err(ExecError::InvalidArgument {
                reason: "max_bytes exceeds hard limit of 64 MiB",
            });
        }
        #[cfg(unix)]
        let stderr = self
            .stderr_pipe
            .as_mut()
            .ok_or(ExecError::InvalidArgument {
                reason: "child stderr is not piped or already taken",
            })?;
        #[cfg(not(unix))]
        let stderr = self
            .child
            .stderr
            .as_mut()
            .ok_or(ExecError::InvalidArgument {
                reason: "child stderr is not piped or already taken",
            })?;
        read_chunk(stderr, max_bytes, "read child stderr")
    }

    /// Sends `signal` to the child process.
    ///
    /// No-ops if the child has already exited and been reaped (preventing PID reuse hazards).
    /// On Windows, [`Signal::SigTerm`] and [`Signal::SigKill`] terminate the process via
    /// `Child::kill`, while other signals fail with [`crate::ExecErrorCode::UnsupportedSignal`].
    pub fn kill(&mut self, signal: Signal) -> Result<(), ExecError> {
        if self.status.is_some() {
            return Ok(());
        }
        if let Some(status) = self
            .child
            .try_wait()
            .map_err(|err| map_io_error("poll child before kill", err))?
        {
            self.status = Some(status_from_exit(&status, false));
            return Ok(());
        }
        send_signal_to_child(&mut self.child, signal)
    }

    /// Non-blocking poll for child exit.
    pub fn try_wait(&mut self) -> Result<Option<CommandStatus>, ExecError> {
        if let Some(ref status) = self.status {
            return Ok(Some(status.clone()));
        }
        match self
            .child
            .try_wait()
            .map_err(|err| map_io_error("try_wait child process", err))?
        {
            Some(exit_status) => {
                let status = status_from_exit(&exit_status, false);
                self.status = Some(status.clone());
                Ok(Some(status))
            }
            None => Ok(None),
        }
    }

    /// Waits for the child process to exit and reaps it.
    pub fn wait(&mut self) -> Result<CommandStatus, ExecError> {
        if let Some(ref status) = self.status {
            return Ok(status.clone());
        }
        if let Some(writer) = self.stdin_writer.take() {
            writer.finish(false);
        }
        let exit_status = self
            .child
            .wait()
            .map_err(|err| map_io_error("wait for child process", err))?;
        let status = status_from_exit(&exit_status, false);
        self.status = Some(status.clone());
        Ok(status)
    }

    /// Closes any remaining `stdin` handle, concurrently drains `stdout` and `stderr`
    /// up to `max_output_bytes` per stream, and waits for exit (enforcing `timeout` if set).
    pub fn wait_with_output(
        mut self,
        max_output_bytes: usize,
        timeout: Option<Duration>,
    ) -> Result<CommandOutput, ExecError> {
        if max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(ExecError::InvalidArgument {
                reason: "max_output_bytes exceeds hard limit of 64 MiB",
            });
        }
        if let Some(dur) = timeout {
            validate_positive_duration(dur, "timeout")?;
        }
        let overall_deadline = timeout.and_then(|dur| Instant::now().checked_add(dur));

        // Close un-taken stdin so children awaiting EOF do not deadlock.
        self.close_stdin();

        let stdout_exceeded = Arc::new(AtomicBool::new(false));
        let stderr_exceeded = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));

        #[cfg(unix)]
        let stdout_handle = self.stdout_pipe.take().map(|pipe| {
            spawn_unix_reader(
                pipe,
                max_output_bytes,
                Arc::clone(&stdout_exceeded),
                Arc::clone(&cancel),
            )
        });
        #[cfg(not(unix))]
        let stdout_handle =
            self.child.stdout.take().map(|pipe| {
                spawn_generic_reader(pipe, max_output_bytes, Arc::clone(&stdout_exceeded))
            });

        #[cfg(unix)]
        let stderr_handle = self.stderr_pipe.take().map(|pipe| {
            spawn_unix_reader(
                pipe,
                max_output_bytes,
                Arc::clone(&stderr_exceeded),
                Arc::clone(&cancel),
            )
        });
        #[cfg(not(unix))]
        let stderr_handle =
            self.child.stderr.take().map(|pipe| {
                spawn_generic_reader(pipe, max_output_bytes, Arc::clone(&stderr_exceeded))
            });

        let wait_res = if let Some(status) = self.status.take() {
            Ok((None, Some(status)))
        } else {
            wait_with_timeout_and_escalation(
                &mut self.child,
                timeout,
                self.kill_signal,
                self.kill_grace,
            )
            .map(|(exit, timed_out)| (Some((exit, timed_out)), None))
        };

        let (exit_pair, cached_status, wait_err) = match wait_res {
            Ok((exit_pair, cached)) => (exit_pair, cached, None),
            Err(err) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
                (None, None, Some(err))
            }
        };

        let timed_out = exit_pair.as_ref().is_some_and(|(_, t)| *t)
            || cached_status.as_ref().is_some_and(|s| s.timed_out);
        let drain_deadline = if timed_out || wait_err.is_some() {
            Instant::now().checked_add(PIPE_DRAIN_GRACE)
        } else {
            overall_deadline
        };

        if let Some(writer) = self.stdin_writer.take() {
            if let Some(dl) = drain_deadline {
                wait_for_done(&writer.signal, Some(dl));
                self.stdin_cancel.store(true, Ordering::Release);
            }
            writer.finish(drain_deadline.is_some());
        }

        let mut read_error: Option<io::Error> = None;
        let stdout = if let Some(ref handle) = stdout_handle {
            let (buf, err) = collect_stream(&handle.signal, drain_deadline);
            if read_error.is_none() {
                read_error = err;
            }
            buf
        } else {
            Vec::new()
        };
        let stderr = if let Some(ref handle) = stderr_handle {
            let (buf, err) = collect_stream(&handle.signal, drain_deadline);
            if read_error.is_none() {
                read_error = err;
            }
            buf
        } else {
            Vec::new()
        };

        if drain_deadline.is_some() {
            cancel.store(true, Ordering::Release);
        }
        if let Some(handle) = stdout_handle {
            handle.finish(drain_deadline.is_some());
        }
        if let Some(handle) = stderr_handle {
            handle.finish(drain_deadline.is_some());
        }

        if let Some(err) = wait_err {
            return Err(map_io_error("wait for child process", err));
        }
        if stdout_exceeded.load(Ordering::Acquire) {
            return Err(ExecError::OutputTooLarge {
                limit: max_output_bytes,
                stream: "stdout",
            });
        }
        if stderr_exceeded.load(Ordering::Acquire) {
            return Err(ExecError::OutputTooLarge {
                limit: max_output_bytes,
                stream: "stderr",
            });
        }
        if let Some(err) = read_error
            && !timed_out
        {
            return Err(map_io_error("read child process output", err));
        }

        if let Some(cached) = cached_status {
            return Ok(CommandOutput {
                code: cached.code,
                signal: cached.signal,
                success: cached.success,
                stdout,
                stderr,
                timed_out: cached.timed_out,
                truncated: false,
            });
        }

        let (exit_status, did_timeout) = exit_pair.ok_or(ExecError::Io {
            context: "wait for child process",
            source: io::Error::other("child exit status unavailable"),
        })?;
        Ok(output_from_exit(
            &exit_status,
            stdout,
            stderr,
            did_timeout,
            false,
        ))
    }
}

fn read_chunk(
    reader: &mut impl io::Read,
    max_bytes: usize,
    context: &'static str,
) -> Result<Vec<u8>, ExecError> {
    if max_bytes == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; max_bytes.min(READ_CHUNK_CAP)];
    loop {
        match reader.read(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return Ok(buf);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(map_io_error(context, err)),
        }
    }
}

#[cfg(unix)]
fn configure_unix_output_stdio(
    cmd: &mut Command,
    mode: &StdioMode,
    is_stdout: bool,
) -> Result<Option<UnixStream>, ExecError> {
    let stream = match mode {
        StdioMode::Inherit => {
            if is_stdout {
                cmd.stdout(Stdio::inherit());
            } else {
                cmd.stderr(Stdio::inherit());
            }
            None
        }
        StdioMode::Null => {
            if is_stdout {
                cmd.stdout(Stdio::null());
            } else {
                cmd.stderr(Stdio::null());
            }
            None
        }
        StdioMode::Piped => {
            let (parent, child) =
                UnixStream::pair().map_err(|err| map_io_error("create output pipe", err))?;
            let stdio = Stdio::from(OwnedFd::from(child));
            if is_stdout {
                cmd.stdout(stdio);
            } else {
                cmd.stderr(stdio);
            }
            Some(parent)
        }
        StdioMode::Bytes(_) => {
            return Err(ExecError::InvalidArgument {
                reason: "StdioMode::Bytes is only valid for stdin",
            });
        }
    };
    Ok(stream)
}

#[cfg(not(unix))]
fn configure_output_stdio(
    cmd: &mut Command,
    mode: &StdioMode,
    is_stdout: bool,
) -> Result<(), ExecError> {
    let stdio = match mode {
        StdioMode::Inherit => Stdio::inherit(),
        StdioMode::Piped => Stdio::piped(),
        StdioMode::Null => Stdio::null(),
        StdioMode::Bytes(_) => {
            return Err(ExecError::InvalidArgument {
                reason: "StdioMode::Bytes is only valid for stdin",
            });
        }
    };
    if is_stdout {
        cmd.stdout(stdio);
    } else {
        cmd.stderr(stdio);
    }
    Ok(())
}

fn execute_sync_inner(
    spec: &CommandSpec,
    stdin_mode: &StdioMode,
    stdout_mode: &StdioMode,
    stderr_mode: &StdioMode,
) -> Result<CommandOutput, ExecError> {
    let mut cmd = build_command(spec)?;
    let max_bytes = spec.max_output_bytes;
    let stdout_exceeded = Arc::new(AtomicBool::new(false));
    let stderr_exceeded = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));

    #[cfg(unix)]
    let (mut child, stdin_handle, stdout_handle, stderr_handle) = {
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let stdin_pair = match stdin_mode {
            StdioMode::Inherit => {
                cmd.stdin(Stdio::inherit());
                None
            }
            StdioMode::Null | StdioMode::Piped => {
                cmd.stdin(Stdio::null());
                None
            }
            StdioMode::Bytes(bytes) => {
                let (parent_in, child_in) =
                    UnixStream::pair().map_err(|err| map_io_error("create stdin pipe", err))?;
                cmd.stdin(Stdio::from(OwnedFd::from(child_in)));
                Some((parent_in, bytes.clone()))
            }
        };

        let stdout_pair = match stdout_mode {
            StdioMode::Inherit => {
                cmd.stdout(Stdio::inherit());
                None
            }
            StdioMode::Null => {
                cmd.stdout(Stdio::null());
                None
            }
            StdioMode::Piped => {
                let (parent_out, child_out) =
                    UnixStream::pair().map_err(|err| map_io_error("create stdout pipe", err))?;
                cmd.stdout(Stdio::from(OwnedFd::from(child_out)));
                Some(parent_out)
            }
            StdioMode::Bytes(_) => {
                return Err(ExecError::InvalidArgument {
                    reason: "StdioMode::Bytes is only valid for stdin",
                });
            }
        };

        let stderr_pair = match stderr_mode {
            StdioMode::Inherit => {
                cmd.stderr(Stdio::inherit());
                None
            }
            StdioMode::Null => {
                cmd.stderr(Stdio::null());
                None
            }
            StdioMode::Piped => {
                let (parent_err, child_err) =
                    UnixStream::pair().map_err(|err| map_io_error("create stderr pipe", err))?;
                cmd.stderr(Stdio::from(OwnedFd::from(child_err)));
                Some(parent_err)
            }
            StdioMode::Bytes(_) => {
                return Err(ExecError::InvalidArgument {
                    reason: "StdioMode::Bytes is only valid for stdin",
                });
            }
        };

        let child = cmd
            .spawn()
            .map_err(|err| map_io_error("spawn child process", err))?;
        // Drop `cmd` immediately so parent never retains child-side FDs.
        drop(cmd);

        let stdin_handle =
            stdin_pair.map(|(pipe, data)| spawn_unix_writer(pipe, data, Arc::clone(&cancel)));
        let stdout_handle = stdout_pair.map(|pipe| {
            spawn_unix_reader(
                pipe,
                max_bytes,
                Arc::clone(&stdout_exceeded),
                Arc::clone(&cancel),
            )
        });
        let stderr_handle = stderr_pair.map(|pipe| {
            spawn_unix_reader(
                pipe,
                max_bytes,
                Arc::clone(&stderr_exceeded),
                Arc::clone(&cancel),
            )
        });

        (child, stdin_handle, stdout_handle, stderr_handle)
    };

    #[cfg(not(unix))]
    let (mut child, stdin_handle, stdout_handle, stderr_handle) = {
        let has_stdin_bytes = match stdin_mode {
            StdioMode::Inherit => {
                cmd.stdin(Stdio::inherit());
                None
            }
            StdioMode::Null | StdioMode::Piped => {
                cmd.stdin(Stdio::null());
                None
            }
            StdioMode::Bytes(bytes) => {
                cmd.stdin(Stdio::piped());
                Some(bytes.clone())
            }
        };
        configure_output_stdio(&mut cmd, stdout_mode, true)?;
        configure_output_stdio(&mut cmd, stderr_mode, false)?;

        let mut child = cmd
            .spawn()
            .map_err(|err| map_io_error("spawn child process", err))?;
        drop(cmd);

        let stdin_pipe = child.stdin.take();
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let stdin_handle = stdin_pipe
            .zip(has_stdin_bytes)
            .map(|(pipe, data)| spawn_generic_writer(pipe, data));
        let stdout_handle = stdout_pipe
            .map(|pipe| spawn_generic_reader(pipe, max_bytes, Arc::clone(&stdout_exceeded)));
        let stderr_handle = stderr_pipe
            .map(|pipe| spawn_generic_reader(pipe, max_bytes, Arc::clone(&stderr_exceeded)));

        (child, stdin_handle, stdout_handle, stderr_handle)
    };

    let overall_deadline = spec.timeout.and_then(|d| Instant::now().checked_add(d));
    let mut timed_out = false;
    let mut exit_status: Option<ExitStatus> = None;
    let mut wait_error: Option<io::Error> = None;

    match wait_with_timeout_and_escalation(
        &mut child,
        spec.timeout,
        spec.kill_signal,
        spec.kill_grace,
    ) {
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
        overall_deadline
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
        return Err(map_io_error("wait for child process", err));
    }
    if stdout_exceeded.load(Ordering::Acquire) {
        return Err(ExecError::OutputTooLarge {
            limit: max_bytes,
            stream: "stdout",
        });
    }
    if stderr_exceeded.load(Ordering::Acquire) {
        return Err(ExecError::OutputTooLarge {
            limit: max_bytes,
            stream: "stderr",
        });
    }
    if let Some(err) = read_error
        && !timed_out
    {
        return Err(map_io_error("read child process output", err));
    }

    let exit_status = exit_status.ok_or(ExecError::Io {
        context: "wait for child process",
        source: io::Error::other("child exit status unavailable"),
    })?;

    Ok(output_from_exit(
        &exit_status,
        stdout,
        stderr,
        timed_out,
        false,
    ))
}

fn build_command(spec: &CommandSpec) -> Result<Command, ExecError> {
    let resolved_program = resolve_program_for_spawn(spec)?;
    let mut cmd = Command::new(&resolved_program);
    cmd.args(&spec.args);

    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    if spec.clear_env {
        cmd.env_clear();
    }
    for op in &spec.env_ops {
        match op {
            EnvOp::Set(key, val) => {
                cmd.env(key, val);
            }
            EnvOp::Remove(key) => {
                cmd.env_remove(key);
            }
        }
    }
    Ok(cmd)
}

fn resolve_program_for_spawn(spec: &CommandSpec) -> Result<PathBuf, ExecError> {
    #[cfg(windows)]
    {
        if !crate::which::is_bare_program_name_win(&spec.program) {
            return Ok(PathBuf::from(&spec.program));
        }
        let env_lookup = |target: &str| -> Option<std::ffi::OsString> {
            let mut removed = false;
            let mut value: Option<String> = None;
            for op in &spec.env_ops {
                match op {
                    EnvOp::Set(k, v) if k.eq_ignore_ascii_case(target) => {
                        removed = false;
                        value = Some(v.clone());
                    }
                    EnvOp::Remove(k) if k.eq_ignore_ascii_case(target) => {
                        removed = true;
                        value = None;
                    }
                    _ => {}
                }
            }
            if let Some(v) = value {
                return Some(std::ffi::OsString::from(v));
            }
            if removed || spec.clear_env {
                return None;
            }
            std::env::var_os(target)
        };
        let path_var = env_lookup("PATH");
        let pathext_var = env_lookup("PATHEXT");
        crate::which::resolve_bare_program_in_path(
            &spec.program,
            path_var.as_deref(),
            pathext_var.as_deref(),
            ';',
        )
        .ok_or_else(|| ExecError::NotFound {
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "executable not found on absolute PATH entries",
            ),
        })
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from(&spec.program))
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn send_unix_signal(pid: u32, signal: Signal) -> Result<(), ExecError> {
    let raw_pid = libc::pid_t::try_from(pid).map_err(|_| ExecError::InvalidArgument {
        reason: "process id overflows pid_t",
    })?;
    if raw_pid <= 0 {
        return Err(ExecError::InvalidArgument {
            reason: "process id must be positive",
        });
    }
    let signo = signal.to_posix();
    // SAFETY: `raw_pid` is strictly validated > 0 above so `libc::kill` targets
    // only that single process (never 0, -1, or a negative process group), and
    // `signo` is a valid POSIX signal constant returned by `Signal::to_posix`.
    let rc = unsafe { libc::kill(raw_pid, signo) };
    if rc == 0 {
        Ok(())
    } else {
        Err(map_io_error(
            "send signal to child process",
            io::Error::last_os_error(),
        ))
    }
}

fn send_signal_to_child(child: &mut Child, signal: Signal) -> Result<(), ExecError> {
    #[cfg(unix)]
    {
        send_unix_signal(child.id(), signal)
    }
    #[cfg(not(unix))]
    {
        match signal {
            Signal::SigTerm | Signal::SigKill => child
                .kill()
                .map_err(|err| map_io_error("terminate child process", err)),
            Signal::SigInt
            | Signal::SigHup
            | Signal::SigQuit
            | Signal::SigUsr1
            | Signal::SigUsr2 => Err(ExecError::UnsupportedSignal {
                signal: signal.as_str(),
            }),
        }
    }
}

fn wait_with_timeout_and_escalation(
    child: &mut Child,
    timeout: Option<Duration>,
    kill_signal: Signal,
    kill_grace: Duration,
) -> io::Result<(ExitStatus, bool)> {
    let Some(dur) = timeout else {
        return Ok((child.wait()?, false));
    };

    let deadline = Instant::now()
        .checked_add(dur)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout overflows clock"))?;
    let mut sleep_dur = Duration::from_millis(1);

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }

        let remaining = deadline.saturating_duration_since(now);
        std::thread::sleep(sleep_dur.min(remaining));
        sleep_dur = (sleep_dur * 2).min(Duration::from_millis(10));
    }

    // Timeout reached: send `kill_signal` first if graceful escalation is configured.
    if kill_signal != Signal::SigKill
        && !kill_grace.is_zero()
        && send_signal_to_child(child, kill_signal).is_ok()
        && let Some(grace_deadline) = Instant::now().checked_add(kill_grace)
    {
        let mut grace_sleep = Duration::from_millis(1);
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok((status, true));
            }
            let now = Instant::now();
            if now >= grace_deadline {
                break;
            }
            let remaining = grace_deadline.saturating_duration_since(now);
            std::thread::sleep(grace_sleep.min(remaining));
            grace_sleep = (grace_sleep * 2).min(Duration::from_millis(10));
        }
    }

    // Escalate to SIGKILL / unconditional termination and reap.
    let _ = child.kill();
    let status = child.wait()?;
    Ok((status, true))
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

impl<T> std::fmt::Debug for WorkerHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHandle")
            .field("has_join", &self.join.is_some())
            .finish()
    }
}

impl<T> WorkerHandle<T> {
    fn finish(self, timed_out: bool) {
        #[cfg(unix)]
        let has_shutdown = if let Some(stream) = self.shutdown_stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            true
        } else {
            false
        };
        if let Some(join) = self.join {
            #[cfg(unix)]
            {
                if has_shutdown || !timed_out {
                    let _ = join.join();
                }
            }
            #[cfg(not(unix))]
            if !timed_out {
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
fn spawn_generic_writer(
    mut pipe: impl Write + Send + 'static,
    payload: Vec<u8>,
) -> WorkerHandle<bool> {
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
    max_bytes: usize,
    exceeded: Arc<AtomicBool>,
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
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let remaining = max_bytes.saturating_sub(state.buf.len());
                    if n > remaining {
                        state.buf.extend_from_slice(&chunk[..remaining]);
                        exceeded.store(true, Ordering::Release);
                        state.done = true;
                        cvar.notify_all();
                        drop(state);
                        drop(stream);
                        break;
                    }
                    state.buf.extend_from_slice(&chunk[..n]);
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
fn spawn_generic_reader(
    mut reader: impl io::Read + Send + 'static,
    max_bytes: usize,
    exceeded: Arc<AtomicBool>,
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
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let remaining = max_bytes.saturating_sub(state.buf.len());
                    if n > remaining {
                        state.buf.extend_from_slice(&chunk[..remaining]);
                        exceeded.store(true, Ordering::Release);
                        state.done = true;
                        cvar.notify_all();
                        drop(state);
                        drop(reader);
                        break;
                    }
                    state.buf.extend_from_slice(&chunk[..n]);
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
        #[cfg(unix)]
        shutdown_stream: None,
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
            if !guard.done && !remaining.is_zero() {
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

fn status_from_exit(status: &ExitStatus, timed_out: bool) -> CommandStatus {
    let (code, signal) = extract_code_and_signal(status);
    let success = status.success() && !timed_out;
    CommandStatus {
        code,
        signal,
        success,
        timed_out,
    }
}

fn output_from_exit(
    status: &ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
    truncated: bool,
) -> CommandOutput {
    let (code, signal) = extract_code_and_signal(status);
    let success = status.success() && !timed_out;
    CommandOutput {
        code,
        signal,
        success,
        stdout,
        stderr,
        timed_out,
        truncated,
    }
}

fn extract_code_and_signal(status: &ExitStatus) -> (i32, Option<String>) {
    if let Some(code) = status.code() {
        return (code, None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(sig) = status.signal() {
            return (128 + sig, Some(signal_name(sig)));
        }
    }
    (-1, None)
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        libc::SIGHUP => "SIGHUP".to_owned(),
        libc::SIGINT => "SIGINT".to_owned(),
        libc::SIGQUIT => "SIGQUIT".to_owned(),
        libc::SIGILL => "SIGILL".to_owned(),
        libc::SIGTRAP => "SIGTRAP".to_owned(),
        libc::SIGABRT => "SIGABRT".to_owned(),
        libc::SIGFPE => "SIGFPE".to_owned(),
        libc::SIGKILL => "SIGKILL".to_owned(),
        libc::SIGSEGV => "SIGSEGV".to_owned(),
        libc::SIGPIPE => "SIGPIPE".to_owned(),
        libc::SIGALRM => "SIGALRM".to_owned(),
        libc::SIGTERM => "SIGTERM".to_owned(),
        libc::SIGUSR1 => "SIGUSR1".to_owned(),
        libc::SIGUSR2 => "SIGUSR2".to_owned(),
        other => format!("SIG{other}"),
    }
}
