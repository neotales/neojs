# vaeljs-dts

Engine-neutral TypeScript declaration (`.d.ts`) bundles for `vaeljs` (V8) and
`vaeljs-lite` (QuickJS) runtime profiles.

This crate has zero JavaScript engine dependencies (`deno_core`, `v8`, and
`rquickjs` are absent) and zero runtime dependencies beyond `std`.

```sh
cargo test --locked -p vaeljs-dts
cargo clippy --locked -p vaeljs-dts --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p vaeljs-dts --no-deps
cargo tree --locked -p vaeljs-dts -e normal
```

## Profiles

* `Profile::V8` (`vaeljs`, `Vael.engine === "vael"`):
  * `console`: `log`, `debug`, `info`, `warn`, `error`
  * 20 `vael:*` built-in modules: `vael:age`, `vael:assert`, `vael:chars`,
    `vael:console`, `vael:cron`, `vael:crypto`, `vael:db`, `vael:fs`, `vael:http`,
    `vael:os`, `vael:path`, `vael:slices`, `vael:ssh`, `vael:stdio`, `vael:strings`,
    `vael:template`, `vael:test`, `vael:uuid`, `vael:uuid/v4`, `vael:uuid/v7`
* `Profile::Lite` (`vaeljs-lite`, `Vael.engine === "vael-lite"`):
  * `console`: `log`, `debug`, `info`, `warn`, `error`
  * 15 `vael:*` built-in modules: `vael:assert`, `vael:chars`, `vael:console`,
    `vael:crypto` (13 synchronous exports only), `vael:fs`, `vael:os`, `vael:path`,
    `vael:slices`, `vael:ssh`, `vael:strings`, `vael:template`, `vael:test`,
    `vael:uuid`, `vael:uuid/v4`, `vael:uuid/v7`

## Public API

* `files(profile)` — embedded `.d.ts` files (`index.d.ts`, `globals.d.ts`, and
  `vael/*.d.ts`) for release archive `types/` staging and LSP virtual URIs.
* `globals_dts(profile)` and `module_dts(profile, specifier)` — direct lookup
  of ambient globals or a single built-in module declaration.
* `render_bundle(profile)` — deterministic single-file `.d.ts` bundle for
  `vjs types` stdout.
* `write_types_dir(profile, dir)` — writes the `types/` directory layout to
  disk.
* `expected_exports(profile)` and `parse_declared_exports(dts)` — machine-checked
  export lists for CI drift verification against live `Vael.getBuiltInModulesList()`
  and `Object.keys(moduleNamespace)`.
