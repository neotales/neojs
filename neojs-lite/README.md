# neojs-lite

A secondary command-line wrapper around `neojs-lite-core`, the embeddable
QuickJS runtime. The primary NeoJS Lite interface is the library; use the
command only when you want to run a module from a shell.

See [`neojs-lite-core/README.md`](../neojs-lite-core/README.md) for embedding,
the runtime lifecycle, and the complete limitations.

## Run a module

```sh
cargo run -p neojs-lite -- hello.ts
```

```ts
const name: string = "neojs-lite";
console.log(`hello from ${name}`);
```

`.js`, `.mjs`, `.ts`, `.mts`, and extensionless files run. TypeScript is stripped
using the same erasable TypeScript stripper as `neojs` (annotations, interfaces,
type aliases, generics, `as`/`satisfies`, `import type`), without type checking;
line and column numbers are preserved in stack traces. Non-erasable constructs
(`enum`, `namespace`, parameter properties) and `.tsx`/`.jsx`/`.cts`/declaration
files are rejected.

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader. Local modules
discover `neo.jsonc` upward for `imports` and `workspaces` (including
enclosing workspace roots when running inside a member directory) unless
`--config <path>` or `--no-config` is passed; remote `https:`, `gh:`, `jsr:`,
and `npm:` targets are rejected because remote loading is not supported in
`neojs-lite`. `console.log` writes to standard output and `console.error` to
standard error.

The command maps module and job failures to exit 1 and preflight, argument, or
invalid/missing manifest failures to exit 2, matching the V8 `neojs` command's
classes.

## Run tests

```sh
cargo run -p neojs-lite -- test 'tests/js/shared/*.test.js' 'tests/js/shared/*.test.ts'
```

`neojs-lite test <file-or-glob>...` expands each argument using the same
deterministic glob rules as `neojs test`, runs each file in a fresh runtime, and
exits `0` when every test passes, `1` on a test/module/unsettled/unhandled
failure or when a file registers no tests, and `2` on startup or glob errors.

## Emit TypeScript declarations

```sh
cargo run -p neojs-lite -- types > neojs-lite.d.ts
cargo run -p neojs-lite -- types --out-dir ./types
cargo run -p neojs-lite -- types --profile v8 > neojs.d.ts
```

`neojs-lite types [--profile <lite|v8>] [--out-dir <dir>]` emits `.d.ts`
declarations for globals (`Neo`, `console`) and enabled `neo:*` built-in
modules from `neojs-dts`, defaulting `--profile` to `lite` (`neojs-lite`).

## Limitations

This command does **not** claim parity with `neojs` (V8):

* **No non-erasable TypeScript, JSX, or type checking.** Only erasable `.ts`
  and `.mts` syntax is supported. Transforms (`enum`, `namespace`, parameter
  properties), `.tsx`, `.jsx`, `.cts`, and declaration files are rejected.
* **No host timers or external asynchrony.** There are no timers and no event
  loop beyond QuickJS's own job queue. A top-level `await` works only when
  QuickJS's own job queue can settle it, and `test(..., { timeout })` is
  rejected explicitly.
* **Most built-in `neo:` modules are absent.** Only the shared `neo:assert`,
  `neo:chars`, `neo:fs`, `neo:os`, `neo:ssh`, `neo:strings`, `neo:test`,
  `neo:uuid`, `neo:uuid/v4`, and `neo:uuid/v7` modules are implemented. Other V8
  modules are not importable, and the `Neo` list does not claim them.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions and
erasable TypeScript stripping, so `.js`/`.mjs`/`.ts`/`.mts`/extensionless names
and the rejection reasons for `.cjs`, `.cts`, JSX/TSX, declaration files, JSON,
WebAssembly, and native addons are the same.

## Build features

The command selects `manifest`, `neo-assert`, `neo-chars`, `neo-fs`, `neo-os`,
`neo-ssh`, `neo-strings`, `neo-test`, `neo-uuid`, and `typescript` explicitly
from `neojs-lite-core` with `default-features = false`, so its advertised
surface is a command decision and does not drift with the library defaults. The
library default enables the eight built-in module features and `typescript`
(`manifest` is opt-in), and a library consumer can opt out completely with
`default-features = false` (which removes `deno_ast` as well as the built-in
module dependencies). Cargo features are additive and unify across the dependency
graph: a build that any dependent enables the feature for contains it. Selection
is not a sandbox.

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile
QuickJS or add `rquickjs` to the default graph. Build this command explicitly:

```sh
cargo build -p neojs-lite
cargo test -p neojs-lite-core -p neojs-lite
```

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `neojs-lite-core/THIRD_PARTY_NOTICES.md`.
