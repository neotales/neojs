# vaeljs-lite-core

The primary VaelJS Lite interface: an embeddable QuickJS runtime for ECMAScript
and erasable TypeScript module files. `vaeljs-lite` is a separate, secondary
command-line wrapper around this crate.

`vaeljs-lite-core` is built with `rquickjs`/QuickJS-NG and remains 100%
synchronous with zero `tokio`, zero `hyper`, and zero V8/`deno_core`. It is a
separate crate from the V8-based `vaeljs-core`; the workspace's default build
does not compile it and does not put QuickJS or `rquickjs` in the V8 graph.

## Embed a module file

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

A complete example is in `examples/embed.rs`:

```sh
cargo run -p vaeljs-lite-core --example embed -- examples/hello.js
```

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader.

With the default `remote` and `npm` features enabled:

* **Offline by default:** `Runtime::new()` / `RuntimeOptions::new()` keeps
  network fetches disabled (`remote = None`).
* **Remote & JSR (`remote`):** Opt in with `Runtime::with_remote(RemoteOptions::new())`
  or `RuntimeOptions::with_remote(...)` to load `https:`, `gh:`/`github:`, and
  `jsr:` entrypoints (`Runtime::run_url`) and imports via blocking `vaeljs-remote`
  (`ureq` + `rustls`, zero Tokio/hyper). Plaintext `http://` and credentialed
  URLs are rejected, and query/userinfo secrets are redacted from diagnostics.
* **npm & `package.json` (`npm`):** Supports `npm:` specifiers, nearest
  `package.json` (`dependencies`, `devDependencies`, `optionalDependencies`,
  `exports`, `#imports`, peer dependencies, self-references), content-verified
  CAS storage, `vael.lock` v1, and `.npmrc` scoped registries with HTTPS-only
  Bearer auth (`NpmOptions`, `RuntimeOptions::with_npm`). Never creates or reads
  `node_modules` and never runs lifecycle scripts.
* **Manifest import maps (`manifest`, opt-in):** Depend on `vaeljs-manifest`,
  build an `ImportResolver` with `discover_manifest` (`ImportResolver::from_manifest`
  or `Workspace::load` + `ImportResolver::from_workspace`), and pass it to
  `Runtime::with_import_resolver(resolver)` or `RuntimeOptions::with_import_resolver`
  to resolve `vael.jsonc` `imports` (local paths, `file:`, `https:`, `gh:`/`github:`,
  `jsr:`, `npm:`, `node:`) and workspace `@scope/pkg` references; `Runtime::new()`
  performs zero implicit manifest discovery.

`console.log` writes to standard output and `console.error` to standard error.

```rust
use vaeljs_lite_core::{ImportResolver, RemoteOptions, Runtime, RuntimeOptions};
use vaeljs_manifest::{Workspace, discover_manifest};

fn run_with_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let mut options = RuntimeOptions::new().with_remote(RemoteOptions::new());
    if let Some(found) = discover_manifest(".", None)? {
        let resolver = if !found.manifest.workspaces.is_empty() {
            let ws = Workspace::load(&found.path)?;
            let active = ws.member_for_path("main.js")?.map(|m| m.name());
            ImportResolver::from_workspace(&ws, active)?
        } else {
            ImportResolver::from_manifest(&found)
        };
        options = options.with_import_resolver(resolver);
    }
    Runtime::with_options(options)?.run_file("main.js")?;
    Ok(())
}
```

