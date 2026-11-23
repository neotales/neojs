# V8 source-build inputs

`v8` 150.4.0 is the rusty_v8 crate for V8 15.0.245.2
(`include/v8-version.h`: 15 / 0 / 245 / 2). A from-source build of that crate
needs inputs that the crates.io archive deliberately excludes. This directory
records which revision supplies each one, how it is verified, and what was
actually built.

Status: **verified** on 2026-11-23, Linux x86_64, glibc, 12 jobs, release
profile, ICU and Temporal enabled.

## The archive is source, not self-contained

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
0 differing files (the vendor commit has additional files). Neither absence is
a crate defect; both are excluded to keep the archive small.

## Pins

Every hash or commit below is recorded in a file covered by the crate archive's
crates.io checksum, so it inherits that pin.

| Input | Pin | Verified by |
|---|---|---|
| Crate archive | sha256 `42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49` | `Cargo.lock` |
| GN | commit `3357c4f51b1a9e676378c695dd9c7e9911c35ee6` (`v8/DEPS` `gn_version`) | git commit, `gn --version` reports `2407 (3357c4f51b1a)` |
| `third_party/rust` | commit `26e8ff47f18a8d28d6187a04b6a16cb7332356f8` | git commit |
| `third_party/icu` | commit `ee5f27adc28bd3f15b2c293f726d14d2e336cbd5` | git commit; 1027-file cross-check against the crate |
| ICU little-endian data | sha256 `1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612` | file hash after commit check |
| ICU big-endian data | sha256 `c58e1567b34e12a672b24ff4a5f21efa4c4481ab64a213dde059309b01aacde2` | file hash after commit check |
| Rust toolchain | sha256 `832de79f8d90940f4aaef023f83a00c1e7210c023f4d57f606b7bf9831c889aa`, 274393124 bytes (`v8/DEPS`) | hash and size |
| Chromium clang | sha256 `de584381536aa5ba2403033c4f8b70f3c39c2e5d7fa87c953b7fd8bfbba0ee2a`, 69635068 bytes (`v8/DEPS`) | hash and size |

The clang tarball is `llvmorg-23-init-10931-g20b6ec66-11`; the Rust toolchain
is `rustc 1.96.0-nightly (4c4205163 2026-04-09)` built against that clang. GN
is built from source at the exact commit rather than taken from CIPD: CIPD's
resolve API does not carry the archive hash into `DEPS`, so a CIPD download
would be authenticated by TLS alone.

## Procedure

```sh
eng/v8-source/prepare.sh          # fetch, verify, overlay, build GN
. ~/.cache/neotales/neojs-v8-build/work/build-env.sh
```

`prepare.sh` is idempotent and defaults to
`$XDG_CACHE_HOME/neotales/neojs-v8-build` (override with
`NEOTALES_V8_CACHE`). `--verify-only` checks an existing tree without
downloading or writing. It never touches the cargo registry and never reads a
credential.

Build the crate from an **isolated copy** of the extracted archive, not from
`~/.cargo/registry/src`: `build.rs` writes `third_party/rust-toolchain` and the
GN output into the manifest directory.

The profile must match the workspace: `is_debug=false`, `use_custom_libcxx`,
`simdutf`, no pointer compression, no sandbox. That is what `deno_core` 0.413.0
selects.

## What was measured

- `gn gen`: 766 targets from 215 files; `ninja -n rusty_v8`: 2,378 steps, no
  missing inputs.
- Full release build: **32.4 min** wall on 12 jobs (`ninja` timing from
  `.ninja_log`), `is_debug=false`, no debug info.
- `librusty_v8.a`: 179 MB, sha256
  `76847187cd8523bdcaf6c131ce67a37292dbba422a77bca1a7bb73827edf60b1`
  (build output; not a deliverable).
- Symbols: 456 `JSTemporal` symbols, 108 ICU entry points, `icudtl_dat.o`
  present, version string `15.0.245.2-rusty`.
- Runtime smoke (temporary binary in the cache, not committed):
  `version=15.0.245.2-rusty`,
  `2026-11-23T07:48:11.642405029Z|2020-01-01|1.234,5|1/1/1970` — Temporal and
  ICU-backed `Intl` both working, including `de-DE` number data.
- Disk: 5.1 GB total for sources, toolchains, downloads and build output;
  ~133 GB still free.

## Things that will bite

- **Do not build the crate standalone against its own `Cargo.lock`.** That lock
  pins `bindgen 0.72.0`, which generates `WriteFlags_kNullTerminate` while
  `src/string.rs` expects `v8_String_WriteFlags_kNullTerminate`. The workspace
  lock pins `bindgen 0.72.1`, which generates the expected name. If a
  standalone probe is used, force `bindgen 0.72.1` first.
- `tools/rust_toolchain.py` and `tools/clang/scripts/update.py` do not verify
  their own downloads. `prepare.sh` verifies against `v8/DEPS` before use.
- GN's `build/gen.py` runs `git describe --match initial-commit`; a shallow
  clone fails until `git fetch --unshallow`. `--no-last-commit-position` is not
  a workaround: the source still includes `last_commit_position.h`.
- Put the verified Chromium clang on `CLANG_BASE_PATH`. A system clang of the
  same major exists, but lacks Chromium's per-target runtime layout
  (`lib/clang/23/lib/x86_64-unknown-linux-gnu/libclang_rt.builtins.a`).
  `LIBCLANG_PATH` may still point at a system `libclang` for bindgen.
- `ninja` is host-provided. This run used 1.13.2; `v8/DEPS` pins the CIPD
  package `infra/3pp/tools/ninja` at `1.12.1.chromium.4`. It only schedules
  commands, so it cannot change object code, but it is the one unpinned host
  tool in this procedure.

## Network contacts

The preparation fetches only public, pinned sources: `static.crates.io` (crate),
`storage.googleapis.com` (clang and Rust toolchain), `gn.googlesource.com`, and
`chromium.googlesource.com`. The dependency build may use crates.io, but every
artifact is checked against a `Cargo.lock` checksum. No authentication, no
cookies, no credentials.

## Blockers

None for acquiring the inputs or building V8 15.0.245.2 from source with ICU
and Temporal. The two deviations are recorded above: the host ninja version,
and the standalone-crate bindgen resolution. There is no 200 MB vendored tree
to commit; all fetched inputs stay in the cache.
