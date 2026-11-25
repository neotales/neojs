# GitHub Actions Workflows

## `ci.yml`

Runs on pushes to `dev` and `main`, and on pull requests targeting `dev` or `main`.
Stale pull-request runs are cancelled automatically; `dev` and `main` push runs
are never cancelled.

Jobs:
- **fmt-and-lint**: Verifies `eng/size/toolchain` matches `RUST_TOOLCHAIN` (`1.100.0`),
  checks `rustfmt`, runs `clippy -D warnings` across the default workspace,
  standalone `neojs-core`, `neojs-builtins --all-features`, `neojs-module-policy --all-features`,
  `neojs-crypto-core --all-features`, `neojs-remote --no-default-features`,
  `neojs-npm --all-features`, and `neojs-lite-core`/`neojs-lite` feature
  combinations; checks `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps`
  across the 10 engine-neutral/Lite crates (`neojs-crypto-core`, `neojs-dbapi`,
  `neojs-dts`, `neojs-fs-core`, `neojs-manifest`, `neojs-module-policy`,
  `neojs-npm`, `neojs-ssh-core`, `neojs-lite-core`, `neojs-lite`) plus
  `neojs-remote --no-default-features` and `neojs-module-policy --all-features`;
  and runs `shellcheck` on `eng/size/*.sh`.
- **test-v8**: Runs `cargo test --locked` across the default V8 and shared workspace
  crates and runs `cargo run --locked -p neojs -- test` over
  `tests/js/shared/*.test.js` and `tests/js/shared/*.test.ts`.
- **test-lite**: Runs `cargo test --locked` for `neojs-builtins` (`--all-features`
  and `--no-default-features`), `neojs-crypto-core --all-features`,
  `neojs-remote --no-default-features`, `neojs-npm --all-features`,
  `neojs-module-policy` (`--no-default-features` and `--all-features`), and
  `neojs-lite-core`/`neojs-lite` across default and `--no-default-features`
  configurations, plus `cargo run --locked -p neojs-lite -- test` over
  `tests/js/shared/*.test.js` and `tests/js/shared/*.test.ts`.
- **engine-isolation**: Verifies via `cargo tree --locked` that:
  1. Default workspace and `neojs` graphs exclude `rquickjs` (`rquickjs`, `rquickjs-core`, `rquickjs-sys`).
  2. `neojs-lite` excludes `deno_core`, `deno_v8`, and `v8`.
  3. `neojs-dts` has zero normal dependencies.
  4. `neojs-module-policy --no-default-features` has zero normal dependencies.
  5. `neojs-ssh-core` and `neojs-crypto-core` normal graphs exclude JS engines and `tokio`.
  6. `neojs-remote --no-default-features` and `neojs-npm` normal graphs exclude `ureq` and JS engines.
  7. `neojs-lite-core --no-default-features` normal graph excludes `deno_ast`, `deno_core`, `deno_v8`, `v8`, `neojs-builtins`, `neojs-fs-core`, `neojs-ssh-core`, `neojs-crypto-core`, `neojs-manifest`, and `neojs-dts`.
- **size-gate**: Populates Cargo metadata (`cargo fetch --locked`) and runs
  `bash eng/size/check.sh`.

### Documented failure modes (`ci.yml`)
- **Toolchain drift**: `eng/size/toolchain` was updated without updating
  `RUST_TOOLCHAIN` in `ci.yml` / `release.yml` (or vice versa). Update all three
  together in one deliberate commit.
- **Offline size-gate failure (`cargo tree failed`)**: `eng/size/measure.sh`
  invokes `cargo tree --offline`; `cargo fetch --locked` must run (or be restored
  from cache) before `bash eng/size/check.sh`.
- **Engine or feature leakage**: Adding an unguarded dependency between `neojs*`
  (V8) and `neojs-lite*` (QuickJS), or into zero-dep/offline crates, breaks
  `engine-isolation`. Inspect `Cargo.toml` feature wiring and run
  `cargo tree -e normal -p <pkg>`.
