# VaelJS

Fast, secure-by-default JavaScript and TypeScript runtimes for automation, Go-style web apps, and Rust embedding — **`vjs`** (V8) and **`vjs-lite`** (QuickJS).

[![Neotales](https://github.com/neotales/brand/raw/main/assets/logo.png)](https://github.com/neotales/vaeljs)

- **Erasable TypeScript out of the box:** Run `.ts`, `.mts`, `.js`, and `.mjs` directly without a build step (`enum`, `namespace`, and transforms are rejected; no type-checking).
- **Explicit `vael:*` standard library:** Canonical specifiers (`vael:http`, `vael:db`, `vael:template`, `vael:fs`, `vael:ssh`, `vael:crypto`, `vael:age`, `vael:cron`, `vael:test`, and more).
- **Zero-`node_modules` `npm:` packages:** Content-verified CAS cache, deterministic `vael.lock` v1, `.npmrc` scoped auth (`${ENV_VAR}`), and zero auto-run install scripts.
- **Opt-outable `node:*` compatibility:** Core `node:*` modules enabled by default (`--no-node-compat` to disable).
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

Remote loading (`https:`, `gh:`, `github:`, `jsr:`, and network `npm:` fetches) is disabled by default in `vjs`; pass `--allow-remote` or set `VAELJS_ALLOW_REMOTE=1` to opt in.

## `vjs` vs `vjs-lite`

| Feature | `vjs` (`vaeljs`) | `vjs-lite` (`vaeljs-lite`) |
| --- | --- | --- |
| **JS Engine** | V8 (`deno_core 0.413.0`) | QuickJS-NG (`rquickjs 0.14.0`) |
| **Rust `Vael.engine`** | `"vael"` | `"vael-lite"` |
| **Erasable `.ts` / `.mts`** | Yes (`deno_ast`) | Yes (`deno_ast`, opt-outable) |
| **Async I/O, Timers & Server** | Yes (Tokio + Hyper + `vael:http` / `vael:db` / `vael:cron`) | Job queue only (no host timers or TCP server) |
| **Built-in Modules** | 31 total (`20 vael:*` + `11 node:*`) | 25 sync total (`15 vael:*` + `10 node:*`) |
| **`npm:` / `jsr:` / `https:` / `gh:`** | Yes (`npm:` offline-first; remote opt-in) | Local & `vael.jsonc` workspaces only |
| **Embedding Footprint** | Full V8 + Tokio + Hyper (`vaeljs-core`) | Small, zero-V8 / zero-Tokio (`vaeljs-lite-core`) |

See [vjs vs vjs-lite](docs/src/content/docs/getting-started/vjs-vs-vjs-lite.md) for the complete module and flag matrix.

## Built-in Modules Overview

Built-ins are imported strictly by canonical `vael:*` or `node:*` specifiers (bare names never shadow built-ins).

| Module(s) | Summary | Engine | Reference |
| --- | --- | --- | --- |
| `vael:http` | Go 1.22+ `ServeMux`, middleware, `.static()`, `serve()`, zero-socket `.fetch()` | `vjs` | [vael:http](docs/src/content/docs/modules/vael-http.md) |
| `vael:db` | SQLite (`openSqlite`), `@name`/`?` bindings, `transaction()` | `vjs` | [vael:db](docs/src/content/docs/modules/vael-db.md) |
| `vael:template` | Go `text/template` + Sprig helpers with `htmlEscape` | Both | [vael:template](docs/src/content/docs/modules/vael-template.md) |
| `vael:fs` | Bounded sync filesystem ops & `writeFileAtomic` | Both | [vael:fs](docs/src/content/docs/modules/vael-fs.md) |
| `vael:ssh` | Sync SSH exec with mandatory host trust (`hostKey` / `knownHosts`) | Both | [vael:ssh](docs/src/content/docs/modules/vael-ssh.md) |
| `vael:crypto` | AES-GCM/CBC, SHA-2, HMAC, PBKDF2, bcrypt, Shamir (13 sync; +7 async on `vjs`) | Both | [vael:crypto](docs/src/content/docs/modules/vael-crypto.md) |
| `vael:age` | X25519 binary age v1 encryption & decryption | `vjs` | [vael:age](docs/src/content/docs/modules/vael-age.md) |
| `vael:os`, `vael:path` | Host target constants and lexical POSIX/Win32 path utilities | Both | [os & path](docs/src/content/docs/modules/vael-os-path.md) |
| `vael:chars`, `vael:strings`, `vael:slices` | Unicode char classes, ASCII case-folding, live array/char slices | Both | [text & slices](docs/src/content/docs/modules/vael-text.md) |
| `vael:test`, `vael:assert` | Registration test runner and structural/Promise assertions | Both | [test & assert](docs/src/content/docs/modules/vael-testing.md) |
| `vael:cron`, `vael:console`, `vael:stdio`, `vael:uuid` | UTC cron (`vjs`), formatted console, stdio (`vjs`), UUID v4/v7 | Both / `vjs` | [utilities](docs/src/content/docs/modules/vael-utilities.md) |
| `node:*` | `buffer`, `child_process`, `crypto`, `fs`, `fs/promises` (`vjs`), `os`, `path`, `process`, `url` | Both | [node compat](docs/src/content/docs/modules/node-compat.md) |

## Embedding in Rust (`vaeljs-lite-core`)

Embed QuickJS with zero V8, Tokio, or Hyper dependencies using [`vaeljs-lite-core`](vaeljs-lite-core/README.md):

```rust
use vaeljs_lite_core::Runtime;

fn main() -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("examples/hello.js")
}
```

Granular Cargo features (`--no-default-features` + `vael-fs`, `vael-crypto`, `vael-template`, `manifest`, `typescript`, etc.) let you select the exact surface compiled in. See the [Embedding Guide](docs/src/content/docs/guides/embedding-vaeljs-lite.md).

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

## Documentation (`docs/`)

The Starlight documentation site lives in [`docs/`](docs/):

- **Getting Started:** [Installation](docs/src/content/docs/getting-started/installation.md) · [Quickstart](docs/src/content/docs/getting-started/quickstart.md) · [vjs vs vjs-lite](docs/src/content/docs/getting-started/vjs-vs-vjs-lite.md)
- **CLI & Config:** [Commands & Flags](docs/src/content/docs/cli/commands.md) · [vael.jsonc & Workspaces](docs/src/content/docs/cli/vael-jsonc.md) · [npm & Remote Loading](docs/src/content/docs/cli/npm-and-remote.md)
- **Guides:** [Automation](docs/src/content/docs/guides/automation.md) · [Go-Style WebApps](docs/src/content/docs/guides/webapps.md) · [Embedding vaeljs-lite](docs/src/content/docs/guides/embedding-vaeljs-lite.md)
- **Architecture Notes:** [Database API](docs/db-api.md) · [`vael.jsonc` Spec](docs/vael-jsonc.md) · [ESM/npm Design](docs/esm-npm-proposal.md)

## License

[MIT](LICENSE.md) — Copyright (c) 2026 NeoTales and x0craft. Third-party notices ship in `vaeljs-core/THIRD_PARTY_NOTICES.md` and `vaeljs-lite-core/THIRD_PARTY_NOTICES.md`.
