# V8 source-build inputs

`v8` 150.4.0 is the rusty_v8 crate for V8 15.0.245.2
(`v8/include/v8-version.h`: 15 / 0 / 245 / 2). A from-source build needs four
inputs that the crates.io archive deliberately excludes. This directory records
the exact revision of each, how it is verified, and how to assemble them into an
isolated tree.

Status: **procedure verified** on 2026-11-23, Linux x86_64, glibc 2.44, release
profile, ICU and Temporal enabled. Repeatable on the same host and toolchain;
**not bit-for-bit reproducible** (see below). The native V8 build itself was not
re-run in the review cache; it was previously built from the same pinned inputs
(32.4 min, `librusty_v8.a` 179 MB, sha256
`76847187cd8523bdcaf6c131ce67a37292dbba422a77bca1a7bb73827edf60b1`).

## What the crate archive is missing

The checksummed `v8-150.4.0.crate` contains the V8 tree, but these are absent:

| Missing input | Why GN needs it |
|---|---|
| `buildtools/linux64/gn` | GN is the build-file generator; it is not in the crate. |
| `third_party/icu/common/icudtl.dat` (and `icudtb.dat`) | `icu_use_data_file=true` embeds the ICU 77 data bundle. |
| `third_party/rust/` sources | The crate ships only per-crate `BUILD.gn` stubs, not the vendor tree. |
| `third_party/rust-toolchain/` | Temporal support builds `//third_party/rust/temporal_capi` with the Chromium Rust toolchain. |

The crate's ICU tree was compared byte-for-byte with the pinned ICU commit:
1027 files compared, 0 mismatches, and only the two `.dat` blobs missing. The
crate's `third_party/rust` stubs were compared with the pinned vendor commit:
0 differing files (the vendor commit has additional files).

## Pins

Every hash or commit below is recorded in a file covered by the crate archive's
crates.io checksum, so it inherits that pin.

| Input | Pin | Kind |
|---|---|---|
| Crate archive | sha256 `42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49` | SHA-256 |
| GN | commit `3357c4f51b1a9e676378c695dd9c7e9911c35ee6` (`v8/DEPS` `gn_version`) | Git SHA-1 |
| `third_party/rust` | commit `26e8ff47f18a8d28d6187a04b6a16cb7332356f8` | Git SHA-1 |
| `third_party/icu` | commit `ee5f27adc28bd3f15b2c293f726d14d2e336cbd5` | Git SHA-1 |
| ICU little-endian data | sha256 `1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612` | SHA-256 |
| ICU big-endian data | sha256 `c58e1567b34e12a672b24ff4a5f21efa4c4481ab64a213dde059309b01aacde2` | SHA-256 |
| Rust toolchain | sha256 `832de79f8d90940f4aaef023f83a00c1e7210c023f4d57f606b7bf9831c889aa`, 274393124 bytes | SHA-256 |
| Chromium clang | sha256 `de584381536aa5ba2403033c4f8b70f3c39c2e5d7fa87c953b7fd8bfbba0ee2a`, 69635068 bytes | SHA-256 |

GN is built from source at the exact commit instead of being taken from CIPD:
CIPD's resolve API does not carry the archive hash into `DEPS`, so a CIPD
download would be authenticated by TLS alone. Git pins are SHA-1 object IDs,
which is Git's integrity model, not a SHA-256 pin.

## Procedure

Use an exclusive cache. The cache is a build input and is not safe to share
between concurrent builds.

```sh
export NEOTALES_V8_CACHE=$HOME/.cache/neotales/neojs-v8-source
eng/v8-source/prepare.sh        # acquire, verify, assemble; builds GN
eng/v8-source/prepare.sh --verify-only   # strict read-only re-check
. "$NEOTALES_V8_CACHE/work/build-env.sh"
```

`prepare.sh` is idempotent. `--verify-only` performs no writes, opens no
network, and executes none of the downloaded toolchains or GN. It checks
archive hashes, git commit content, extracted-tree digests and the environment
file, then exits.

