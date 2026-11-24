# GitHub Actions Workflows

## `ci.yml`

Runs on pushes to `dev` and `main`, and on pull requests targeting `dev` or `main`.
Stale pull-request runs are cancelled automatically; `dev` and `main` push runs
are never cancelled.

Jobs:
- **fmt-and-lint**: Verifies `eng/size/toolchain` matches `RUST_TOOLCHAIN` (`1.100.0`),
  checks `rustfmt`, runs `clippy -D warnings` across workspace/builtins/lite feature
  combinations, and runs `shellcheck` on `eng/size/*.sh`.
- **test-v8**: Runs `cargo test --locked` and, when `tests/js/shared/*.test.js`
  or `tests/js/shared/*.test.ts` exist, runs `cargo run --locked -p neojs -- test`
  over those shared files.
- **test-lite**: Runs `neojs-builtins`, `neojs-lite-core`, and `neojs-lite` tests
  across default and `--no-default-features` configurations, plus `neojs-lite`
  shared JS/TS tests once `tests/js/shared/*.test.{js,ts}` and the `neojs-lite test`
  subcommand exist.
- **engine-isolation**: Verifies via `cargo tree --locked` that the default
  workspace and `neojs` graphs contain no `rquickjs`, that `neojs-lite` contains
  no `deno_core`/`deno_v8`/`v8`, and that `neojs-lite-core --no-default-features`
  omits `deno_ast` and `neojs-builtins` as well.
- **size-gate**: Populates Cargo metadata (`cargo fetch --locked`) and runs
  `bash eng/size/check.sh`.

### Documented failure modes
- **Toolchain drift**: `eng/size/toolchain` was updated without updating
  `RUST_TOOLCHAIN` in `ci.yml` / `release.yml` (or vice versa). Update all three
  together in one deliberate commit.
- **Offline size-gate failure (`cargo tree failed`)**: `eng/size/measure.sh`
  invokes `cargo tree --offline`; `cargo fetch --locked` must run (or be restored
  from cache) before `bash eng/size/check.sh`.
- **Engine leakage**: Adding an unguarded dependency between `neojs*` (V8) and
  `neojs-lite*` (QuickJS) breaks `engine-isolation`. Check `Cargo.toml` feature
  wiring and run `cargo tree -p <pkg>`.
- **Workspace `--no-default-features` not used for `neojs-core`**:
  `neojs-core/tests/jsr.rs` and `neojs-core/tests/remote.rs` currently import
  `neo:strings` without `#[cfg(feature = "neo-strings")]`, so full-package
  `cargo test -p neojs-core --no-default-features` is not supported yet.

## `release.yml`

Tag-only (`v*`) build and verification skeleton with `contents: read` only (no
secrets, signing keys, or publish tokens).

### Documented failure modes
- **Lightweight tag pushed**: `git cat-file -t refs/tags/<tag>` returns `commit`
  instead of `tag`. Recreate as an annotated tag (`git tag -a vX.Y.Z -m "..."`).
- **Version mismatch**: `neojs` / `neojs-lite` package versions in `Cargo.toml`
  do not match `${GITHUB_REF_NAME#v}`. Bump workspace version before tagging.
- **Signing & publishing**: Not handled by this workflow; coordinate with
  `system-admin` for artifact signing and registry/release publishing.
