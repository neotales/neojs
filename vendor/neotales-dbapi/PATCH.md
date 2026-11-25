# Vendored `neotales-dbapi` patch

This directory is a copy of the `neotales-dbapi` crate, pinned at **0.0.0-alpha.0**, from
the crates.io release:

- crate checksum: `a7d9dbc2eb79b7ae0f0e5f504a3e966118905516f931afa4a5ead135f98ef653`
- upstream git revision recorded by the crate: `1e78a92c0b92dfc484feb9079bc374304e0be347`
- upstream path in the repository: `dbapi`
- license: MIT (see `LICENSE.md`)

It is vendored only to add a narrow public seam on `Row` in `src/lib.rs`:

- `Row::columns(&self) -> &[(String, Value)]`
- `Row::column_name(&self, ordinal: usize) -> Option<&str>`

`neotales_dbapi::Row` already stores `columns: Vec<(String, Value)>` privately and exposes
by-name and by-ordinal cell reads, but does not expose column names by ordinal or iteration.
The `neo:db` JavaScript adapter needs column names in ordinal order to materialize
`Record<string, unknown>` row objects without hardcoding column lists.

No provider, pooling, parameter, validation, or error-redaction behaviour is changed. The
replacement is applied in the workspace `Cargo.toml` with `[patch.crates-io]`.

Remove this directory and the `[patch.crates-io]` entry as soon as an upstream
`neotales-dbapi` release exposes `Row::columns` / `Row::column_name` (or an equivalent
public API).
