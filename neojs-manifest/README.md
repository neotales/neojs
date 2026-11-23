# neojs-manifest

Foundation for a `neo.jsonc` project manifest.

This crate parses the JSONC dialect, validates the manifest fields this project
has agreed on, and normalizes import specifiers. It does **not** install an
import map into a runtime, expand workspaces, apply `include`/`exclude`, fetch a
package, or implement npm, JSR, GitHub, or Node loading.

See `docs/neo-jsonc.md` in the repository for the exact syntax, status, and open
ambiguities. The public Rust API is documented on `Manifest` and
`normalize_specifier`.
