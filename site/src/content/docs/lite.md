---
title: NeoJS Lite (QuickJS)
description: The embeddable QuickJS runtime, its lifecycle, and the features it deliberately does not implement.
sidebar:
  order: 2
---

`neojs-lite-core` is the primary NeoJS Lite interface: an embeddable QuickJS runtime for ECMAScript
module files. `neojs-lite` is a separate, secondary command-line wrapper around that crate.

Both are built with `rquickjs`/QuickJS-NG. They are separate from the V8-based `neojs-core`; the
workspace's default build does not compile them and does not put QuickJS or `rquickjs` in the V8
graph.

## Embed a module file

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

A complete example is in `neojs-lite-core/examples/embed.rs`:

```sh
cargo run -p neojs-lite-core --example embed -- examples/hello.js
```

Relative imports, absolute paths, and `file:` URLs resolve from the importing module. Static imports
and dynamic `import()` use the same loader. `console.log` writes to standard output and
`console.error` to standard error.

## Run an in-memory module

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_source("/app/main.js", "console.log('embedded');")
}
```

`run_source(name, source)` uses `name` as the module identity and the base for relative imports.

- Pass an absolute path when the source imports a file.
- A relative name resolves imports against the process working directory.
- The in-memory entry is not read from disk and its own extension is not checked.
- Imported files still go through the loader and shared extension policy.

## Run from the shell

```sh
cargo run -p neojs-lite -- hello.js
```

```js
const name = "neojs-lite";
console.log(`hello from ${name}`);
```

The command maps module and job failures to exit 1 and preflight failures to exit 2, matching the
V8 command's classes.

## Lifecycle

A runtime evaluates one entry.

- A preflight failure (unsupported format, unreadable file) leaves it unused, so another entry can be
  tried.
- Once evaluation starts, the runtime is consumed even if evaluation, the job drain, or an unhandled
  rejection fails. Create a new runtime for subsequent work.
- After the entry promise settles, the runtime drains the remaining QuickJS job queue, so a trailing
  `.then` or an unawaited dynamic import still runs.
- The drain stops after 100,000 jobs and fails closed on a nonconverging chain.
- A queued job that throws is an error.
- A promise still rejected without a handler when the queue empties is an error; a rejection handled
  later in the queue is not.
- Diagnostics do not execute script code: primitive values and an Error's own `message`/`stack` data
  are used, never getters, proxies, `toString`, or `String`.

## Limitations

NeoJS Lite does **not** claim parity with the V8-based `neojs`:

- **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason. Use `.js` or `.mjs`.
- **No host asynchrony.** There are no timers or host Promise integration. A top-level `await` works
  only when QuickJS's own job queue can settle it.
- **No built-in `neo:` modules.** Local ECMAScript files only.
- **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions, so
`.js`/`.mjs`/extensionless names and the rejection reasons for `.cjs`, JSX/TSX, declaration files,
JSON, WebAssembly, and native addons are the same.

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile QuickJS or add
`rquickjs` to the default graph. Build the Lite runtime explicitly:

```sh
cargo build -p neojs-lite-core
cargo build -p neojs-lite
cargo test -p neojs-lite-core -p neojs-lite
```

## Source

- `neojs-lite-core/src/lib.rs`
- `neojs-lite-core/src/runtime.rs`
- `neojs-lite-core/src/loader.rs`
- `neojs-lite/src/main.rs`
- `neojs-module-policy/src/lib.rs`

See [V8 and Lite compared](/v8-vs-lite/) for a feature table.
