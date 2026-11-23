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
module. Static imports and dynamic `import()` use the same loader. `console.log`
writes to standard output and `console.error` to standard error.

The command maps module and job failures to exit 1 and preflight failures to
exit 2, matching the V8 `neojs` command's classes.

## Limitations

This command does **not** claim parity with `neojs` (V8):

* **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason. Use
  `.js` or `.mjs`.
* **No host asynchrony.** There are no timers or host Promise integration. A
  top-level `await` works only when QuickJS's own job queue can settle it.
* **No built-in `neo:` modules.** Local ECMAScript files only. The `Neo`
  global is introspection only: its list is empty,
  `getBuiltInModule` returns `undefined` for every string name, and a
  non-string name throws `TypeError`.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions, so
`.js`/`.mjs`/extensionless names and the rejection reasons for `.cjs`, JSX/TSX,
declaration files, JSON, WebAssembly, and native addons are the same.

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
