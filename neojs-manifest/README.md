# neojs-manifest

Foundation for `neo.jsonc` project manifests.

This crate parses the JSONC dialect, validates the manifest fields this project
has agreed on, normalizes import specifiers, discovers manifests upward within
explicit bounds (`discover_manifest`), expands workspace member globs
deterministically (`Workspace::load`), and resolves import-map and workspace
`@scope/pkg` references (`ImportResolver`). It does **not** install hooks into
a JavaScript runtime, apply `include`/`exclude`, fetch packages, or implement
npm, JSR, GitHub, or Node loading.

See `docs/neo-jsonc.md` in the repository for the exact syntax, status, and open
ambiguities. The public Rust API is documented on `Manifest`,
`discover_manifest`, `Workspace`, `ImportResolver`, and `normalize_specifier`.
