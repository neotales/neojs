# neojs-lite-core

The primary NeoJS Lite interface: an embeddable QuickJS runtime for ECMAScript
module files. `neojs-lite` is a separate, secondary command-line wrapper around
this crate.

`neojs-lite-core` is built with `rquickjs`/QuickJS-NG. It is a separate crate
from the V8-based `neojs-core`; the workspace's default build does not compile
it and does not put QuickJS or `rquickjs` in the V8 graph.

## Embed a module file

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

A complete example is in `examples/embed.rs`:

```sh
cargo run -p neojs-lite-core --example embed -- examples/hello.js
```

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader. `console.log`
writes to standard output and `console.error` to standard error.

## Run an in-memory module

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_source("/app/main.js", "console.log('embedded');")
}
```

`run_source(name, source)` uses `name` as the module identity and the base for
relative imports. Pass an absolute path when the source imports a file; a
relative name resolves imports against the process working directory. The
in-memory entry is not read from disk and its own extension is not checked;
imported files still go through the loader and shared extension policy.

## Runtime introspection

The runtime installs a minimal `Neo` global. Its `engine` property is
`"neo-lite"`. With the default `neo-chars` and `neo-strings` features the
registry contains the canonical shared modules `neo:chars` and `neo:strings`.
`Neo.getBuiltInModulesList()` returns one frozen array of canonical specifiers,
and `Neo.getBuiltInModule(specifier)` returns the frozen registry object whose
named functions are the same function objects the module namespace exports. The
namespace itself is a distinct object. With `default-features = false` the
registry and list are empty and no native module is compiled in. A bare name,
another scheme, a query or fragment, or any unimplemented specifier returns
`undefined`; a non-string name throws `TypeError`. The `Neo` object and the
global property are frozen and non-writable, and the list is the same array for
every lookup.

```js
console.log(Neo.engine); // "neo-lite"
console.log(Neo.getBuiltInModulesList()); // [ "neo:chars", "neo:strings" ]
console.log(Neo.getBuiltInModule("neo:chars").isLetter(0x41)); // true
console.log(Neo.getBuiltInModule("neo:strings").equalFold("K", "\u212A")); // true
console.log(Neo.getBuiltInModule("chars")); // undefined
```

This is not the V8 runtime's full built-in surface. Only `neo:chars` and
`neo:strings` are implemented, and the list does not claim another module exists
or will exist. The Rust logic comes from the engine-neutral `neojs-builtins`
crate and is bound as native QuickJS modules; the V8 runtime uses the same
logic. The
Unicode version is fixed by the crate's tables rather than each engine; see
[`neojs-builtins/README.md`](../neojs-builtins/README.md) for the precise
limits.

## Lifecycle

A runtime evaluates one entry. A preflight failure (unsupported format,
unreadable file) leaves it unused, so another entry can be tried. Once
evaluation starts, the runtime is consumed even if evaluation, the job drain, or
an unhandled rejection fails. Create a new runtime for subsequent work.

After the entry promise settles, the runtime drains the remaining QuickJS job
queue, so a trailing `.then` or an unawaited dynamic import still runs. The
drain stops after 100,000 jobs and fails closed on a nonconverging chain. A
queued job that throws is an error. A promise still rejected without a handler
when the queue empties is an error; a rejection handled later in the queue is
not. Diagnostics do not execute script code: primitive values and an Error's own
`message`/`stack` data are used, never getters, proxies, `toString`, or `String`.

## Limitations

This crate does **not** claim parity with the V8-based `neojs`:

* **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason.
* **No host asynchrony.** There are no timers or host Promise integration. A
  top-level `await` works only when QuickJS's own job queue can settle it.
* **Most built-in `neo:` modules are absent.** With the default `neo-chars` and
  `neo-strings` features, only the shared `neo:chars` and `neo:strings` modules
  are implemented. Other V8 modules are not importable, and the `Neo` list does
  not claim them. With default features off, no built-in module is implemented.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions, so
`.js`/`.mjs`/extensionless names and the rejection reasons for `.cjs`, JSX/TSX,
declaration files, JSON, WebAssembly, and native addons are the same.

## Build features

A default build enables `neo-chars` and `neo-strings`, the shared `neo:chars`
and `neo:strings` modules and their canonical registry, loader, and
introspection-list entries. Each feature controls the optional `neojs-builtins`
dependency; with `default-features = false` the dependency, the implementation
source, the registry objects, the loader entries, and the list entries are all
absent. The list is then the frozen empty array.

```sh
# Default: both shared modules present
cargo test -p neojs-lite-core

# No built-in module: dependency and source absent
cargo test -p neojs-lite-core --no-default-features --test feature_flags

# One feature at a time
cargo test -p neojs-lite-core --no-default-features --features neo-chars --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-strings --test feature_flags
```

Cargo features are additive and unify across the dependency graph. A build that
another dependent enables a module feature for contains the module even if this
crate was requested without it. Feature selection is compile-time surface
selection, not a security boundary or sandbox; code that runs keeps the
process's authority. `neojs-lite` explicitly selects `neo-chars` and
`neo-strings` with `default-features = false`, so a change to this crate's
defaults cannot silently change what the command advertises.

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile
QuickJS or add `rquickjs` to the default graph. Build this runtime explicitly:

```sh
cargo build -p neojs-lite-core
cargo test -p neojs-lite-core -p neojs-lite
```

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `THIRD_PARTY_NOTICES.md`.
