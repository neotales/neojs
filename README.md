# VaelJS

Fast, secure-by-default JavaScript and TypeScript runtimes for automation, Go-style web apps, and Rust embedding — **`vjs`** (V8) and **`vjs-lite`** (QuickJS).

[![Neotales](https://github.com/neotales/brand/raw/main/assets/logo.png)](https://github.com/neotales/vaeljs)

- **Erasable TypeScript out of the box:** Run `.ts`, `.mts`, `.js`, and `.mjs` directly without a build step (`enum`, `namespace`, and transforms are rejected; no type-checking).
- **Explicit `vael:*` standard library:** Canonical specifiers (`vael:http`, `vael:db`, `vael:template`, `vael:fs`, `vael:ssh`, `vael:crypto`, `vael:age`, `vael:cron`, `vael:test`, and more).
- **Zero-`node_modules` `npm:`, remote & `vael.jsonc`:** Content-verified CAS cache, `vael.lock` v1, `.npmrc` scoped auth (`${ENV_VAR}`), `jsr:` / `gh:` / `https:` imports, `vael.jsonc` workspaces, and zero install scripts across both runtimes.
- **Opt-outable `node:*` compatibility:** Core `node:*` modules (`11` in `vjs`, `10` sync in `vjs-lite`) enabled by default (`--no-node-compat` to disable).
- **Go-style frameworkless web apps:** Immutable `(req, ctx) => Response` handlers over `vael:http` `ServeMux`, SQLite via `vael:db`, and XSS-safe `vael:template`.

## Quick Start

Run a TypeScript or JavaScript module, run tests, generate `.d.ts` declarations, or launch the Go-style web app prototype:

```sh
# Run an ES module or erasable TypeScript file
vjs examples/hello.js
vjs-lite examples/hello.js

# Run tests (each file runs in an isolated runtime)
vjs test "tests/js/shared/*.test.*"
vjs-lite test "tests/js/shared/*.test.*"

# Emit TypeScript declarations for your editor
vjs types --out-dir ./types
vjs-lite types --out-dir ./types

# Run the zero-framework Go-style webapp prototype and its tests
vjs examples/webapp/main.ts
vjs test examples/webapp/app.test.ts
```

Remote loading (`https:`, `gh:`, `github:`, `jsr:`, and network `npm:` fetches) is disabled by default in both `vjs` and `vjs-lite`; pass `--allow-remote` or set `VAELJS_ALLOW_REMOTE=1` to opt in.

## `vjs` vs `vjs-lite`

| Feature | `vjs` (`vaeljs`) | `vjs-lite` (`vaeljs-lite`) |
| --- | --- | --- |
| **JS Engine** | V8 (`deno_core 0.413.0`) | QuickJS-NG (`rquickjs 0.14.0`) |
| **Rust `Vael.engine`** | `"vael"` | `"vael-lite"` |
| **Erasable `.ts` / `.mts`** | Yes (`deno_ast`) | Yes (`deno_ast`, opt-outable) |
| **Async I/O, Timers & Server** | Yes (Tokio + Hyper + `vael:http` / `vael:db` / `vael:cron`) | Job queue only (no host timers or TCP server) |
| **Built-in Modules** | 31 total (`20 vael:*` + `11 node:*`) | 25 sync total (`15 vael:*` + `10 node:*`, opt-outable) |
| **`npm:`, Remote & `vael.jsonc`** | Yes (`npm:`, `jsr:`, `gh:`/`github:`, `https:`, `vael.jsonc`; remote opt-in) | Yes (`npm:`, `jsr:`, `gh:`/`github:`, `https:`, `vael.jsonc`; blocking `ureq` + `rustls`, opt-outable) |
| **Embedding Footprint** | Full V8 + Tokio + Hyper (`vaeljs-core`) | Small, zero-V8 / zero-Tokio / zero-Hyper (`vaeljs-lite-core`, `--no-default-features`) |

See [vjs vs vjs-lite](https://vaeljs.com/getting-started/vjs-vs-vjs-lite/) for the complete module and flag matrix.

## Built-in Modules Overview

Built-ins are imported strictly by canonical `vael:*` or `node:*` specifiers (bare names never shadow built-ins).

| Module(s) | Summary | Engine | Reference |
| --- | --- | --- | --- |
| `vael:http` | Go 1.22+ `ServeMux`, middleware, `.static()`, `serve()`, zero-socket `.fetch()` | `vjs` | [vael:http](https://vaeljs.com/modules/vael-http/) |
| `vael:db` | SQLite (`openSqlite`), `@name`/`?` bindings, `transaction()` | `vjs` | [vael:db](https://vaeljs.com/modules/vael-db/) |
| `vael:template` | Go `text/template` + Sprig helpers with `htmlEscape` | Both | [vael:template](https://vaeljs.com/modules/vael-template/) |
| `vael:fs` | Bounded sync filesystem ops & `writeFileAtomic` | Both | [vael:fs](https://vaeljs.com/modules/vael-fs/) |
| `vael:ssh` | Sync SSH exec with mandatory host trust (`hostKey` / `knownHosts`) | Both | [vael:ssh](https://vaeljs.com/modules/vael-ssh/) |
| `vael:crypto` | AES-GCM/CBC, SHA-2, HMAC, PBKDF2, bcrypt, Shamir (13 sync; +7 async on `vjs`) | Both | [vael:crypto](https://vaeljs.com/modules/vael-crypto/) |
| `vael:age` | X25519 binary age v1 encryption & decryption | `vjs` | [vael:age](https://vaeljs.com/modules/vael-age/) |
| `vael:os`, `vael:path` | Host target constants and lexical POSIX/Win32 path utilities | Both | [os & path](https://vaeljs.com/modules/vael-os-path/) |
| `vael:chars`, `vael:strings`, `vael:slices` | Unicode char classes, ASCII case-folding, live array/char slices | Both | [text & slices](https://vaeljs.com/modules/vael-text/) |
| `vael:test`, `vael:assert` | Registration test runner and structural/Promise assertions | Both | [test & assert](https://vaeljs.com/modules/vael-testing/) |
| `vael:cron`, `vael:console`, `vael:stdio`, `vael:uuid` | UTC cron (`vjs`), formatted console, stdio (`vjs`), UUID v4/v7 | Both / `vjs` | [utilities](https://vaeljs.com/modules/vael-utilities/) |
| `node:*` | `buffer`, `child_process`, `crypto`, `fs`, `fs/promises` (`vjs`), `os`, `path` (`posix`, `win32`), `process`, `url` | Both | [node compat](https://vaeljs.com/modules/node-compat/) |

## Embedding in Rust (`vaeljs-lite-core`)

Embed QuickJS with zero V8, Tokio, or Hyper dependencies using [`vaeljs-lite-core`](vaeljs-lite-core/README.md):

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("examples/hello.js")
}
```

Default features include `remote` (blocking `ureq` + `rustls`, zero Tokio/hyper), `npm`, `node-compat`, `typescript`, and sync `vael:*` modules; opt out with `--no-default-features` and enable only what you need (`vael-fs`, `vael-crypto`, `vael-template`, `manifest`, `remote`, `npm`, `typescript`, etc.). See the [Embedding Guide](https://vaeljs.com/guides/embedding-vaeljs-lite/).

## Building & Testing from Source

Requires Rust `1.100.0` (matching `Cargo.toml` and `eng/size/toolchain`):

```sh
# Build and test the default V8 workspace (vjs + shared crates)
cargo test --locked

# Build and test the opt-in QuickJS runtime (vjs-lite + vaeljs-lite-core)
cargo test --locked -p vaeljs-lite-core -p vaeljs-lite

# Run the shared JS/TS test suite on both runtimes
cargo run --locked -p vaeljs --bin vjs -- test tests/js/shared/*.test.*
cargo run --locked -p vaeljs-lite --bin vjs-lite -- test tests/js/shared/*.test.*

# Run lints and format checks
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Prebuilt release archives for 6 targets (`linux`, `macos`, `windows` × `amd64`, `arm64`) with `SHA256SUMS.txt` and bundled `types/` are published on [GitHub Releases](https://github.com/neotales/vaeljs/releases).

## Documentation

The VaelJS documentation site lives at [**vaeljs.com**](https://vaeljs.com) and is maintained in [`neotales/vaeljs-site`](https://github.com/neotales/vaeljs-site) (see [`docs/README.md`](docs/README.md); [`vaeljs-dts/types/`](vaeljs-dts/types/) remains the API and `@since` source of truth in this repository):

- **Getting Started:** [Installation](https://vaeljs.com/getting-started/installation/) · [Quickstart](https://vaeljs.com/getting-started/quickstart/) · [vjs vs vjs-lite](https://vaeljs.com/getting-started/vjs-vs-vjs-lite/)
- **CLI & Config:** [Commands & Flags](https://vaeljs.com/cli/commands/) · [vael.jsonc & Workspaces](https://vaeljs.com/cli/vael-jsonc/) · [npm & Remote Loading](https://vaeljs.com/cli/npm-and-remote/)
- **Guides & Modules:** [Automation](https://vaeljs.com/guides/automation/) · [Go-Style WebApps](https://vaeljs.com/guides/webapps/) · [Embedding vaeljs-lite](https://vaeljs.com/guides/embedding-vaeljs-lite/) · [Modules Overview](https://vaeljs.com/modules/overview/)
- **Architecture, Roadmap & Blog:** [Database API](https://vaeljs.com/architecture/db-api/) · [`vael.jsonc` Spec](https://vaeljs.com/architecture/vael-jsonc-spec/) · [ESM/npm Design](https://vaeljs.com/architecture/esm-and-npm/) · [Roadmap](https://vaeljs.com/roadmap/) · [Blog](https://vaeljs.com/blog/)

## License

[MIT](LICENSE.md) — Copyright (c) 2026 NeoTales and x0craft. Third-party notices ship in `vaeljs-core/THIRD_PARTY_NOTICES.md` and `vaeljs-lite-core/THIRD_PARTY_NOTICES.md`.
