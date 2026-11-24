# neojs-lite-core

The primary NeoJS Lite interface: an embeddable QuickJS runtime for ECMAScript
and erasable TypeScript module files. `neojs-lite` is a separate, secondary
command-line wrapper around this crate.

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
module. Static imports and dynamic `import()` use the same loader. With the
opt-in `manifest` feature enabled, embedders can depend on `neojs-manifest`,
build an `ImportResolver` with `discover_manifest` (`ImportResolver::from_manifest`
or `Workspace::load` + `ImportResolver::from_workspace`), and pass it to
`Runtime::with_import_resolver(resolver)` to resolve `neo.jsonc` `imports` and
workspace `@scope/pkg` references to local `.js`/`.mjs`/`.ts`/`.mts` modules (remote
`https:`, `gh:`, `jsr:`, and `npm:` targets are rejected); `Runtime::new()`
performs zero implicit manifest discovery. `console.log` writes to standard
output and `console.error` to standard error.

```rust
use neojs_lite_core::{ImportResolver, Runtime};
use neojs_manifest::{Workspace, discover_manifest};

fn run_with_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = match discover_manifest(".", None)? {
        Some(found) if !found.manifest.workspaces.is_empty() => {
            let ws = Workspace::load(&found.path)?;
            let active = ws.member_for_path("main.js")?.map(|m| m.name());
            Runtime::with_import_resolver(ImportResolver::from_workspace(&ws, active)?)?
        }
        Some(found) => Runtime::with_import_resolver(ImportResolver::from_manifest(&found))?,
        None => Runtime::new()?,
    };
    runtime.run_file("main.js")?;
    Ok(())
}
```

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

