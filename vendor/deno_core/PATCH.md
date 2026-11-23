# Vendored `deno_core` patch

This directory is a copy of the `deno_core` crate, pinned at **0.413.0**, from
the crates.io release:

- crate checksum: `7ad7d5294f640fd99db08c455e2453c9860b1fa728b76fbdacf0eed3e308d48a`
- upstream git revision recorded by the crate: `fe7ed012d92c642fd8e5607ac5a45d517538b5c5`
- upstream path in the repository: `libs/core`
- license: MIT. The crates.io package does not include a standalone license file;
  every Rust source header says `Copyright 2018-2026 the Deno authors. MIT
  license.` The `LICENSE` file here uses that copyright line and the standard
  MIT permission text, which is present verbatim in the cached Deno sibling
  crates `deno_error` 0.7.1 and `deno_ast` 0.53.3. No license text was
  downloaded or invented.

It is vendored only to add the narrow public seam
`JsRuntime::register_native_module` in `runtime/jsruntime.rs`. That method uses
the crate's existing `ModuleMap::new_synthetic_module` to register an eagerly
evaluated synthetic ECMAScript module with explicit named exports derived from
native V8 values. It returns an error before touching the module map when the
specifier is already registered under any requested module type, has an existing
`synthetic_esm` mapping, has a known or residual lazy ESM source, or the export
list contains a duplicate name. It changes no loader, resolver, cache, or
module-graph behaviour. The replacement is applied in the workspace
`Cargo.toml` with `[patch.crates-io]`.

Remove this directory and the `[patch.crates-io]` entry as soon as an upstream
`deno_core` release exposes an equivalent public API. The local Rust diff is:

* `runtime/jsruntime.rs`: `register_native_module` and test-only lazy-ESM,
  JSON-name, and JSON-alias helpers gated by the `native_module_testing`
  feature;
* `modules/module_map_data.rs` and `modules/map.rs`: `has_name_any_type`, which
  checks the by-name table under every requested module type, plus a small
  `ModuleMap::alias` wrapper for the collision test;
* `Cargo.toml`: the `native_module_testing` feature.

The added `LICENSE` documents the upstream MIT terms. `neojs-core` enables
`native_module_testing` only as a dev-dependency; normal builds do not expose
the helpers.
