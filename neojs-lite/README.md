# neojs-lite

A secondary command-line wrapper around `neojs-lite-core`, the embeddable
QuickJS runtime. The primary NeoJS Lite interface is the library; use the
command only when you want to run a module from a shell.

See [`neojs-lite-core/README.md`](../neojs-lite-core/README.md) for embedding,
the runtime lifecycle, and the complete limitations.

## Run a module

```sh
cargo run -p neojs-lite -- hello.js
```

```js
const name = "neojs-lite";
console.log(`hello from ${name}`);
```

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
cargo run -p neojs-lite -- test 'tests/js/shared/*.test.js'
```

`neojs-lite test <file-or-glob>...` expands each argument using the same
deterministic glob rules as `neojs test`, runs each file in a fresh runtime, and
exits `0` when every test passes, `1` on a test/module/unsettled/unhandled
failure or when a file registers no tests, and `2` on startup or glob errors.

## Limitations

This command does **not** claim parity with `neojs` (V8):

* **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason. Use
  `.js` or `.mjs`.
* **No host timers or external asynchrony.** There are no timers and no event
  loop beyond QuickJS's own job queue. A top-level `await` works only when
  QuickJS's own job queue can settle it, and `test(..., { timeout })` is
  rejected explicitly.
* **Most built-in `neo:` modules are absent.** Only the shared `neo:assert`,
  `neo:chars`, `neo:os`, `neo:strings`, `neo:test`, `neo:uuid`, `neo:uuid/v4`, and
  `neo:uuid/v7` modules are implemented. Other V8 modules are not importable,
  and the `Neo` list does not claim them.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions, so
`.js`/`.mjs`/extensionless names and the rejection reasons for `.cjs`, JSX/TSX,
declaration files, JSON, WebAssembly, and native addons are the same.

## Build features

The command selects `manifest`, `neo-assert`, `neo-chars`, `neo-os`,
`neo-strings`, `neo-test`, and `neo-uuid` explicitly from `neojs-lite-core` with
`default-features = false`, so its advertised surface is a command decision and
does not drift with the library defaults. The library default enables the six
built-in modules (`manifest` is opt-in), and a library consumer can opt out
completely with `default-features = false`. Cargo features are additive and
unify across the dependency graph: a build that any dependent enables the module
for contains it. Selection is not a sandbox.

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
