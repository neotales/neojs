# vaeljs-exec-core

Engine-neutral synchronous and streaming process execution, executable `PATH` lookup, and
cross-platform OS advisory file locking core for `vael:exec`, designed to be shared by both
the V8 runtime (`vaeljs-core`) and the QuickJS runtime (`vaeljs-lite-core`).

This crate has **zero** Tokio, **zero** Hyper, and **zero** JavaScript engine dependencies
(`deno_core`, `v8`, `rquickjs` are absent). Its direct runtime dependencies are `thiserror`
(plus `libc` on Unix for targeted single-process `kill(pid, signo)`).

```sh
cargo test --offline -p vaeljs-exec-core
cargo clippy --offline -p vaeljs-exec-core --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --offline --no-deps -p vaeljs-exec-core
cargo tree --offline -p vaeljs-exec-core
```

## Surface

* **Argv-only command builder**: `CommandSpec`, `StdioMode`, `Signal`
* **Synchronous execution**: `exec_sync` -> `CommandOutput` (with `text()`, `lines()`,
  `error_text()`, `error_lines()`), `run_sync` -> `CommandStatus`
* **Managed streaming child**: `spawn` -> `SpawnedChild` (`pid`, `take_*`, `write_stdin`,
  `close_stdin`, `read_stdout`, `read_stderr`, `kill`, `try_wait`, `wait`, `wait_with_output`)
* **Executable lookup**: `which`, `which_all`
* **Advisory file locking**: `FileLock`, `LockOptions`, `LockMode`
* **Structured errors**: `ExecError`, `ExecErrorCode` (`VAEL_EXEC_*`)

## Security invariants

* **Zero implicit shell**: Arguments are passed directly via OS argv without `sh -c` or
  `cmd.exe /c`.
* **Safe bare-program resolution on Windows**: Searches `PATH` + `PATHEXT` while skipping
  empty/relative `PATH` entries and never implicitly searching the current working directory.
* **Bounded I/O & clean timeouts**: `stdout` and `stderr` are drained concurrently up to
  `max_output_bytes` (default `16 MiB`, max `64 MiB`), and timeouts escalate cleanly from
  `kill_signal` (`SIGTERM` by default) through `kill_grace` (`2s` by default) to `SIGKILL`.
* **Symlink-safe file locks**: `FileLock::acquire` rejects symlinks (`O_NOFOLLOW` on Unix)
  and uses Rust 1.100.0's safe `std::fs::File` advisory lock methods.
* **Minimal `unsafe`**: `#![deny(unsafe_code)]` at crate root, with `unsafe` confined solely
  to the Unix `libc::kill(pid, signo)` wrapper guarded against `pid <= 0`.
