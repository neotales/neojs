# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.0-alpha.0] - 2026-11-24

> **Alpha Pre-Release Notice:** `v0.0.0-alpha.0` is an early alpha pre-release. Public Rust and JavaScript APIs may change before `0.1.0` and `1.0.0`. Crates are not published to crates.io (`publish = false` across all workspace members).

### Added

- **Dual-Engine Runtimes:**
  - `neojs` / `neojs-core`: V8-backed ECMAScript module runtime via vendored `deno_core 0.413.0` and `v8 150.4.0`.
  - `neojs-lite` / `neojs-lite-core`: QuickJS-backed ECMAScript module runtime via `rquickjs 0.14.0` with zero V8/Tokio dependencies.
  - Shared erasable TypeScript stripping (`.ts`, `.mts`) via `deno_ast` in `neojs-module-policy` across both `neojs` and `neojs-lite` (type-checking and transpilation transforms remain out of scope).
- **Shared Built-in Modules (both `neojs` and `neojs-lite`):**
  - `neo:assert`: Shared assertion helpers and rejection/error assertions.
  - `neo:chars`: Unicode `General_Category` character classification and case helpers.
  - `neo:crypto`: Engine-neutral synchronous primitives via `neojs-crypto-core` (13 exports shared on both `neojs` and `neojs-lite`: `generateEnvelopeKey`, `randomBytes`, `encrypt`, `decrypt`, `createEncryptor`, `createDecryptor`, `createHash`, `createHmac`, `pbkdf2`, `bcryptHash`, `bcryptVerify`, `shamirSplit`, `shamirCombine`; plus 7 `*Async` worker-pool exports on `neojs` V8).
  - `neo:fs`: Engine-neutral core filesystem operations via `neojs-fs-core` (14 exports: `readFile`, `readTextFile`, `writeFile`, `writeTextFile`, `writeFileAtomic`, `exists`, `stat`, `lstat`, `readDir`, `mkdir`, `remove`, `rename`, `copyFile`, `realpath`).
  - `neo:os`: Target `platform` and `arch` constants.
  - `neo:ssh`: Synchronous SSH client via `neojs-ssh-core` (`neotales-ssh`) with mandatory host trust verification and bounded execution.
  - `neo:strings`: Unicode-aware string helpers.
  - `neo:test`: Shared test registration and execution API (`test(...)`).
  - `neo:uuid`: RFC 9562 v4 and v7 UUID generation (`neo:uuid`, `neo:uuid/v4`, `neo:uuid/v7`) backed by fallible OS CSPRNG (`getrandom`).
- **V8-Only Built-in Modules (`neojs` / `neojs-core`):**
  - `neo:age`: Age v1 encryption and decryption (`neotales-age`).
  - `neo:console`: Structured console output module.
  - `neo:cron`: Cron expression parsing and scheduling.
  - `neo:crypto`: Asynchronous worker-pool APIs (`hashAsync`, `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, `bcryptVerifyAsync`, `shamirSplitAsync`, `shamirCombineAsync`) in addition to the 13 synchronous primitives.
  - `neo:path`: Cross-platform path manipulation helpers.
  - `neo:slices`: Character slice utilities built on `neo:chars`.
  - `neo:stdio`: Standard I/O stream helpers.
- **CLI, Manifest, Remote Loading & Type Declarations:**
  - `neojs test` and `neojs-lite test`: Deterministic glob expansion and shared JS/TS test runner (`tests/js/shared/`).
  - `neojs types` and `neojs-lite types`: Bundled TypeScript `.d.ts` declarations (`neojs-dts`) for `v8` and `lite` profiles (`--profile <v8|lite>`, `--out-dir <dir>`), verified by live runtime CI drift tests (`dts_drift`).
  - `neo.jsonc` support (`neojs-manifest`): Bounded upward manifest discovery, `imports` maps, `workspaces` resolution, and `--config <path>` / `--no-config` CLI flags on both `neojs` and `neojs-lite`.
  - Remote module loading on `neojs` (`neojs-remote`): Opt-in `--allow-remote` / `NEOJS_ALLOW_REMOTE` support for `https:`, `gh:`/`github:`, and `jsr:` specifiers with SHA-256 verification, redirect pointers, and offline cache reads.
- **Foundational Engine-Neutral Crates (not yet wired to CLI):**
  - `neojs-npm`: Phase 1 ESM-only no-`node_modules` specifier parser, SemVer matching, `package.json`/`exports` resolver, and content-addressed tarball ingestor.
  - `neojs-dbapi`: Driver-neutral SQL Rust foundation with SQLite provider (`neotales-dbapi` / `neotales-dbapi-sqlite`).

### Security & Policy

- Remote module loading is disabled by default (`--no-allow-remote` overrides `NEOJS_ALLOW_REMOTE`); `neojs-lite` stays strictly local/workspace-only.
- Canonical `neo:` module specifiers and fail-closed `neojs-module-policy` validation across both runtimes.
- `publish = false` enforced across all 14 workspace crates; all workspace versions aligned to `0.0.0-alpha.0`.

### Known Limitations

- Pre-release APIs across Rust crates and `neo:*` modules may change before `0.1.0` and `1.0.0`.
- Crates are not published to crates.io (`publish = false`).
- `npm:` runtime wiring, `node:*` compatibility modules, `neo.lock`, `neo:os/dirs`, `neo:fs/acl`, and `neojs lsp` are not included in `0.0.0-alpha.0` and are planned for subsequent alpha releases.

[Unreleased]: https://github.com/neotales/neojs/compare/v0.0.0-alpha.0...HEAD
[0.0.0-alpha.0]: https://github.com/neotales/neojs/releases/tag/v0.0.0-alpha.0
