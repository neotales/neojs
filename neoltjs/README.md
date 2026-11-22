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

## Embedding

```rust
use neoltjs_core::Runtime;

fn main() -> Result<(), neoltjs_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

`neoltjs-core` exposes only the file-oriented runtime today. A runtime evaluates
one entry; create another for subsequent work.

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
