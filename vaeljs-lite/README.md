# vaeljs-lite

A secondary command-line wrapper around `vaeljs-lite-core`, the embeddable
synchronous QuickJS runtime (zero `tokio`, zero `hyper`, zero V8). The primary
VaelJS Lite interface is the library; use the command when you want to run a
module or test suite from a shell.

See [`vaeljs-lite-core/README.md`](../vaeljs-lite-core/README.md) for embedding,
the runtime lifecycle, security invariants, and the complete limitations.

## Run a module or URL

```sh
cargo run -p vaeljs-lite --bin vjs-lite -- hello.ts
cargo run -p vaeljs-lite --bin vjs-lite -- --allow-remote https://example.test/mod.ts
cargo run -p vaeljs-lite --bin vjs-lite -- --allow-remote jsr:@scope/pkg@^1.0.0
```

```ts
const name: string = "vaeljs-lite";
console.log(`hello from ${name}`);
```

`.js`, `.mjs`, `.ts`, `.mts`, and extensionless files run. TypeScript is stripped
using the same erasable TypeScript stripper as `vaeljs` (annotations, interfaces,
type aliases, generics, `as`/`satisfies`, `import type`), without type checking;
line and column numbers are preserved in stack traces. Non-erasable constructs
(`enum`, `namespace`, parameter properties) and `.tsx`/`.jsx`/`.cts`/declaration
files are rejected.

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader.

* **Manifest discovery (`vael.jsonc`):** Local modules discover `vael.jsonc`
  upward for `imports` (local paths, `file:`, `https:`, `gh:`/`github:`, `jsr:`,
  `npm:`, and `node:`) and `workspaces` (including enclosing workspace roots when
  running inside a member directory) unless `--config <path>` or `--no-config` is
  passed.
* **Remote loading (`--allow-remote` / `VAELJS_ALLOW_REMOTE`):** Offline by
  default. `--allow-remote` (or `VAELJS_ALLOW_REMOTE=1`/`true`) enables `https:`,
  `gh:`/`github:`, and `jsr:` entrypoints (`vjs-lite --allow-remote <url>`) and
  imports, plus online `npm:` registry fetches. `--no-allow-remote` forces remote
  loading off, overriding `VAELJS_ALLOW_REMOTE`. Plaintext `http://` and
  credentialed URLs are rejected.
* **npm & `package.json`:** Resolves `npm:` specifiers, nearest `package.json`
  (`dependencies`, `exports`, `#imports`, peer deps), `vael.lock`, and `.npmrc`
  scoped registries + HTTPS-only Bearer auth via the verified CAS cache with zero
  `node_modules` and zero lifecycle scripts. Control policy with `--no-npm`,
  `--frozen-lockfile`, and `--offline`.
* **Node compatibility:** Built-in `node:*` compatibility modules (`node:buffer`,
  `node:child_process`, `node:crypto`, `node:fs`, `node:os`, `node:path`,
  `node:path/posix`, `node:path/win32`, `node:process`, `node:url`) are enabled
  unless `--no-node-compat` is passed.

`console.log`, `console.debug`, and `console.info` write to standard output and
`console.warn` and `console.error` to standard error.

The command maps module and job failures to exit 1 and preflight, argument, or
invalid/missing manifest failures to exit 2, matching the V8 `vaeljs` command's
classes.

## Run tests

```sh
cargo run -p vaeljs-lite --bin vjs-lite -- test 'tests/js/shared/*.test.js' 'tests/js/shared/*.test.ts'
cargo run -p vaeljs-lite --bin vjs-lite -- test --allow-remote --offline 'apps/**/*.test.ts'
```

`vjs-lite test <file-or-glob>...` expands each argument using the same
deterministic glob rules as `vjs test`, runs each file in a fresh runtime, and
exits `0` when every test passes, `1` on a test/module/unsettled/unhandled
failure or when a file registers no tests, and `2` on startup or glob errors.
`--allow-remote`, `--no-allow-remote`, `--no-npm`, `--frozen-lockfile`,
`--offline`, `--no-node-compat`, `--config <path>`, and `--no-config` may appear
before or after `test`.

## Emit TypeScript declarations

```sh
cargo run -p vaeljs-lite --bin vjs-lite -- types > vjs-lite.d.ts
cargo run -p vaeljs-lite --bin vjs-lite -- types --out-dir ./types
cargo run -p vaeljs-lite --bin vjs-lite -- types --profile v8 > vjs.d.ts
```

`vjs-lite types [--profile <lite|v8>] [--out-dir <dir>]` emits `.d.ts`
declarations for globals (`Vael`, `console`) and enabled `vael:*` built-in
modules from `vaeljs-dts`, defaulting `--profile` to `lite` (`vaeljs-lite`).

## Limitations

This command does **not** claim parity with `vaeljs` (V8):

* **No non-erasable TypeScript, JSX, or type checking.** Only erasable `.ts`
  and `.mts` syntax is supported. Transforms (`enum`, `namespace`, parameter
  properties), `.tsx`, `.jsx`, `.cts`, and declaration files are rejected.
* **No host timers or external asynchrony.** There are no timers and no event
  loop beyond QuickJS's own job queue. A top-level `await` works only when
  QuickJS's own job queue can settle it, and `test(..., { timeout })` is
  rejected explicitly.
* **Most V8-only async/platform `vael:` modules are absent.** Only the shared
  `vael:assert`, `vael:chars`, `vael:console`, `vael:crypto` (13 synchronous exports;
  no `*Async` helpers), `vael:fs`, `vael:os`, `vael:path`, `vael:slices`, `vael:ssh`,
  `vael:strings`, `vael:template`, `vael:test`, `vael:uuid`, `vael:uuid/v4`, and
  `vael:uuid/v7` modules and synchronous `node:*` compatibility modules are
  implemented. Other V8 modules (`vael:age`, `vael:cron`, `vael:db`, `vael:http`,
  `vael:stdio`, `node:fs/promises`) are not importable, and the `Vael` list does
  not claim them.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `vaeljs-module-policy` for executable-extension decisions and
erasable TypeScript stripping, so `.js`/`.mjs`/`.ts`/`.mts`/extensionless names
and the rejection reasons for `.cjs`, `.cts`, JSX/TSX, declaration files, JSON,
WebAssembly, and native addons are the same.

## Build features

`vaeljs-lite` selects its base modules (`manifest`, `vael-assert`,
`vael-chars`, `vael-console`, `vael-crypto`, `vael-fs`, `vael-os`, `vael-path`,
`vael-slices`, `vael-ssh`, `vael-strings`, `vael-test`, `vael-uuid`,
`typescript`) explicitly from `vaeljs-lite-core` with `default-features = false`
and exposes opt-outable `default = ["vael-template", "node-compat", "remote", "npm"]`
features (`--no-default-features` excludes `vaeljs-template-core`,
`vaeljs-node-core`, `vaeljs-remote`, and `vaeljs-npm`, and causes remote/npm
CLI flags to fail cleanly with a feature-disabled startup error).

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile
QuickJS or add `rquickjs` to the default graph. Build this command explicitly:

```sh
cargo build --offline --locked -p vaeljs-lite
cargo test --offline --locked -p vaeljs-lite-core -p vaeljs-lite
```

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `vaeljs-lite-core/THIRD_PARTY_NOTICES.md`.