`prepare.sh` reads the cargo registry cache (`$HOME/.cargo/registry/cache/...`)
read-only to reuse the crate archive cargo already verified. It never writes
there. The later `cargo build` reads the registry/index and writes
`$CARGO_TARGET_DIR`; with `--offline` it makes no network requests, and every
crate is checked against its `Cargo.lock` checksum.

The build must run from an **isolated copy** of the extracted archive, not from
`~/.cargo/registry/src`: `build.rs` writes `third_party/rust-toolchain` and the
GN output into the manifest directory.

### Explicit workspace override

The workspace `Cargo.lock` pins the registry `v8`. To build against the verified
tree, use a scratch copy of the workspace; never edit the canonical checkout.

```sh
cache=${NEOTALES_V8_CACHE:?}
scratch=$(mktemp -d)
tar -C . --exclude=.git --exclude=target -cf - . | tar -C "$scratch" -xf -
python3 eng/v8-source/patch_workspace.py "$scratch/Cargo.toml" "$cache/src/v8/v8-150.4.0"
cd "$scratch"
cargo build --release --offline
```

`patch_workspace.py` requires `[patch.crates-io]` to exist, refuses an existing
`v8` entry, serialises the path with `json.dumps` (JSON escapes are valid TOML
basic-string escapes for quotes, backslashes and newlines), and re-parses the
result with `tomllib` before writing. This changes only the scratch copy's lock;
Cargo reports that it is adding `v8` as a path package. `cargo metadata` on the
scratch copy resolves `v8 v150.4.0` to `<cache>/src/v8/v8-150.4.0` with
`source: null` (tested).

Two alternatives do **not** work and should not be used:

- `--config 'patch.crates-io.v8.path=...'` on the real workspace fails
  `--locked` because Cargo must rewrite the real `Cargo.lock`.
- `--config 'paths=["<prepared>"]'` recursively walks the V8 tree and fails on
  vendored Rust sources with no `Cargo.toml`.

## Verification model

- **Downloads**: SHA-256 and, where `DEPS` gives one, size.
- **Git checkouts**: the repository config is a controlled minimal file (no
  includes, remotes, URL rewrites, hooks or fsmonitor); every fetch names the
  pinned HTTPS URL explicitly, including `unshallow`. `HEAD` equals the pin,
  `git fsck` passes, and every tracked file is compared byte-for-byte and
  mode-for-mode against the commit's tree by `verify_git_tree.py`. The
  comparison uses `git ls-tree` and `git cat-file`, not the index. Working trees
  are materialised by `materialize_git_tree.py` from the commit tree, not
  checked out: no index, filters or ignored content participate. Every extra
  path is rejected, including ignored files; only `gn/out/` is allowed. Only the
  root `.git` metadata is skipped: a nested `.git` directory or gitdir file is
  extra content and fails verification.
- **Archives**: `validate_tar.py` rejects absolute paths, `..`, escaping
  symlinks and hardlinks, symlink/file ancestors, devices/fifos, duplicate
  members and setuid/setgid bits. Extraction is staged under the cache and
  swapped in atomically; modes and ownership are normalised.
- **Extracted trees**: a digest over file bytes, symlink targets, directory
  modes (including `.`) and each file's normalised executable bit is recorded
  in `$CACHE/.stamps/`; a mismatch fails `--verify-only` and causes
  `prepare.sh` to re-extract from the verified archive. The old tree is moved
  to `.quarantine/`, not deleted.
- **Prepared tree**: a digest covering the crate, Rust vendor tree, toolchain
  and ICU data. A mismatch rebuilds it from the verified components.
- **GN**: source commit and content are verified; the binary's SHA-256 is
  recorded at build time. `--verify-only` reads the stamp and does not execute
  GN.
