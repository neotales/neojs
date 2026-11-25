---
title: Modules Overview
description: Built-in vael:* and node:* module availability, the Vael global, and TypeScript declaration generation across vjs and vjs-lite.
---

VaelJS exposes a canonical, explicit module surface across both the full V8 runtime (`vjs`) and the QuickJS runtime (`vjs-lite`). Built-in modules use the `vael:*` scheme, and opt-outable Node.js automation compatibility modules use the `node:*` scheme.

## Canonical `vael:*` Specifier Rules

Every VaelJS built-in module requires its exact `vael:*` specifier (for example `"vael:fs"` or `"vael:uuid/v4"`). Bare specifiers such as `"fs"`, `"path"`, or `"crypto"` are never silently aliased to `vael:*` modules.

```ts
import { readTextFile } from "vael:fs";
import { v7 } from "vael:uuid/v7";

const id = v7();
const config = readTextFile("./vael.jsonc");
console.log(id, config.length);
```

## The `Vael` and `console` Globals

Both runtimes install only two ambient globals — `Vael` and `console` — with no browser or legacy host globals.

| Global | Property / Method | Description |
| --- | --- | --- |
| `Vael` | `engine` | Readonly runtime identifier: `"vael"` in `vjs`, `"vael-lite"` in `vjs-lite`. |
| `Vael` | `getBuiltInModulesList()` | Returns a frozen `readonly string[]` of enabled canonical `vael:*` specifiers in registration order. |
| `Vael` | `getBuiltInModule(specifier)` | Returns the cached frozen module object for a registered `vael:*` specifier, or `undefined` if absent. Throws `TypeError` if `specifier` is not a string. |
| `console` | `log`, `debug`, `info` | Format arguments separated by spaces to stdout, followed by a newline. |
| `console` | `warn`, `error` | Format arguments separated by spaces to stderr, followed by a newline. |

:::note
`Vael.getBuiltInModulesList()` and `Vael.getBuiltInModule()` cover only `vael:*` built-in modules (`20` in `vjs`, `15` in `vjs-lite`). Compatibility modules under `node:*` are imported via `import ... from "node:*"` and are not listed in `Vael.getBuiltInModulesList()`.
:::

For `"vael:assert"` and `"vael:console"`, `Vael.getBuiltInModule()` returns the same value as their default export (the callable `assert` function and `console` object, respectively). For `"vael:template"` and `"vael:test"`, it returns the module object with named exports only.

## Generating TypeScript Declarations (`types`)

Both `vjs` and `vjs-lite` embed profile-accurate `.d.ts` declarations (`vaeljs-dts`) for IDE support and type checking.

```sh
# Emit a single self-contained .d.ts bundle to stdout
vjs types > vjs.d.ts
vjs-lite types > vjs-lite.d.ts

# Cross-generate the other profile's declarations
vjs types --profile lite > vjs-lite.d.ts
vjs-lite types --profile v8 > vjs.d.ts

# Materialize the full types/ directory (index.d.ts, globals.d.ts, vael/*, node/*)
vjs types --out-dir ./types
```

`types` does not accept runtime policy flags (`--allow-remote`, `--no-allow-remote`, `--no-node-compat`, `--config`, `--no-config`, `--no-npm`, `--frozen-lockfile`, or `--offline`).

## Module Availability Matrix

By default, `vjs` provides **31 modules** (`20` `vael:*` + `11` `node:*`) and `vjs-lite` provides **25 modules** (`15` `vael:*` + `10` `node:*`).

### `vael:*` Built-in Modules

