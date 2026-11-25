# neojs-dts

Engine-neutral TypeScript declaration (`.d.ts`) bundles for `neojs` (V8) and
`neojs-lite` (QuickJS) runtime profiles.

This crate has zero JavaScript engine dependencies (`deno_core`, `v8`, and
`rquickjs` are absent) and zero runtime dependencies beyond `std`.

```sh
cargo test --locked -p neojs-dts
cargo clippy --locked -p neojs-dts --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p neojs-dts --no-deps
cargo tree --locked -p neojs-dts -e normal
```

## Profiles

* `Profile::V8` (`neojs`, `Neo.engine === "neo"`):
  * `console`: `log`, `debug`, `info`, `warn`, `error`
  * 18 `neo:*` built-in modules: `neo:age`, `neo:assert`, `neo:chars`,
    `neo:console`, `neo:cron`, `neo:crypto`, `neo:fs`, `neo:http`, `neo:os`,
    `neo:path`, `neo:slices`, `neo:ssh`, `neo:stdio`, `neo:strings`, `neo:test`,
    `neo:uuid`, `neo:uuid/v4`, `neo:uuid/v7`
* `Profile::Lite` (`neojs-lite`, `Neo.engine === "neo-lite"`):
  * `console`: `log`, `debug`, `info`, `warn`, `error`
  * 14 `neo:*` built-in modules: `neo:assert`, `neo:chars`, `neo:console`,
    `neo:crypto` (13 synchronous exports only), `neo:fs`, `neo:os`, `neo:path`,
    `neo:slices`, `neo:ssh`, `neo:strings`, `neo:test`, `neo:uuid`,
    `neo:uuid/v4`, `neo:uuid/v7`

## Public API

* `files(profile)` — embedded `.d.ts` files (`index.d.ts`, `globals.d.ts`, and
  `neo/*.d.ts`) for release archive `types/` staging and LSP virtual URIs.
* `globals_dts(profile)` and `module_dts(profile, specifier)` — direct lookup
  of ambient globals or a single built-in module declaration.
* `render_bundle(profile)` — deterministic single-file `.d.ts` bundle for
  `neojs types` stdout.
* `write_types_dir(profile, dir)` — writes the `types/` directory layout to
  disk.
* `expected_exports(profile)` and `parse_declared_exports(dts)` — machine-checked
  export lists for CI drift verification against live `Neo.getBuiltInModulesList()`
  and `Object.keys(moduleNamespace)`.
