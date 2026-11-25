---
title: Embedding vaeljs-lite
description: Embed QuickJS-NG in Rust with vaeljs-lite-core, granular Cargo feature flags, RuntimeOptions, and zero V8/Tokio/Hyper dependencies.
---

**`vaeljs-lite-core`** is the primary interface for VaelJS Lite: a compact, embeddable QuickJS-NG (`rquickjs 0.14.0`) runtime for ECMAScript modules and erasable TypeScript with **zero V8, Tokio, or Hyper dependencies**.

`vaeljs-lite-core` is isolated from the workspace's default V8 members, so normal workspace builds (`cargo build`) never pull QuickJS into the V8 dependency graph.

## Basic Embedding

Add `vaeljs-lite-core` as a path (or workspace) dependency (`publish = false` during alpha):

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("examples/hello.js")
}
```

Run the included workspace example:

```sh
cargo run --locked -p vaeljs-lite-core --example embed -- examples/hello.js
```

## Running In-Memory Modules (`run_source`)

Use `run_source(name, source)` to evaluate an in-memory ES module without reading the entry from disk:

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_source(
        "/app/main.js",
        r#"
        import { v4 } from "vael:uuid";
        console.log("job id:", v4());
        "#,
    )
}
```

- `name` sets the module identity and base directory for relative imports (pass an absolute path when the module imports local files).
- The in-memory entry string's own extension is not checked, while any imported files still go through `vaeljs-module-policy`.

## Running Tests from Rust (`run_tests`)

Register tests in JS/TS with `vael:test` and retrieve structured counts via `Runtime::run_tests()`:

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("example.test.ts")?;
    let report = runtime.run_tests()?;
    assert!(report.all_passed(), "failures: {:?}", report.failures);
    Ok(())
}
```

`TestReport` exposes `total`, `passed`, `failed`, `pending`, `unhandled`, `unsettled`, `failures: Vec<String>`, and `all_passed()`. Requires the `vael-test` feature.

## `RuntimeOptions` & `vael.jsonc` Manifests

`Runtime::new()` performs **zero implicit `vael.jsonc` discovery** and enables `node:*` modules when `node-compat` is compiled in. Customize behavior with `RuntimeOptions`:

```rust
use vaeljs_lite_core::{ImportResolver, Runtime, RuntimeOptions};
use vaeljs_manifest::{Workspace, discover_manifest};

fn run_configured() -> Result<(), Box<dyn std::error::Error>> {
    let mut options = RuntimeOptions::new().with_node_compat(false);

    if let Some(found) = discover_manifest(".", None)? {
        let resolver = if found.manifest.workspaces.is_empty() {
            ImportResolver::from_manifest(&found)
        } else {
            let ws = Workspace::load(&found.path)?;
            let active = ws.member_for_path("main.ts")?.map(|m| m.name());
            ImportResolver::from_workspace(&ws, active)?
        };
        options = options.with_import_resolver(resolver);
    }

    let mut runtime = Runtime::with_options(options)?;
    runtime.run_file("main.ts")?;
    if let Some(code) = runtime.exit_code() {
        println!("script requested exit code {code}");
    }
    Ok(())
}
```

`with_import_resolver` requires the opt-in `manifest` feature on `vaeljs-lite-core`.

## Granular Cargo Feature Flags

With `default-features = false`, `vaeljs-lite-core` omits all `vaeljs-builtins`, `vaeljs-crypto-core`, `vaeljs-fs-core`, `vaeljs-ssh-core`, `vaeljs-template-core`, `vaeljs-node-core`, and `deno_ast` (`typescript`) dependencies—leaving `Vael.getBuiltInModulesList()` as an empty frozen array and keeping only a minimal `console.{log,error}` shim.

| Feature | Default | Enables |
| --- | --- | --- |
| `vael-assert` | Yes | `vael:assert` + Promise observation hooks |
| `vael-chars` | Yes | `vael:chars` Unicode character predicates |
| `vael-console` | Yes | `vael:console` and formatted `console.{log,debug,info,warn,error}` |
| `vael-crypto` | Yes | `vael:crypto` (13 synchronous exports via `vaeljs-crypto-core`) |
| `vael-fs` | Yes | `vael:fs` synchronous filesystem ops (`vaeljs-fs-core`) |
| `vael-os` | Yes | `vael:os` (`platform`, `arch`) |
| `vael-path` | Yes | `vael:path` lexical path helpers |
| `vael-slices` | Yes | `vael:slices` (`Slice`, `CharSlice`; implies `vael-chars`) |
| `vael-ssh` | Yes | `vael:ssh` synchronous SSH client (`vaeljs-ssh-core`) |
| `vael-strings` | Yes | `vael:strings` ASCII case-folding & trimming |
| `vael-template` | Yes | `vael:template` Go `text/template` + Sprig (`vaeljs-template-core`) |
| `vael-test` | Yes | `vael:test` + `Runtime::run_tests()` |
| `vael-uuid` | Yes | `vael:uuid`, `vael:uuid/v4`, `vael:uuid/v7` |
| `node-compat` | Yes | Sync `node:*` compatibility modules (`vaeljs-node-core`) |
| `typescript` | Yes | Erasable `.ts`/`.mts` stripping (`vaeljs-module-policy/typescript` / `deno_ast`) |
| `manifest` | No | `vaeljs-manifest` `ImportResolver` + `Runtime::with_import_resolver` |

```toml
# Minimal footprint: pure ES module evaluation + only vael:fs and vael:template
[dependencies]
vaeljs-lite-core = { path = "../vaeljs-lite-core", default-features = false, features = ["vael-fs", "vael-template"] }
```

:::caution[Compile-Time Surface Selection, Not a Sandbox]
Cargo features control which modules and dependencies are linked into your binary. They are additive across the dependency graph and are **not** a security sandbox—script code retains the host process's authority.
:::

## Runtime Lifecycle & Guards

- **Single-use per entry:** Preflight failures (`Error::Entry`, `Error::UnsupportedEntry`, `Error::TypeScript`) leave the `Runtime` unused. Once evaluation begins, the runtime is consumed and a second `run_file`/`run_source` returns `Error::RuntimeConsumed`.
- **Bounded job drain:** After the entry settles, QuickJS drains queued jobs up to `100,000` jobs (`Error::NonConvergingJobs`) and reports remaining unhandled promise rejections (`Error::UnhandledRejection`).

## When to Embed `vaeljs-core` Instead

Choose **`vaeljs-core`** (V8 + Tokio + Hyper) over `vaeljs-lite-core` when your host application needs:

- Async host I/O, timers, or servers (`vael:http`, `vael:db`, `vael:cron`, `vael:age`, `*Async` crypto, `node:fs/promises`, `execFile`).
- `Runtime::run_file_async` polled on an async executor.
- Opt-in remote loading (`https:`, `gh:`, `jsr:` via `RemoteOptions`) or `npm:` / `package.json` CAS resolution (`NpmOptions`).
- V8 JIT throughput for CPU-heavy JavaScript workloads.