| Specifier | `vjs` (V8) | `vjs-lite` (QuickJS) | Feature / Notes | Reference |
| --- | --- | --- | --- | --- |
| `vael:age` | Yes | No | Binary age v1 X25519 encryption | [`vael:age`](/modules/vael-age/) |
| `vael:assert` | Yes | Yes | Assertions + `rejects` / `doesNotReject` | [`vael:test` & `vael:assert`](/modules/vael-testing/) |
| `vael:chars` | Yes | Yes | Unicode scalar classification & case | [`vael:strings`, `vael:chars`, `vael:slices`](/modules/vael-text/) |
| `vael:console` | Yes | Yes | Mirrors global `console` | [Utilities](/modules/vael-utilities/) |
| `vael:cron` | Yes | No | UTC 5-field cron scheduler & `next` | [Utilities](/modules/vael-utilities/) |
| `vael:crypto` | Sync + Async (20) | Sync only (13) | 7 `*Async` pool ops are V8-only | [`vael:crypto`](/modules/vael-crypto/) |
| `vael:db` | Yes | No | `vael-db` feature (SQLite by default) | [`vael:db`](/modules/vael-db/) |
| `vael:fs` | Yes | Yes | 14 synchronous filesystem ops | [`vael:fs`](/modules/vael-fs/) |
| `vael:http` | Yes | No | `vael-http` feature (`ServeMux`, `serve`) | [`vael:http`](/modules/vael-http/) |
| `vael:os` | Yes | Yes | Build-target `platform` & `arch` | [`vael:os` & `vael:path`](/modules/vael-os-path/) |
| `vael:path` | Yes | Yes | Lexical `posix` & `win32` path tools | [`vael:os` & `vael:path`](/modules/vael-os-path/) |
| `vael:slices` | Yes | Yes | Zero-copy `Slice` & `CharSlice` views | [`vael:strings`, `vael:chars`, `vael:slices`](/modules/vael-text/) |
| `vael:ssh` | Yes | Yes | Synchronous SSH-2 client | [`vael:ssh`](/modules/vael-ssh/) |
| `vael:stdio` | Yes | No | Bounded `Readable`, `Writable`, `stdout`/`stderr` | [Utilities](/modules/vael-utilities/) |
| `vael:strings` | Yes | Yes | Unicode 16.0 case-folded search & trim | [`vael:strings`, `vael:chars`, `vael:slices`](/modules/vael-text/) |
| `vael:template` | Yes | Yes | `vael-template` feature (Go + Sprig) | [`vael:template`](/modules/vael-template/) |
| `vael:test` | Yes | Yes | `timeout` option is V8-only | [`vael:test` & `vael:assert`](/modules/vael-testing/) |
| `vael:uuid` | Yes | Yes | RFC 9562 `v4` & `v7` | [Utilities](/modules/vael-utilities/) |
| `vael:uuid/v4` | Yes | Yes | Subpath re-export of `v4` | [Utilities](/modules/vael-utilities/) |
| `vael:uuid/v7` | Yes | Yes | Subpath re-export of `v7` | [Utilities](/modules/vael-utilities/) |

### `node:*` Compatibility Modules

All `node:*` modules require the `node-compat` Cargo feature (enabled by default) and can be disabled at runtime with `--no-node-compat`. See [Node Compatibility (`node:*`)](/modules/node-compat/) for exact export tables.

| Specifier | `vjs` (V8) | `vjs-lite` (QuickJS) | Notes |
| --- | --- | --- | --- |
| `node:buffer` | Yes | Yes | `Buffer` subclass of `Uint8Array` |
| `node:child_process` | Sync + `execFile` | Sync only | `execFile` async is V8-only |
| `node:crypto` | Yes | Yes | 6 sync functions |
| `node:fs` | Sync + `promises` | Sync only | `promises` property is V8-only |
| `node:fs/promises` | Yes | No | Async filesystem promises (V8-only) |
| `node:os` | Yes | Yes | Node-vocabulary OS helpers |
| `node:path` | Yes | Yes | Host dialect + `.posix` / `.win32` |
| `node:path/posix` | Yes | Yes | POSIX dialect |
| `node:path/win32` | Yes | Yes | Win32 dialect |
| `node:process` | Yes | Yes | `argv`, `cwd`, `env`, `exit`, `hrtime` |
| `node:url` | Yes | Yes | `URL`, `URLSearchParams`, `fileURLToPath`, `pathToFileURL` |