## Run registered tests

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("example.test.js")?;
    let report = runtime.run_tests()?;
    assert!(report.all_passed(), "{:?}", report.failures);
    Ok(())
}
```

`Runtime::run_tests` evaluates `Neo.getBuiltInModule("neo:test").run({ throwOnFailure: false, settle: true })`
and returns a [`TestReport`] (`total`, `passed`, `failed`, `pending`, `unhandled`,
`unsettled`, `failures`, and `all_passed()`). Runner faults, unresolved test
Promises, or disabled `neo-test` builds return `Error::TestRunner`.

## Runtime introspection

The runtime installs a minimal `Neo` global. Its `engine` property is
`"neo-lite"`. With the default `neo-assert`, `neo-chars`, `neo-crypto`, `neo-fs`,
`neo-os`, `neo-ssh`, `neo-strings`, `neo-test`, and `neo-uuid` features the
registry contains the canonical shared modules `neo:assert`, `neo:chars`,
`neo:crypto` (13 synchronous exports only), `neo:fs`, `neo:os`, `neo:ssh`,
`neo:strings`, `neo:test`, `neo:uuid`, `neo:uuid/v4`, and `neo:uuid/v7`.
`Neo.getBuiltInModulesList()` returns one frozen array of canonical specifiers,
and `Neo.getBuiltInModule(specifier)` returns the frozen registry object whose
named exports share identity with the module namespace. The namespace itself is
a distinct object. With `default-features = false` the registry and list are
empty and no shared module is compiled in. A bare name, another scheme, a query
or fragment, or any unimplemented specifier returns `undefined`; a non-string
name throws `TypeError`. The `Neo` object and the global property are frozen and
non-writable, and the list is the same array for every lookup.

```js
console.log(Neo.engine); // "neo-lite"
console.log(Neo.getBuiltInModulesList());
// [ "neo:assert", "neo:chars", "neo:crypto", "neo:fs", "neo:os", "neo:ssh", "neo:strings", "neo:test", "neo:uuid", "neo:uuid/v4", "neo:uuid/v7" ]
console.log(Neo.getBuiltInModule("neo:chars").isLetter(0x41)); // true
console.log(Neo.getBuiltInModule("neo:crypto").randomBytes(16).length); // 16
console.log(Neo.getBuiltInModule("neo:fs").exists(".")); // true
console.log(Neo.getBuiltInModule("neo:os").platform); // e.g. "linux"
console.log(Neo.getBuiltInModule("neo:ssh").MAX_COMMAND_BYTES); // 65536
console.log(Neo.getBuiltInModule("neo:strings").equalFold("K", "\u212A")); // true
console.log(Neo.getBuiltInModule("neo:uuid").v4()); // e.g. "919108f7-52d1-4320-9bac-f847db4148a8"
console.log(Neo.getBuiltInModule("chars")); // undefined
```

This is not the V8 runtime's full built-in surface. Only `neo:assert`,
`neo:chars`, `neo:crypto` (13 synchronous exports; no `*Async` functions),
`neo:fs`, `neo:os`, `neo:ssh`, `neo:strings`, `neo:test`, `neo:uuid`,
`neo:uuid/v4`, and `neo:uuid/v7` are implemented, and the list does not claim
another module exists or will exist. The shared logic comes from `neojs-builtins`
(`chars`, `os`, `strings`, and `uuid` bound as native QuickJS modules, `assert`
and `test` evaluated from shared ECMAScript factories with native QuickJS Promise
and queue hooks), `neojs-crypto-core` (`crypto` synchronous exports bound as a
native QuickJS module), `neojs-fs-core` (`fs` bound as a native QuickJS module),
and `neojs-ssh-core` (`ssh` bound as a native QuickJS module). See
[`neojs-builtins/README.md`](../neojs-builtins/README.md),
[`neojs-crypto-core/README.md`](../neojs-crypto-core/README.md),
[`neojs-fs-core/README.md`](../neojs-fs-core/README.md), and
[`neojs-ssh-core/README.md`](../neojs-ssh-core/README.md) for the precise limits.

## Lifecycle

A runtime evaluates one entry. A preflight failure (unsupported format,
unreadable file, or TypeScript strip failure) leaves it unused, so another entry
can be tried. Once evaluation starts, the runtime is consumed even if evaluation,
the job drain, or an unhandled rejection fails. Create a new runtime for
subsequent work.

After the entry promise settles, the runtime drains the remaining QuickJS job
queue, so a trailing `.then` or an unawaited dynamic import still runs. The
drain stops after 100,000 jobs and fails closed on a nonconverging chain. A
queued job that throws is an error. A promise still rejected without a handler
when the queue empties is an error (or recorded in `TestReport.unhandled` when
`neo:test` is active); a rejection handled later in the queue is not.
Diagnostics do not execute script code: primitive values and an Error's own
`message`/`stack` data are used, never getters, proxies, `toString`, or `String`.

## Limitations

This crate does **not** claim parity with the V8-based `neojs`:

* **No non-erasable TypeScript, JSX, or type checking.** With the default
  `typescript` feature, `.ts` and `.mts` files run via `neojs-module-policy`'s
  erasable TypeScript stripper (`deno_ast`), preserving line and column numbers
  for QuickJS stack traces. Type errors are not reported, and constructs that
  require a transform (`enum`, `namespace`, parameter properties) are rejected.
  With `default-features = false` (or `typescript` disabled), `.ts` and `.mts`
  are rejected with `Error::UnsupportedEntry` / a loader error and `deno_ast`
  is omitted from the build.
* **No host timers or external asynchrony.** There are no timers and no event
  loop beyond QuickJS's own job queue. A top-level `await` works only when
  QuickJS's own job queue can settle it, and `test(..., { timeout })` is
  rejected explicitly with `NEO_TEST_NO_TIMER`.
* **Most built-in `neo:` modules are absent.** With the default features, only
  the shared `neo:assert`, `neo:chars`, `neo:crypto` (13 synchronous exports),
  `neo:fs`, `neo:os`, `neo:ssh`, `neo:strings`, `neo:test`, `neo:uuid`,
  `neo:uuid/v4`, and `neo:uuid/v7` modules are implemented. Other V8 modules
  (and `*Async` crypto helpers) are not importable, and the `Neo` list does not
  claim them. With default features off, no built-in module is implemented.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions and
erasable TypeScript stripping, so `.js`/`.mjs`/`.ts`/`.mts`/extensionless names
and the rejection reasons for `.cjs`, `.cts`, JSX/TSX, declaration files, JSON,
WebAssembly, and native addons are the same.

## Build features

A default build enables `neo-assert`, `neo-chars`, `neo-crypto`, `neo-fs`,
`neo-os`, `neo-ssh`, `neo-strings`, `neo-test`, `neo-uuid`, and `typescript`,
along with the canonical registry, loader, introspection-list entries, and
`Runtime::run_tests`. The opt-in `manifest` feature enables `neojs-manifest` and
`Runtime::with_import_resolver`. Each feature controls its optional dependency
(`neojs-builtins`, `neojs-crypto-core`, `neojs-fs-core`, `neojs-ssh-core`,
`neojs-manifest`, or `neojs-module-policy/typescript`); with
`default-features = false` those dependencies (along with `aes`, `aes-gcm`,
`bcrypt`, `cbc`, `hmac`, `pbkdf2`, `sha2`, `neotales-ssh`, `deno_ast`,
`getrandom`, `uuid`, `glob`, `semver`, and `serde_json`), the implementation
source, the registry objects, the loader entries, and the list entries are all
absent. The list is then the frozen empty array.

```sh
# Default: all shared modules present
cargo test -p neojs-lite-core

# No built-in module: dependency and source absent
cargo test -p neojs-lite-core --no-default-features --test feature_flags

# One feature at a time
cargo test -p neojs-lite-core --no-default-features --features neo-assert --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-chars --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-crypto --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-fs --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-os --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-ssh --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-strings --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-test --test feature_flags
cargo test -p neojs-lite-core --no-default-features --features neo-uuid --test feature_flags
```

Cargo features are additive and unify across the dependency graph. A build that
another dependent enables a module or TypeScript feature for contains it even if
this crate was requested without it. Feature selection is compile-time surface
selection, not a security boundary or sandbox; code that runs keeps the
process's authority. `neojs-lite` explicitly selects `neo-assert`, `neo-chars`,
`neo-crypto`, `neo-fs`, `neo-os`, `neo-ssh`, `neo-strings`, `neo-test`,
`neo-uuid`, and `typescript` with `default-features = false`, so a change to
this crate's defaults cannot silently change what the command advertises.

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
