# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.0-alpha.1] - 2026-11-26

> **Alpha Pre-Release Notice:** `v0.0.0-alpha.1` brings `vjs-lite` to full remote/npm module-resolution parity with `vjs`, completes JSDoc coverage across the TypeScript declaration bundles, and aligns `vael:os` target strings with Deno. Public Rust and JavaScript APIs remain unstable until `0.1.0` / `1.0.0`. All 17 workspace crates remain `publish = false`.

### Added

- **`vjs-lite` (`vaeljs-lite` / `vaeljs-lite-core`) Remote & `npm:` Parity:**
  - Full `npm:`, `jsr:`, `gh:`/`github:`, `https:`, `package.json`, `vael.lock` v1, `.npmrc` scoped registries/auth, URL entrypoints, and ancestor `vael.jsonc` bare import maps on QuickJS (`vjs-lite`).
  - Matching CLI and policy flags: `--allow-remote`, `--no-allow-remote`, `VAELJS_ALLOW_REMOTE`, `--no-npm`, `--frozen-lockfile`, and `--offline`.
  - Uses blocking `ureq` + `rustls` with zero Tokio, Hyper, or V8 dependencies (opt-outable via `--no-default-features` for offline-only builds).
- **Rich TypeScript Declaration JSDoc (`vaeljs-dts`):**
  - Complete Node/JSR-style JSDoc (`@param`, `@returns`, `@throws`, `@example`, `@since`) across all 36 module and global `.d.ts` files in `vaeljs-dts/types/` (`vael:*`, `node:*`, `v8/globals`, and `lite/globals`).
- **Integration Coverage:**
  - End-to-end verification of ancestor `vael.jsonc` bare import maps resolving across `npm:`, `jsr:`, `gh:`/`github:`, and `https:`.

### Changed

- **Breaking — `vael:os` Target Vocabulary:** `platform` in `vael:os` (`Vael.os.platform`) now uses Deno's `Deno.build.os` target strings (`"darwin"` on macOS instead of Rust's `"macos"`). Callers checking `platform === "macos"` must update to `platform === "darwin"`.
- **Example WebApp (`examples/webapp/`):** Renamed `mux` → `router` across application, entrypoint, test, and README files.
- **Documentation:** Slimmed root `README.md` and moved full guides and module reference documentation to `https://vaeljs.com` (`neotales/vaeljs-site`).

## [0.0.0-alpha.0] - 2026-11-25

> **Alpha Pre-Release Notice:** `v0.0.0-alpha.0` is the inaugural `vaeljs` alpha pre-release. Public Rust and JavaScript APIs may change before `0.1.0` and `1.0.0`. All 17 workspace crates remain `publish = false` (no crates.io publishing yet).

### Added

- **Shippable Binaries & Release Packaging (`vjs` & `vjs-lite`):**
  - `vjs` (`vaeljs` / `vaeljs-core`): V8-backed ECMAScript module runtime via vendored `deno_core 0.413.0` and `v8 150.4.0`.
  - `vjs-lite` (`vaeljs-lite` / `vaeljs-lite-core`): QuickJS-backed ECMAScript module runtime via `rquickjs 0.14.0` with zero V8, Tokio, or Hyper dependencies.
  - `[profile.release]` enables `strip = "symbols"` across all 6 release targets (`linux`/`macos`/`windows` × `amd64`/`arm64`), bundling `LICENSE.md`, profile-matched TypeScript `types/`, and `SHA256SUMS.txt`. All 17 `vaeljs*` workspace crates remain `publish = false`.
- **`vael:*` Standard Library & `Vael` Global (20 modules on `vjs`, 15 sync modules on `vjs-lite`):**
  - **`vael:http` (`vjs`):** Go 1.22+-style `ServeMux` router, pure `(req, ctx) => Response` handlers, ergonomic builders/errors/middleware, traversal-proof `.static()`, `hyper 1` HTTP/1.1 server, and zero-socket in-process `.fetch()`.
  - **`vael:db` (`vjs`):** Uniform `@name`/`?` parameterized SQLite database client over `vaeljs-dbapi` (`neotales-dbapi` / `neotales-dbapi-sqlite`) with atomic transactions and redacted error surfaces.
  - **`vael:template` (both `vjs` and `vjs-lite`):** Go `text/template` and Sprig helpers over `neotales-gotmpl` & `neotales-sprig` (`vaeljs-template-core`) with XSS-safe `htmlEscape: true`.
  - **Shared modules (both `vjs` and `vjs-lite`):** `vael:assert`, `vael:chars`, `vael:console`, `vael:crypto` (13 synchronous primitives via `vaeljs-crypto-core`, plus 7 `*Async` worker-pool APIs on `vjs`), `vael:fs` (`vaeljs-fs-core`), `vael:os`, `vael:path`, `vael:slices`, `vael:ssh` (`vaeljs-ssh-core` / `neotales-ssh` with mandatory host trust verification and bounded execution), `vael:strings`, `vael:test`, and `vael:uuid` (`vael:uuid`, `vael:uuid/v4`, `vael:uuid/v7` backed by fallible `getrandom`).
  - **Additional V8-only modules (`vjs`):** `vael:age` (`neotales-age`), `vael:cron`, and `vael:stdio`.