- **Local stamps are integrity records, not signatures.** They detect drift and
  accidental corruption. An attacker who can write the cache can forge them.
  The cryptographic roots are the crate checksum and the git commit hashes;
  `--verify-only` re-hashes the archives and re-reads git content. To
  re-establish trust after suspected tampering, remove the stamp and re-run
  `prepare.sh`, which re-extracts from the verified archive or git commit.

### Read-only and no-exec proof

`--verify-only` was run under bubblewrap with the whole filesystem read-only,
the network unshared, and an `LD_PRELOAD` guard that blocks `execve` of any path
under the cache. Modes are not changed, so mode-aware digests remain valid.

```sh
cc -shared -fPIC -o exec_guard.so exec_guard.c -ldl
bwrap --ro-bind / / --dev /dev --proc /proc --unshare-net \
  --chdir <repo> \
  --setenv NEOTALES_V8_CACHE "$CACHE" \
  --setenv LD_PRELOAD <abs>/exec_guard.so \
  --setenv V8_EXEC_GUARD_ROOT "$CACHE" \
  eng/v8-source/prepare.sh --verify-only
```

The guard was checked to block a deliberate exec of `$CACHE/src/gn/out/gn`
(exit 126). Result of the real run: exit 0, cache snapshot before/after
identical, no network.

## Tests

`eng/v8-source/test.sh` runs 48 deterministic tests with no network and no real
V8 inputs: hostile archives (traversal, absolute paths, escaping symlinks and
hardlinks, symlink ancestors, fifo, setuid, duplicates), safe extraction,
tamper detection, partial-extraction replacement, quarantine, content and
mode-aware tree digests, git content verification against a commit, ignored
file and `include` config rejection, executable-hook rejection, nested `.git`
rejection, commit-tree materialisation including exec bits and escaping
symlinks, a stream-mismatch deadlock test (`timeout 10`, exits 2), TOML path
serialisation with quotes, backslash and newline, environment quoting with
spaces/dollars/quotes, isolated git/curl argument and environment checks, and a
missing-cache `--verify-only` check that proves no directory is created. Python
helpers are run with `python3`; only the shell entry points are executable.

Tamper tests against the real review cache:

| Tampered object | `--verify-only` | `prepare.sh` repair |
|---|---|---|
| prepared-tree ICU data | fails (`tree digest mismatch`) | rebuilds prepared tree, quarantines old |
| pristine crate file | fails (`tree digest mismatch`) | re-extracts crate, rebuilds prepared tree |
| toolchain file | fails (`tree digest mismatch`) | re-extracts toolchain, rebuilds prepared tree |
| prepared-tree executable bit | fails (`tree digest mismatch`) | rebuilds prepared tree |
| ignored file in Rust checkout | fails (`unverified git checkout`) | re-fetches and re-materialises |
| `.git/config` `include` | fails (`unverified git checkout`) | re-fetches with controlled config |
| environment file | fails (`stale or unquoted`) | rewrites it |

## Reproducibility

Repeatable given the same host and toolchain; **not bit-for-bit reproducible**.
Reasons include:

- V8 embeds absolute build paths and build metadata in its objects;
- the host C++ compiler and linker build GN and are not otherwise pinned;
- `mksnapshot`/Torque output and archive member ordering are not guaranteed to
  be byte-stable across hosts;
- the Rust dependency graph is pinned by `Cargo.lock`, but codegen depends on
  the exact rustc and LLVM build.

The `librusty_v8.a` SHA-256 above is therefore a record of one host's output,
not a reproducible target. The pinned, reproducible part is the input set: the
crate checksum, four git commits and two archive hashes.

## Licensing

The cache is a build input, not a distributable artifact. Redistribution
requires the upstream notices; several are **not** present in the downloaded
archives:

