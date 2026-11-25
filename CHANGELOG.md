# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.0-alpha.0] - 2026-11-24

> **Alpha Pre-Release Notice:** `v0.0.0-alpha.0` is an early alpha pre-release. Public Rust and JavaScript APIs may change before `0.1.0` and `1.0.0`. Crates are not published to crates.io (`publish = false` across all workspace members).

### Added

- **Dual-Engine Runtimes:**
  - `vaeljs` / `vaeljs-core`: V8-backed ECMAScript module runtime via vendored `deno_core 0.413.0` and `v8 150.4.0`.
  - `vaeljs-lite` / `vaeljs-lite-core`: QuickJS-backed ECMAScript module runtime via `rquickjs 0.14.0` with zero V8/Tokio dependencies.
  - Shared erasable TypeScript stripping (`.ts`, `.mts`) via `deno_ast` in `vaeljs-module-policy` across both `vaeljs` and `vaeljs-lite` (type-checking and transpilation transforms remain out of scope).
- **Shared Built-in Modules (both `vaeljs` and `vaeljs-lite`):**
  - `vael:assert`: Shared assertion helpers and rejection/error assertions.
  - `vael:chars`: Unicode `General_Category` character classification and case helpers.
  - `vael:crypto`: Engine-neutral synchronous primitives via `vaeljs-crypto-core` (13 exports shared on both `vaeljs` and `vaeljs-lite`: `generateEnvelopeKey`, `randomBytes`, `encrypt`, `decrypt`, `createEncryptor`, `createDecryptor`, `createHash`, `createHmac`, `pbkdf2`, `bcryptHash`, `bcryptVerify`, `shamirSplit`, `shamirCombine`; plus 7 `*Async` worker-pool exports on `vaeljs` V8).
  - `vael:fs`: Engine-neutral core filesystem operations via `vaeljs-fs-core` (14 exports: `readFile`, `readTextFile`, `writeFile`, `writeTextFile`, `writeFileAtomic`, `exists`, `stat`, `lstat`, `readDir`, `mkdir`, `remove`, `rename`, `copyFile`, `realpath`).
  - `vael:os`: Target `platform` and `arch` constants.
  - `vael:ssh`: Synchronous SSH client via `vaeljs-ssh-core` (`neotales-ssh`) with mandatory host trust verification and bounded execution.
  - `vael:strings`: Unicode-aware string helpers.
  - `vael:test`: Shared test registration and execution API (`test(...)`).
  - `vael:uuid`: RFC 9562 v4 and v7 UUID generation (`vael:uuid`, `vael:uuid/v4`, `vael:uuid/v7`) backed by fallible OS CSPRNG (`getrandom`).
- **V8-Only Built-in Modules (`vaeljs` / `vaeljs-core`):**
  - `vael:age`: Age v1 encryption and decryption (`neotales-age`).
  - `vael:console`: Structured console output module.
  - `vael:cron`: Cron expression parsing and scheduling.
  - `vael:crypto`: Asynchronous worker-pool APIs (`hashAsync`, `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, `bcryptVerifyAsync`, `shamirSplitAsync`, `shamirCombineAsync`) in addition to the 13 synchronous primitives.
  - `vael:path`: Cross-platform path manipulation helpers.
  - `vael:slices`: Character slice utilities built on `vael:chars`.
  - `vael:stdio`: Standard I/O stream helpers.
- **CLI, Manifest, Remote Loading & Type Declarations:**
  - `vjs test` and `vjs-lite test`: Deterministic glob expansion and shared JS/TS test runner (`tests/js/shared/`).
  - `vjs types` and `vjs-lite types`: Bundled TypeScript `.d.ts` declarations (`vaeljs-dts`) for `v8` and `lite` profiles (`--profile <v8|lite>`, `--out-dir <dir>`), verified by live runtime CI drift tests (`dts_drift`).
  - `vael.jsonc` support (`vaeljs-manifest`): Bounded upward manifest discovery, `imports` maps, `workspaces` resolution, and `--config <path>` / `--no-config` CLI flags on both `vjs` and `vjs-lite`.
  - Remote module loading on `vaeljs` (`vaeljs-remote`): Opt-in `--allow-remote` / `VAELJS_ALLOW_REMOTE` support for `https:`, `gh:`/`github:`, and `jsr:` specifiers with SHA-256 verification, redirect pointers, and offline cache reads.
- **Foundational Engine-Neutral Crates (not yet wired to CLI):**
  - `vaeljs-npm`: Phase 1 ESM-only no-`node_modules` specifier parser, SemVer matching, `package.json`/`exports` resolver, and content-addressed tarball ingestor.
  - `vaeljs-dbapi`: Driver-neutral SQL Rust foundation with SQLite provider (`neotales-dbapi` / `neotales-dbapi-sqlite`).

### Security & Policy

- Remote module loading is disabled by default (`--no-allow-remote` overrides `VAELJS_ALLOW_REMOTE`); `vaeljs-lite` stays strictly local/workspace-only.
- Canonical `vael:` module specifiers and fail-closed `vaeljs-module-policy` validation across both runtimes.
- `publish = false` enforced across all 17 workspace crates; all workspace versions aligned to `0.0.0-alpha.0`.

### Known Limitations

- Pre-release APIs across Rust crates and `vael:*` modules may change before `0.1.0` and `1.0.0`.
- Crates are not published to crates.io (`publish = false`).
- `npm:` runtime wiring, `node:*` compatibility modules, `vael.lock`, `vael:os/dirs`, `vael:fs/acl`, and `vjs lsp` are not included in `0.0.0-alpha.0` and are planned for subsequent alpha releases.

[Unreleased]: https://github.com/neotales/vaeljs/compare/v0.0.0-alpha.0...HEAD
[0.0.0-alpha.0]: https://github.com/neotales/vaeljs/releases/tag/v0.0.0-alpha.0
