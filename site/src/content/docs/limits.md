---
title: Limits and unsupported features
description: What NeoJS implements, what it deliberately does not, and which gaps are shared with NeoJS Lite.
---

This page exists so a missing feature is not mistaken for a bug. When this page and the source
disagree, the source wins.

## Implemented in the V8 runtime

- ECMAScript modules: static imports, dynamic `import()`, relative, absolute, and `file:` URLs.
- Erasable TypeScript in `.ts` and `.mts`, with no type checking.
- Top-level `await`.
- Opt-in HTTPS module loading with explicit resource limits.
- Console output.
- `neojs test` over multiple files and globs.
- The built-in modules `age`, `assert`, `chars`, `console`, `crypto`, `os`, `path`, `slices`, `ssh`,
  `stdio`, `strings`, `test`, and `uuid`.
- The `Neo` built-in registry.
- The Rust embedding API.

## Implemented in NeoJS Lite

- `.js`, `.mjs`, and extensionless ECMAScript modules.
- Relative imports, absolute paths, and `file:` URLs.
- Static imports and dynamic `import()`.
- Top-level `await` only when QuickJS's own job queue can settle it.
- Bounded job draining after the entry settles.

## Not implemented in either runtime

- **Node.js compatibility.** `node:`, `npm:`, `jsr:`, `http:`, `data:`, and `blob:` imports are
  rejected. `node:test` is not an alias for `test`.
- **CommonJS.** `.cjs` and `.cts` are rejected by extension. A `.js` file that uses `require`,
  `module`, or `exports` fails as JavaScript.
- **A permission system or sandbox.** The loader reads any file the process can read. Test files run
  with the process's full filesystem authority. `Deno.core` internals are reachable but unsupported.
- **A REPL or stdin input.**
- **Workers.**
- **`fetch`, `URL`, or `TextEncoder` as documented NeoJS APIs.** V8 supplies the ECMAScript
  built-ins; NeoJS does not add these host APIs.
- **Import maps, module type attributes, and bare specifiers that are not built-ins.**
- **JSON and WebAssembly module files**, both as the entry and as imports. V8's `WebAssembly` global
  is present but untested.
- **Windows verification.** Only Linux x86-64 is verified in the repository.
- **A timeout or interrupt.** A JavaScript loop that never exits hangs the process.

## Not implemented in the V8 runtime

- **Type checking and transpilation.** Only syntax that can be erased is accepted.
- **`.tsx`, `.jsx`, `.cts`, `.d.ts`, `.d.mts`, `.d.cts`.** Rejected by extension.
- **`enum`, `namespace`, `module` blocks, parameter properties, `import value = require(...)`,
  `export =`, angle-bracket assertions (`<T>value`), and decorators.** Rejected by the type
  stripper.
- **Remote loading by default.** It is opt-in, and `http:` is always rejected.

## Not implemented in NeoJS Lite

- **TypeScript.** `.ts` and `.mts` are rejected with a clear reason.
- **Host asynchrony and timers.**
- **Built-in `neo:` modules.**
- **Remote HTTPS loading.**
- **The `neojs test` built-in runner.**

## TypeScript details

There is **no type checking**. A type error is not reported and the program runs with whatever
values it has, so `const value: number = "text"` prints `text`. Type stripping is not
transpilation: nothing is downlevelled, and only syntax that can be erased is accepted.

Erased text is replaced by spaces, so a stack trace from a `.ts` file points at the original line
and column. A rejection names the file and, where the stripper reports one, the line and column.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The module finished, or every selected test file ran with no failures |
| `1` | The module failed to compile, resolve imports, link, or run; or a test failed, was pending, left an unhandled rejection or unsettled event-loop work, or the file registered no tests |
| `2` | NeoJS could not start: bad arguments, an unreadable or unsupported entry file, or a test file or glob that could not be expanded |

## What is shared

Both runtimes share `neojs-module-policy` for executable-extension decisions. `.js`, `.mjs`, and
extensionless names are accepted; `.cjs`, JSX/TSX, declaration files, JSON, WebAssembly, and native
addons fail with the same explanation in either runtime.

See [V8 and Lite compared](/v8-vs-lite/) for the runtime table.