| Component | License | Notice in cache |
|---|---|---|
| V8 15.0.245.2 | BSD-3-Clause | **No.** The crate excludes `LICENSE*`; fetch V8's `LICENSE` at the matching revision. |
| ICU 77 | Unicode License V3 | Yes: `src/icu/LICENSE`, `license.html` |
| GN | BSD-3-Clause | Yes: `src/gn/LICENSE` |
| Chromium Rust vendor | per-crate (mostly MIT/Apache-2.0) | Yes: `chromium_crates_io/vendor/*/LICENSE*`, `README.chromium` |
| Chromium clang 23 | Apache-2.0 WITH LLVM-exception | **No.** The archive carries no license file; obtain LLVM's notice. |
| Rust toolchain | Apache-2.0/MIT and third-party | Yes: `src/rust-toolchain/share/doc/rustc/licenses/` |

No redistribution or warranty is claimed.

## Host dependencies

The procedure was run with these host tools; they are trusted as installed and
are not signature-verified by this script.

| Tool | Version used | Notes |
|---|---|---|
| Linux / glibc | x86_64 / 2.44 | Only Linux is tested. |
| bash | 5.3.20 | `%q` escaping and process substitution are required. |
| coreutils / findutils | 9.12 / 4.11 | `sha256sum`, `stat`, `realpath`, `readlink`, `cmp`, `find`, `xargs`, `sort`. |
| tar / xz | 1.35 / 5.8.4 | Archives are opened by Python's `tarfile`, not by `tar`. |
| python3 | 3.14.8 | Archive validation and git content verification. `patch_workspace.py` also needs `tomllib` (3.11+). |
| git | 2.56.0 | Controlled local config; explicit HTTPS URL per fetch; no hooks. |
| curl | 8.23.0 (OpenSSL 3.6.5) | Isolated config; no `.curlrc`, no netrc, no proxy, no redirects. |
| ninja | 1.13.2 | Host tool. `v8/DEPS` pins CIPD `1.12.1.chromium.4`; this is the one unpinned host tool. |
| C++ compiler | system clang++ 23.1.3 | Used only to build GN from source; also compiles the exec guard for the proof. |
| timeout | GNU coreutils 9.12 | Deadlock test only. |
| bubblewrap | 0.13.1 | Tests only. |

The V8 build uses the cached Chromium clang and Chromium Rust toolchain, not the
host `clang`/`rustc`. Host `libclang.so` 23.1 is used by bindgen. There is no
claim of a dependency-free build.

## Network contacts

Only public, pinned sources: `static.crates.io` (crate), `storage.googleapis.com`
(clang and Rust toolchain), `gn.googlesource.com`, and
`chromium.googlesource.com`. HTTPS only, no redirects, no proxy variables, no
netrc, no credential helper, no cookies, no credentials. Git and curl run with
isolated configuration and an empty home.

## Gaps and deviations

- The crate's own `Cargo.lock` pins `bindgen 0.72.0`, which generates
  `WriteFlags_kNullTerminate` while `src/string.rs` expects
  `v8_String_WriteFlags_kNullTerminate`. The workspace lock pins `0.72.1`,
  which generates the expected name. Build through the workspace lock (or force
  `bindgen 0.72.1`).
- Ninja is host-provided 1.13.2; `v8/DEPS` pins `1.12.1.chromium.4`.
- Git pins are SHA-1, not SHA-256.
- Local tree stamps can be forged by anyone who can write the cache; they are
  drift detection, not signatures.
- Cached working trees are materialised from the commit and must have exactly
  the controlled `.git/config`; a cache written by an older version is
  re-fetched. Escaping symlinks and gitlinks are rejected; the three pins
  contain neither.
- `patch_workspace.py` requires an existing `[patch.crates-io]` section and
  refuses to add a second `v8` entry.
- V8's and LLVM's license texts are absent from the archives.
- Not verified in this review: Windows/macOS, musl, cross-compilation,
  pointer-compression, sandbox, or `v8_enable_v8_checks` variants. The native
  V8 build was not repeated in the review cache; only the input procedure was.