## Run an in-memory module

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
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
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("example.test.js")?;
    let report = runtime.run_tests()?;
    assert!(report.all_passed(), "{:?}", report.failures);
    Ok(())
}
```

`Runtime::run_tests` evaluates `Vael.getBuiltInModule("vael:test").run({ throwOnFailure: false, settle: true })`
and returns a `TestReport` (`total`, `passed`, `failed`, `pending`, `unhandled`,
`unsettled`, `failures`, and `all_passed()`). Runner faults, unresolved test
Promises, or disabled `vael-test` builds return `Error::TestRunner`.

## Runtime introspection

The runtime installs a minimal `Vael` global. Its `engine` property is
`"vael-lite"`. With default features enabled, the registry contains the
canonical shared modules `vael:assert`, `vael:chars`, `vael:console`, `vael:crypto`
(13 synchronous exports only), `vael:fs`, `vael:os`, `vael:path`, `vael:slices`,
`vael:ssh`, `vael:strings`, `vael:template`, `vael:test`, `vael:uuid`,
`vael:uuid/v4`, and `vael:uuid/v7`.
`Vael.getBuiltInModulesList()` returns one frozen array of canonical specifiers,
and `Vael.getBuiltInModule(specifier)` returns the cached registry object (frozen
for every module except `vael:console`, which is the live mutable
`globalThis.console` object) whose named exports share identity with the module
namespace. The namespace itself is a distinct object. With `default-features = false` the registry and list are
empty and no shared module is compiled in. A bare name, another scheme, a query
or fragment, or any unimplemented specifier returns `undefined`; a non-string
name throws `TypeError`. The `Vael` object and the global property are frozen and
non-writable, and the list is the same array for every lookup.

Canonical `vael:` (including ASCII case-insensitive `VAEL:` scheme) and `node:`
built-in specifiers are never shadowed by `vael.jsonc` or `package.json`.

This is not the V8 runtime's full built-in surface. Other V8 modules (`vael:age`,
`vael:cron`, `vael:db`, `vael:http`, `vael:stdio`, `node:fs/promises`, and
`*Async` crypto helpers) are not importable, and the `Vael` list does not claim
them. See
[`vaeljs-builtins/README.md`](../vaeljs-builtins/README.md),
[`vaeljs-crypto-core/README.md`](../vaeljs-crypto-core/README.md),
[`vaeljs-fs-core/README.md`](../vaeljs-fs-core/README.md),
[`vaeljs-node-core/README.md`](../vaeljs-node-core/README.md),
[`vaeljs-npm/README.md`](../vaeljs-npm/README.md),
[`vaeljs-remote/README.md`](../vaeljs-remote/README.md),
[`vaeljs-ssh-core/README.md`](../vaeljs-ssh-core/README.md), and
[`vaeljs-template-core/README.md`](../vaeljs-template-core/README.md).

## Lifecycle

A runtime evaluates one entry (`run_file`, `run_url`, or `run_source`). A
preflight failure (unsupported format, unreadable file, remote entry error, or
TypeScript strip failure) leaves it unused, so another entry can be tried. Once
evaluation starts, the runtime is consumed even if evaluation, the job drain, or
an unhandled rejection fails. Create a new runtime for subsequent work.

After the entry promise settles, the runtime drains the remaining QuickJS job
queue, so a trailing `.then` or an unawaited dynamic import still runs. The
drain stops after 100,000 jobs and fails closed on a nonconverging chain. A
queued job that throws is an error. A promise still rejected without a handler
when the queue empties is an error (or recorded in `TestReport.unhandled` when
`vael:test` is active); a rejection handled later in the queue is not.
Diagnostics do not execute script code: primitive values and an Error's own
`message`/`stack` data are used, never getters, proxies, `toString`, or `String`.

## Limitations

This crate does **not** claim parity with the V8-based `vaeljs`:

* **No non-erasable TypeScript, JSX, or type checking.** With the default
  `typescript` feature, `.ts` and `.mts` files run via `vaeljs-module-policy`'s
  erasable TypeScript stripper (`deno_ast`), preserving line and column numbers
  for QuickJS stack traces. Type errors are not reported, and constructs that
  require a transform (`enum`, `namespace`, parameter properties) are rejected.
  With `default-features = false` (or `typescript` disabled), `.ts` and `.mts`
  are rejected with `Error::UnsupportedEntry` / a loader error and `deno_ast`
  is omitted from the build.
* **No host timers or external asynchrony.** There are no timers and no event
  loop beyond QuickJS's own job queue. A top-level `await` works only when
  QuickJS's own job queue can settle it, and `test(..., { timeout })` is
  rejected explicitly with `VAEL_TEST_NO_TIMER`.
* **Most V8-only async/platform `vael:` modules are absent.** Only the shared
  synchronous `vael:` and `node:` modules listed above are implemented.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `vaeljs-module-policy` for executable-extension decisions and
erasable TypeScript stripping, so `.js`/`.mjs`/`.ts`/`.mts`/extensionless names
and the rejection reasons for `.cjs`, `.cts`, JSX/TSX, declaration files, JSON,
WebAssembly, and native addons are the same.

## Build features

A default build enables `vael-assert`, `vael-chars`, `vael-console`, `vael-crypto`,
`vael-fs`, `vael-os`, `vael-path`, `vael-slices`, `vael-ssh`, `vael-strings`,
`vael-template`, `vael-test`, `vael-uuid`, `node-compat`, `typescript`,
`remote`, and `npm`. The opt-in `manifest` feature enables `vaeljs-manifest` and
`Runtime::with_import_resolver`.

With `default-features = false`, all optional runtime dependencies
(`vaeljs-builtins`, `vaeljs-crypto-core`, `vaeljs-fs-core`, `vaeljs-manifest`,
`vaeljs-node-core`, `vaeljs-npm`, `vaeljs-remote`, `vaeljs-ssh-core`,
`vaeljs-template-core`, and `deno_ast`), built-in modules, remote/JSR/npm
loaders, and TypeScript stripping are omitted.

```sh
# Default: shared modules, node-compat, typescript, remote, and npm present
cargo test --offline --locked -p vaeljs-lite-core

# Minimal build: optional runtime crates omitted
cargo test --offline --locked -p vaeljs-lite-core --no-default-features

# Manifest + deterministic offline transport tests
cargo test --offline --locked -p vaeljs-lite-core --features "manifest,test-support"
```

Cargo features are additive and unify across the dependency graph. Selection is
compile-time surface selection, not a security boundary or sandbox. `vaeljs-lite`
selects its base features from `vaeljs-lite-core` with `default-features = false`
and exposes opt-outable `vael-template`, `node-compat`, `remote`, and `npm`
features.

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile
QuickJS or add `rquickjs` to the default graph. Build this runtime explicitly:

```sh
cargo build --offline --locked -p vaeljs-lite-core
cargo test --offline --locked -p vaeljs-lite-core -p vaeljs-lite
```

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `THIRD_PARTY_NOTICES.md`.
