# neoltjs

Runs ECMAScript modules on QuickJS, as the separate NeoLTJS runtime in this
workspace. It is deliberately smaller than the V8-based `neojs` command.

## Run a module

```sh
cargo run -p neoltjs -- hello.js
```

```js
const name = "neoltjs";
console.log(`hello from ${name}`);
```

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader. `console.log`
writes to standard output and `console.error` to standard error.

After the entry promise settles, the runtime drains the remaining QuickJS job
queue, so a trailing `.then` or an unawaited dynamic import still runs. The
drain stops after 100,000 jobs and fails closed on a nonconverging chain. A
queued job that throws is an error. A promise still rejected without a handler
when the queue empties is an error; a rejection handled later in the queue is
not.

## Embedding

```rust
use neoltjs_core::Runtime;

fn main() -> Result<(), neoltjs_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

`run_source(name, source)` evaluates an in-memory module. `name` is the module
identity and the base for relative imports: use an absolute path when the source
imports a file, because a relative name resolves imports against the process
working directory. The in-memory entry itself is not read from disk and its own
extension is not checked; imported files still go through the loader and shared
extension policy.

A runtime evaluates one entry. A preflight failure (unsupported format,
unreadable file) leaves it unused, so another entry can be tried. Once
evaluation starts, the runtime is consumed even if evaluation, the job drain, or
an unhandled rejection fails. Create a new runtime for subsequent work.

## Differences from neojs (V8)

This crate does **not** claim parity with `neojs`:

* **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason. Use
  `.js` or `.mjs`.
* **No host asynchrony.** There are no timers or host Promise integration. A
  top-level `await` works only when QuickJS's own job queue can settle it.
* **No built-in `neo:` modules.** Local ECMAScript files only.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions, so
`.js`/`.mjs`/extensionless names and the rejection reasons for `.cjs`, JSX/TSX,
declaration files, JSON, WebAssembly, and native addons are the same.

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile
QuickJS or add `rquickjs` to the default graph. Build this runtime explicitly:

```sh
cargo build -p neoltjs
cargo test -p neoltjs-core -p neoltjs
```

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `neoltjs-core/THIRD_PARTY_NOTICES.md`.