- **`npm:` / `package.json` / `vael.lock` v1 / `.npmrc`:**
  - Zero `node_modules` directory, zero auto-run install or lifecycle scripts, and content-addressed verified CAS tarball caching via `vaeljs-npm`.
  - Deterministic `vael.lock` v1 resolution with `--frozen-lockfile`, `--offline`, and `--no-npm` CLI controls.
  - `.npmrc` scoped registry routing and HTTPS-only Bearer/Basic auth (`${ENV_VAR}` interpolation, cross-origin and out-of-scope redirect auth stripping, zero token leakage).
- **Opt-Outable `node:*` Automation Compatibility (`node-compat`, `--no-node-compat`):**
  - 11 modules on `vjs` (`node:buffer`, `node:child_process`, `node:crypto`, `node:fs`, `node:fs/promises`, `node:os`, `node:path`, `node:path/posix`, `node:path/win32`, `node:process`, `node:url`) and 10 sync modules on `vjs-lite` (all except `node:fs/promises`), backed by `vaeljs-node-core`.
- **Frameworkless Go-Style WebApp Prototype (`examples/webapp/`):**
  - Runnable zero-framework TypeScript web application combining `vael:http`, `vael:db`, `vael:template`, `vael:uuid`, and `vael:crypto`, verified with `vjs test examples/webapp/app.test.ts`.
- **TypeScript, Manifests, Remote Loading & Testing:**
  - Shared erasable TypeScript stripping (`.ts`, `.mts`) via `deno_ast` (`vaeljs-module-policy`) on both `vjs` and `vjs-lite`.
  - `vjs types` and `vjs-lite types` `.d.ts` bundles (`vaeljs-dts`, `--profile <v8|lite>`, `--out-dir <dir>`), verified 1:1 against live runtime exports by `dts_drift` tests.
  - `vael.jsonc` manifests & workspaces (`vaeljs-manifest`): bounded upward discovery, `imports` maps, `workspaces` resolution, and `--config <path>` / `--no-config`.
  - Remote module loading on `vjs` (`vaeljs-remote`): opt-in `--allow-remote` / `VAELJS_ALLOW_REMOTE` for `https:`, `gh:`/`github:`, and `jsr:` with SHA-256 verification and offline cache reads.
  - `vjs test` and `vjs-lite test`: deterministic glob expansion and 14 shared JS+TS test files (81 tests in `tests/js/shared/`) passing identically on both engines.

### Security & Policy

- Remote module loading and remote npm fetches are disabled by default (`--no-allow-remote` overrides `VAELJS_ALLOW_REMOTE`); `vjs-lite` stays strictly local/workspace-only.
- Canonical `vael:` and `node:` module specifiers, fail-closed `vaeljs-module-policy` validation, and `--no-node-compat` opt-out across both runtimes.
- `publish = false` enforced across all 17 workspace crates; all workspace versions aligned to `0.0.0-alpha.0`.

### Known Limitations & Deferred Scope

- Pre-release APIs across Rust crates and `vael:*` modules may change before `0.1.0` and `1.0.0`.
- **Explicitly deferred beyond `0.0.0-alpha.0`:** crates.io publishing (`publish = false`), network DB drivers (Postgres, MySQL, MariaDB, and SQL Server remain feature-deferred), `vael:os/dirs`, `vael:fs/acl`, `vael:archive`, `vjs lsp`, and cute dragon mascot / logo assets.

[Unreleased]: https://github.com/neotales/vaeljs/compare/v0.0.0-alpha.1...HEAD
[0.0.0-alpha.1]: https://github.com/neotales/vaeljs/compare/v0.0.0-alpha.0...v0.0.0-alpha.1
[0.0.0-alpha.0]: https://github.com/neotales/vaeljs/releases/tag/v0.0.0-alpha.0
