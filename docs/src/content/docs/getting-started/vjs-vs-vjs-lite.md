---
title: vjs vs vjs-lite
description: Complete technical comparison of vjs (V8) and vjs-lite (QuickJS), including the 31 vs 25 module matrix and decision guide.
---

VaelJS provides two compatible runtimes built on shared module policy, TypeScript stripping, and core Rust crates: **`vjs`** (V8) and **`vjs-lite`** (QuickJS).

## Runtime Comparison

| Dimension | `vjs` (`vaeljs` / `vaeljs-core`) | `vjs-lite` (`vaeljs-lite` / `vaeljs-lite-core`) |
| --- | --- | --- |
| **JS Engine** | V8 via vendored `deno_core 0.413.0` | QuickJS-NG via `rquickjs 0.14.0` |
| **`Vael.engine`** | `"vael"` | `"vael-lite"` |
| **Async / Event Loop** | Full Tokio multi-thread + current-thread event loop (`hyper` HTTP, `vael:db`, `vael:cron`, `*Async` crypto) | QuickJS job queue only (max 100,000 jobs; no host timers or external I/O event loop) |
| **TypeScript Stripping** | `.ts` and `.mts` via `deno_ast` (always enabled) | `.ts` and `.mts` via `deno_ast` (opt-outable in Rust via `--no-default-features`) |
| **Built-in Modules** | **31 total**: 20 `vael:*` + 11 `node:*` | **25 total**: 15 sync `vael:*` + 10 sync `node:*` |
| **`vael.jsonc` & Workspaces** | Built-in discovery + `--config` / `--no-config` | Built-in discovery + `--config` / `--no-config` (opt-in `manifest` feature in core crate) |
| **`npm:` & `package.json`** | Yes (offline-first CAS cache, `vael.lock` v1, `.npmrc`) | Rejected (local & workspace modules only) |
| **Remote (`https:`, `gh:`, `jsr:`)** | Opt-in via `--allow-remote` or `VAELJS_ALLOW_REMOTE=1` | Rejected (local & workspace modules only) |
| **Test Timeouts** | Default `30_000` ms (`test(name, fn, { timeout })`) | Rejected with `VAEL_TEST_NO_TIMER` |
| **Rust Embedding Footprint** | Full V8 + Tokio + Hyper | Compact QuickJS-only; zero V8, Tokio, or Hyper |

## Complete Module Matrix (31 vs 25)

Every module below is verified against `vaeljs-dts` (`Profile::V8` and `Profile::Lite`). Bare names never resolve to built-ins unless mapped explicitly in `vael.jsonc`.

### `vael:*` Built-in Modules (20 vs 15)

| Specifier | `vjs` (V8) | `vjs-lite` (QuickJS) | Notes |
| --- | --- | --- | --- |
| `vael:age` | Yes | — | X25519 binary age v1 encryption/decryption |
| `vael:assert` | Yes | Yes | Callable `assert`, `equals`, `throws`, `rejects`, `AssertionError` |
| `vael:chars` | Yes | Yes | Unicode General_Category predicates and case conversion |
| `vael:console` | Yes | Yes | `log`, `debug`, `info`, `warn`, `error` |
| `vael:cron` | Yes | — | UTC five-field `cron()` scheduler and `next()` |
| `vael:crypto` | Full (20 exports) | Sync only (13 exports) | `vjs` adds 7 `*Async` helpers (`hashAsync`, `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, `bcryptVerifyAsync`, `shamirSplitAsync`, `shamirCombineAsync`) |
| `vael:db` | Yes | — | Async SQLite (`open`, `openSqlite`), `@name`/`?` binding, `transaction()` |
| `vael:fs` | Yes | Yes | Synchronous file/dir ops + `writeFileAtomic` |
| `vael:http` | Yes | — | Go 1.22+ `ServeMux`, middleware, `.static()`, `serve()`, `.fetch()` |
| `vael:os` | Yes | Yes | `platform` and `arch` constants |
| `vael:path` | Yes | Yes | Lexical `posix`, `win32`, `join`, `resolve`, `relative`, `common` |
| `vael:slices` | Yes | Yes | Live mutable/read-only `Slice` and `CharSlice` views |
| `vael:ssh` | Yes | Yes | Sync SSH exec with mandatory `hostKey` or `knownHosts` verification |
| `vael:stdio` | Yes | — | `Readable`, `Writable`, `stdout`, `stderr` |
| `vael:strings` | Yes | Yes | ASCII case-folding search and trimming |
| `vael:template` | Yes | Yes | Go `text/template` + Sprig helpers (`render`, `compile`, `createTemplateSet`, `escapeHtml`) |
| `vael:test` | Yes | Yes | Registration test runner (`timeout` rejected in `vjs-lite`) |
| `vael:uuid` | Yes | Yes | RFC 9562 `v4` and `v7` |
| `vael:uuid/v4` | Yes | Yes | Subpath exporting `v4` (named and default) |
| `vael:uuid/v7` | Yes | Yes | Subpath exporting `v7` (named and default) |

### `node:*` Compatibility Modules (11 vs 10)

| Specifier | `vjs` (V8) | `vjs-lite` (QuickJS) | Notes |
| --- | --- | --- | --- |
| `node:buffer` | Yes | Yes | `Buffer` |
| `node:child_process` | `spawnSync`, `execFileSync`, `execFile` | `spawnSync`, `execFileSync` | Shell-free subprocess execution (`execFile` is V8-only) |
| `node:crypto` | Yes | Yes | `randomBytes`, `randomUUID`, `createHash`, `createHmac`, `pbkdf2Sync`, `timingSafeEqual` |
| `node:fs` | Sync + `.promises` | Sync only | `readFileSync`, `writeFileSync`, `mkdtempSync`, etc. |
| `node:fs/promises` | Yes | — | Async filesystem promises (V8-only) |
| `node:os` | Yes | Yes | `platform`, `arch`, `homedir`, `tmpdir`, `hostname`, `cpus`, etc. |
| `node:path` | Yes | Yes | Platform path helpers |
| `node:path/posix` | Yes | Yes | POSIX path helpers |
| `node:path/win32` | Yes | Yes | Windows path helpers |
| `node:process` | Yes | Yes | `argv`, `cwd`, `chdir`, `env`, `exit`, `exitCode`, `hrtime`, etc. |
| `node:url` | Yes | Yes | `fileURLToPath`, `pathToFileURL`, `URL`, `URLSearchParams` |

## CLI Flag Comparison

| Flag / Setting | `vjs` | `vjs-lite` |
| --- | --- | --- |
| `<file>`, `test <glob>...`, `types` | Yes | Yes |
| `--config <path>`, `--no-config` | Yes | Yes |
| `--no-node-compat` | Yes | Yes |
| `--allow-remote`, `--no-allow-remote`, `VAELJS_ALLOW_REMOTE` | Yes | — |
| `--frozen-lockfile`, `--offline`, `--no-npm` | Yes | — |

## Decision Guide

- **Use `vjs`** when you need `vael:http` web servers, `vael:db` SQLite databases, `vael:cron` schedules, `vael:age` encryption, async filesystem/process APIs (`node:fs/promises`, `execFile`), `npm:` packages, `jsr:`/`https:`/`gh:` imports, or V8 JIT throughput.
- **Use `vjs-lite`** when you want fast-starting synchronous automation scripts (`vael:fs`, `vael:ssh`, `vael:template`, `vael:crypto`, `node:child_process`), shared cross-engine tests, or a lightweight embedded JS/TS engine in Rust without V8, Tokio, or Hyper.
