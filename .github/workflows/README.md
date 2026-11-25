# GitHub Actions Workflows

All GitHub Actions referenced in `ci.yml` and `release.yml` are pinned by
immutable 40-character commit SHAs to Node 24 releases (`actions/checkout` v7.0.1,
`actions/upload-artifact` v7.0.2, `actions/download-artifact` v8.0.2, and
`Swatinem/rust-cache` v2.9.3).

## `ci.yml`

Runs on pushes to `dev` and `main`, and on pull requests targeting `dev` or `main`.
Stale pull-request runs are cancelled automatically; `dev` and `main` push runs
are never cancelled.

Jobs:
- **fmt-and-lint**: Verifies `eng/size/toolchain` matches `RUST_TOOLCHAIN` (`1.100.0`),
  checks `rustfmt`, runs `clippy -D warnings` across the default workspace,
  standalone `vaeljs-core`, `vaeljs --no-default-features`,
  `vaeljs-builtins --all-features`, `vaeljs-module-policy --all-features`,
  `vaeljs-crypto-core --all-features`, `vaeljs-remote --no-default-features`,
  `vaeljs-npm --all-features`, and `vaeljs-lite-core`/`vaeljs-lite` feature
  combinations; checks `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps`
  across the 14 engine-neutral/Lite crates (`vaeljs-http-core`, `vaeljs-node-core`,
  `vaeljs-template-core`, `vaeljs-dbapi`, `vaeljs-npm`, `vaeljs-dts`,
  `vaeljs-fs-core`, `vaeljs-ssh-core`, `vaeljs-crypto-core`, `vaeljs-manifest`,
  `vaeljs-module-policy`, `vaeljs-remote`, `vaeljs-lite-core`, `vaeljs-lite`) plus
  `vaeljs-remote --no-default-features` and `vaeljs-module-policy --all-features`;
  and runs `shellcheck` on `eng/size/*.sh`.
- **test-v8**: Runs `cargo test --locked` across the default V8 and shared workspace
  crates, runs `cargo test --locked -p vaeljs-core -p vaeljs --no-default-features`,
  runs `cargo run --locked -p vaeljs --bin vjs -- test` over
  `tests/js/shared/*.test.js` and `tests/js/shared/*.test.ts`, and runs
  `cargo run --locked -p vaeljs --bin vjs -- test examples/webapp/app.test.ts`.
- **test-lite**: Runs `cargo test --locked` for `vaeljs-builtins` (`--all-features`
  and `--no-default-features`), `vaeljs-crypto-core --all-features`,
  `vaeljs-remote --no-default-features`, `vaeljs-npm --all-features`,
  `vaeljs-module-policy` (`--no-default-features` and `--all-features`),
  `vaeljs-http-core`, `vaeljs-node-core`, `vaeljs-template-core`, `vaeljs-dbapi`,
  and `vaeljs-lite-core`/`vaeljs-lite` across default and `--no-default-features`
  configurations, plus `cargo run --locked -p vaeljs-lite --bin vjs-lite -- test` over
  `tests/js/shared/*.test.js` and `tests/js/shared/*.test.ts`.
- **engine-isolation**: Verifies via `cargo tree --locked` that:
  1. Default workspace and `vaeljs` graphs exclude `rquickjs` (`rquickjs`, `rquickjs-core`, `rquickjs-sys`).
  2. `vaeljs-lite` excludes `deno_core`, `deno_v8`, and `v8`.
  3. `vaeljs-dts` has zero normal dependencies.
  4. `vaeljs-module-policy --no-default-features` has zero normal dependencies.
  5. `vaeljs-builtins`, `vaeljs-crypto-core`, `vaeljs-dbapi`, `vaeljs-fs-core`, `vaeljs-http-core`, `vaeljs-node-core`, `vaeljs-ssh-core`, and `vaeljs-template-core` normal graphs exclude JS engines and `tokio`.
  6. `vaeljs-remote --no-default-features` and `vaeljs-npm` normal graphs exclude `ureq` and JS engines.
  7. `vaeljs-lite-core --no-default-features` normal graph excludes `deno_ast`, `deno_core`, `deno_v8`, `v8`, `vaeljs-builtins`, `vaeljs-crypto-core`, `vaeljs-dts`, `vaeljs-fs-core`, `vaeljs-manifest`, `vaeljs-node-core`, `vaeljs-ssh-core`, and `vaeljs-template-core`.
- **size-gate**: Populates Cargo metadata (`cargo fetch --locked`) and runs
  `bash eng/size/check.sh`.

### Documented failure modes (`ci.yml`)
- **Toolchain drift**: `eng/size/toolchain` was updated without updating
  `RUST_TOOLCHAIN` in `ci.yml` / `release.yml` (or vice versa). Update all three
  together in one deliberate commit.
- **Offline size-gate failure (`cargo tree failed`)**: `eng/size/measure.sh`
  invokes `cargo tree --offline`; `cargo fetch --locked` must run (or be restored
  from cache) before `bash eng/size/check.sh`.
- **Engine or feature leakage**: Adding an unguarded dependency between `vaeljs*`
  (V8) and `vaeljs-lite*` (QuickJS), or into zero-dep/offline crates, breaks
  `engine-isolation`. Inspect `Cargo.toml` feature wiring and run
  `cargo tree -e normal -p <pkg>`.
- **Rustdoc warning regression**: Broken intra-doc links or bare URLs in the
  documented crates fail `RUSTDOCFLAGS="-D warnings" cargo doc`.

## `release.yml`

