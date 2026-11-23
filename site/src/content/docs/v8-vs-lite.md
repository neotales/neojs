---
title: V8 and Lite compared
description: Which runtime has TypeScript, built-in modules, host asynchrony, and remote loading.
sidebar:
  order: 3
---

NeoJS and NeoJS Lite are separate crates with separate engines. They share executable-extension
policy; they do not share a feature set, and Lite does not chase parity with V8.

## Feature table

| Feature | V8 (`neojs`, `neojs-core`) | Lite (`neojs-lite`, `neojs-lite-core`) |
| --- | --- | --- |
| Engine | V8 via `deno_core` | QuickJS via `rquickjs` |
| `.js` and `.mjs` modules | Yes | Yes |
| Extensionless modules | Yes | Yes |
| Relative, absolute, and `file:` imports | Yes | Yes |
| Static and dynamic `import()` | Yes | Yes |
| Top-level `await` | Yes | Only when QuickJS's own job queue can settle it |
| `.ts` / `.mts` | Yes, type stripping only | **No** |
| Type checking | **No** | **No** |
| Built-in `neo:` modules | Thirteen modules | **No** |
| `Neo.getBuiltInModule` | Yes | **No** |
| Host asynchrony | The runtime drives async ops and top-level `await`; there is no user-facing timer API | **No** timers or host Promise integration |
| Remote `https:` loading | Opt-in in the V8 CLI/runtime | **Not implemented** |
| `neojs test` built-in runner | Yes | **No** |
| Permissions or sandbox | **None** | **None** |
| Windows | Unverified | Unverified |
| Verified platform | Linux x86-64 | Linux x86-64 |

## Shared extension policy

Both runtimes use `neojs-module-policy` for executable-extension decisions. `.js`, `.mjs`, and
extensionless names are accepted; `.cjs`, JSX/TSX, declaration files, JSON, WebAssembly, and native
addons fail with the same explanation in either runtime.

That shared policy is about **file extensions**, not language features. Lite still rejects
TypeScript and built-in modules.

## What Lite does not add

There is no compatibility layer to make a V8 module run on Lite. A module that imports `crypto`,
uses timers, or relies on host async work needs a V8 runtime. A module that only uses ECMAScript and
local imports can run on either.

## What neither runtime adds

Neither runtime is Node.js. Both reject `node:`, `npm:`, `jsr:`, `http:`, `data:`, and `blob:`
imports. Neither has a permission system or a sandbox, a REPL, stdin input, workers, `fetch`, `URL`,
or `TextEncoder` as a documented surface.

See [Limits and unsupported features](/limits/).