- **Rustdoc warning regression**: Broken intra-doc links or bare URLs in the
  documented crates fail `RUSTDOCFLAGS="-D warnings" cargo doc`.

## `release.yml`

Tag-only (`v*`) 6-target release build, packaging, and smoke verification workflow
with `permissions: contents: read` only (zero secrets, signing keys, or publish
tokens).

Jobs:
- **verify-tag (`ubuntu-24.04`)**:
  - Checks out full tag history (`fetch-depth: 0`) and rejects lightweight tags
    (`git cat-file -t refs/tags/<tag>` must equal `tag`).
  - Verifies `eng/size/toolchain` matches `RUST_TOOLCHAIN` (`1.100.0`).
  - Verifies via `cargo metadata --no-deps --locked --format-version 1` and `jq`
    that every workspace package has `.version == "${GITHUB_REF_NAME#v}"` and
    `.publish == []` (`publish = false`).
  - Generates canonical V8 and Lite TypeScript declaration trees once via
    `cargo run --locked -p neojs-lite -- types --profile v8 --out-dir dist-types/v8`
    and `--profile lite --out-dir dist-types/lite`, uploading `release-types`.
- **build (`needs: verify-tag`, `fail-fast: false`, 6 targets)**:
  - `linux-amd64` (`x86_64-unknown-linux-gnu` on `ubuntu-24.04`, `.tar.gz`, native)
  - `linux-arm64` (`aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm`, `.tar.gz`, native)
  - `macos-arm64` (`aarch64-apple-darwin` on `macos-15`, `.tar.gz`, native)
  - `macos-amd64` (`x86_64-apple-darwin` on `macos-15`, `.tar.gz`, cross)
  - `windows-amd64` (`x86_64-pc-windows-msvc` on `windows-2022`, `.zip`, native)
  - `windows-arm64` (`aarch64-pc-windows-msvc` on `windows-11-arm`, `.zip`, native)
  - Builds `cargo build --release --locked --target <target> -p neojs -p neojs-lite`
    and emits a `::error` check-run annotation summarizing the tail of the build
    log on failure.
  - Downloads `release-types` and stages two archives per target:
    - `neojs-<tag>-<slug>.{tar.gz,zip}`: `neojs` (or `neojs.exe`), `LICENSE.md`, and `types/` (V8 `.d.ts` tree).
    - `neojs-lite-<tag>-<slug>.{tar.gz,zip}`: `neojs-lite` (or `neojs-lite.exe`), `LICENSE.md`, and `types/` (Lite `.d.ts` tree).
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
- **package-checksums (`needs: build`, `ubuntu-24.04`)**:
  - Downloads all 12 archives into `dist/`, asserts exact count `== 12`, generates
    sorted `SHA256SUMS.txt` via `sha256sum`, verifies `sha256sum -c SHA256SUMS.txt`,
    and uploads `release-bundle-<tag>` (`retention-days: 30`).

### Documented failure modes (`release.yml`)
- **Lightweight tag pushed**: `git cat-file -t refs/tags/<tag>` returns `commit`
  instead of `tag`. Recreate as an annotated tag (`git tag -a vX.Y.Z -m "..."`).
- **Version or `publish = false` mismatch**: Any workspace crate `.version` differs
  from `${GITHUB_REF_NAME#v}` or lacks `publish = false` (`publish != []`).
  Align all workspace manifests before tagging.
- **Unpinned Action SHAs**: `uses:` references currently carry
  `# TODO(ci): pin to full 40-char commit SHA once online`; resolve and pin
  40-char commit SHAs once online before cutting public releases.
- **Cross-compiled target runtime smoke gaps**: If Rosetta 2 is absent on `macos-15`,
  `macos-amd64` falls back to binary-header/`otool` checks and still requires a
  native macOS x86_64 smoke pass before promoting the release.
- **Signing & publishing (`system-admin` boundary)**: This workflow never signs
  artifacts, opens GitHub Releases, or publishes crates. Coordinate with
  `system-admin` to pick up `release-bundle-<tag>`, sign/attest `SHA256SUMS.txt`,
  and create the GitHub Release (`publish = false` blocks crates.io publishing).
