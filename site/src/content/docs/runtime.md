---
title: V8 runtime (neojs-core)
description: Embed the V8 runtime, run a module synchronously or asynchronously, and understand its lifecycle.
sidebar:
  order: 1
---

`neojs-core` is the library behind the `neojs` command. It exposes the same loader and console to
Rust callers and runs ECMAScript modules and erasable TypeScript on V8 through Deno's Rust
bindings.

## Install

```sh
cargo add neojs-core
```

The `neojs` command is built from this workspace:

```sh
cargo install --path neojs
```

## Run a file

```rust
use neojs_core::Runtime;

fn main() -> Result<(), neojs_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("hello.js")?;
    Ok(())
}
```

`run_file` returns an error if compilation, resolution, linking, or evaluation fails. It prints
nothing itself; the module writes through `console`, `stdio`, or host output.

## Run asynchronously

Use `Runtime::run_file_async` inside an existing Tokio runtime, or on any other executor:

```rust
use neojs_core::Runtime;

async fn run() -> Result<(), neojs_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file_async("hello.js").await?;
    Ok(())
}
```

A runtime owns a Tokio driver with one worker thread for V8's delayed foreground tasks and the
bounded `crypto` blocking pool, plus a current-thread runtime that drives `run_file` and
`deno_core`'s async-op driver. The async form does not borrow the caller's runtime.

It does not offload module loading: entry and imported files are read and type-stripped
synchronously on the thread that polls the future, and module compilation and evaluation run there
too.

`Runtime::run_file` returns `Error::BlockingInAsyncContext` inside a Tokio runtime instead of
blocking, unless the runtime is already consumed.

### Async crypto executor

`crypto`'s `hashAsync`, `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, `bcryptVerifyAsync`,
`shamirSplitAsync`, and `shamirCombineAsync` need a Tokio **current-thread** runtime to host the
async-op driver:

- They work under `run_file`.
- Under `run_file_async`, they work only when the caller's executor is a Tokio current-thread
  runtime.
- Otherwise they throw `NEO_CRYPTO_NO_EXECUTOR`.
- The synchronous hash, HMAC, PBKDF2, bcrypt, and Shamir API still works.

## Lifecycle

- A `Runtime` evaluates **one** entry module.
- Once loading starts, another call returns `Error::RuntimeConsumed`, including after a load or
  evaluation failure. Create a new `Runtime` for each entry.
- The async form does work only when polled. Dropping it before the first poll leaves the runtime
  unused, and so does a preflight failure such as an unsupported format or unreadable entry.
- Once loading starts, cancelling it leaves the runtime consumed, and the module may already have
  run and printed.
- Create the `Runtime` on the thread that will use it; it is not `Send`.

## Limits

There is no timeout or interrupt. A JavaScript loop that never exits hangs the process. The
runtime has no permission system and is not a sandbox; the loader reads any file the process can
read.

V8 supplies the ECMAScript built-ins. NeoJS adds the [built-in modules](/modules/) and the `Neo`
registry. It does not add `process`, `fetch`, `URL`, `TextEncoder`, a REPL, stdin input, workers, or
timers as part of its documented surface.

`Deno.core` internals are reachable from JavaScript, but they are not a supported API.

## Source

- `neojs-core/src/lib.rs`
- `neojs-core/src/runtime.rs`
- `neojs-core/src/loader.rs`
- `neojs-core/src/remote.rs`
- `neojs/src/main.rs`

The repository README is the canonical, fuller description. See also [Remote loading and
security](/remote-security/) and [Limits and unsupported features](/limits/).