Tag-only (`v*`) 6-target release build, packaging, smoke verification, and
GitHub Release publishing workflow. Workflow-level permissions are strictly
`contents: read` so `verify-tag` and all 6 `build` matrix jobs remain
read-only; `contents: write` is granted exclusively to `publish-release` to
create or update the GitHub Release via `GITHUB_TOKEN` (zero external secrets;
no crates.io publishing).

Jobs:
- **verify-tag (`ubuntu-24.04`)**:
  - Checks out full tag history (`fetch-depth: 0`) and rejects lightweight tags
    (`git cat-file -t refs/tags/<tag>` must equal `tag`).
  - Verifies `eng/size/toolchain` matches `RUST_TOOLCHAIN` (`1.100.0`).
  - Verifies via `cargo metadata --no-deps --locked --format-version 1` and `jq`
    that every workspace package has `.version == "${GITHUB_REF_NAME#v}"` and
    `.publish == []` (`publish = false`).
  - Generates canonical V8 and Lite TypeScript declaration trees once via
    `cargo run --locked -p vaeljs-lite --bin vjs-lite -- types --profile v8 --out-dir dist-types/v8`
    and `--profile lite --out-dir dist-types/lite`, uploading `release-types`.
- **build (`needs: verify-tag`, `fail-fast: false`, 6 targets)**:
  - `linux-amd64` (`x86_64-unknown-linux-gnu` on `ubuntu-24.04`, `.tar.gz`, native)
  - `linux-arm64` (`aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm`, `.tar.gz`, native)
  - `macos-arm64` (`aarch64-apple-darwin` on `macos-15`, `.tar.gz`, native)
  - `macos-amd64` (`x86_64-apple-darwin` on `macos-15`, `.tar.gz`, cross)
  - `windows-amd64` (`x86_64-pc-windows-msvc` on `windows-2022`, `.zip`, native)
  - `windows-arm64` (`aarch64-pc-windows-msvc` on `windows-11-arm`, `.zip`, native)
  - Builds `cargo build --release --locked --target <target> -p vaeljs -p vaeljs-lite`
    and emits a `::error` check-run annotation summarizing the tail of the build
    log on failure.
  - Downloads `release-types` and stages two archives per target:
    - `vjs-<tag>-<slug>.{tar.gz,zip}`: `vjs` (or `vjs.exe`), `LICENSE.md`, and `types/` (V8 `.d.ts` tree).
    - `vjs-lite-<tag>-<slug>.{tar.gz,zip}`: `vjs-lite` (or `vjs-lite.exe`), `LICENSE.md`, and `types/` (Lite `.d.ts` tree).
  - Extracts each archive into a clean temporary directory and runs smoke checks:
    - Native targets (`linux-amd64`, `linux-arm64`, `macos-arm64`, `windows-amd64`, `windows-arm64`):
      verifies `LICENSE.md` and `types/index.d.ts`, `--version`, `--help` (exit 0),
      `types --help` (exit 0), `--unknown-flag` (exit 2), and runs all shared JS/TS
      tests (`tests/js/shared/*.test.js` and `tests/js/shared/*.test.ts`) with both
      extracted binaries.
    - `macos-amd64`: verifies `file` reports `Mach-O 64-bit executable x86_64`,
      verifies `otool -L` contains no `/opt/homebrew` or `/usr/local` dylibs,
      checks `LICENSE.md` and `types/index.d.ts`, and runs the native smoke suite
      under `arch -x86_64` when Rosetta 2 is available.
  - Uploads `release-archives-<slug>`.
- **publish-release (`needs: build`, `ubuntu-24.04`, `permissions: contents: write`)**:
  - Checks out the tagged repository (`CHANGELOG.md`), downloads all 12 archives
    into `dist/`, asserts exact count `== 12`, generates sorted `SHA256SUMS.txt`
    via `sha256sum`, verifies `sha256sum -c SHA256SUMS.txt`, and uploads
    `release-bundle-<tag>` (`retention-days: 30`).
  - Extracts the `## [<tag_version>]` section from `CHANGELOG.md` (with a
    concise fallback referencing `CHANGELOG.md` and `SHA256SUMS.txt`), marks
    tags containing `-` with `--prerelease`, idempotently deletes any existing
    release (`gh release delete --yes`, preserving the git tag) before running
    `gh release create --verify-tag` using `GITHUB_TOKEN` with all 13 assets
    (12 archives + `SHA256SUMS.txt`) attached, and verifies `isDraft == false`
    and the attached asset count equals `13`.

### Documented failure modes (`release.yml`)
- **Lightweight tag pushed**: `git cat-file -t refs/tags/<tag>` returns `commit`
  instead of `tag`. Recreate as an annotated tag (`git tag -a vX.Y.Z -m "..."`).
- **Version or `publish = false` mismatch**: Any workspace crate `.version` differs
  from `${GITHUB_REF_NAME#v}` or lacks `publish = false` (`publish != []`).
  Align all workspace manifests before tagging.
- **Cross-compiled target runtime smoke gaps**: If Rosetta 2 is absent on `macos-15`,
  `macos-amd64` falls back to binary-header/`otool` checks and still requires a
  native macOS x86_64 smoke pass before promoting the release.
- **GitHub Release publish or asset-count failure**: Re-running `publish-release`
  is idempotent (`gh release delete --yes` without `--cleanup-tag` removes any
  pre-existing draft or stale-asset release while preserving the git tag, then
  `gh release create --verify-tag` creates a fresh published release), and fails
  if `isDraft` is not `false` or fewer/more than 13 assets are attached.
- **Signing, attestation & crates.io (`system-admin` boundary)**: This workflow
  never signs artifacts, generates checksum attestations, or publishes crates
  (`publish = false` blocks crates.io publishing). Coordinate with `system-admin`
  outside this workflow for signing, attestations, or any future crates.io
  publishing.
