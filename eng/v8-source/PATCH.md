# Bounded V8 own-key source patch

## What this is

`v8` 150.4.0 is the Rusty V8 crate for V8 15.0.245.2. The stock V8 public API
has no bounded own-property enumeration: `Object::GetOwnPropertyNames` builds a
complete `v8::Array`. `neojs-node` therefore requires a V8 archive built from
the pinned source with:

- `patches/0001-bounded-own-enumerable-string-keys.patch`

The patch changes only:

- `v8/src/objects/keys.h`
- `v8/src/objects/keys.cc`
- `v8/src/api/api.cc`

It adds an internal `GetOwnEnumerableStringKeysBounded` function and a narrow
`extern "C"` entry point. It does not alter any existing V8 API or key collector
path. Keys are copied to a Rust sink while the V8 handle scope is alive; no V8
handle crosses the C ABI, so there is no escaped-local lifetime. Empty keys and
keys with unpaired surrogates are rejected with `kInvalidKey` before UTF-8
conversion. Conversion uses `String::WriteUtf8` directly, without the
replacement flag, into a bounded buffer; a length/write mismatch is reported as
`kKeyConversionFailed`, and Rust throws a coded `ERR_INVALID_ARG_VALUE` rather
than returning `undefined`. Allocation failure is `kMemory`; the C ABI only
reports `kException` when V8 actually has a pending exception. Test-only
`neojs_v8_reset_element_scan_probe`/`neojs_v8_element_scan_probe` counters are
`thread_local`, so parallel isolates cannot race or observe one another. A
thread-local `neojs_v8_test_set_key_conversion_failure` hook fault-injects the
conversion-failure path for tests; it is inert unless called from native test
code and is not exposed to JavaScript.

## Supported and rejected objects

Supported: `JS_OBJECT_TYPE` ordinary objects with fast or dictionary properties,
no access check, no named/indexed interceptor, no custom-element map, and either
no elements or fast non-double elements.

Rejected before callbacks or output allocation: Proxy, functions, other
instance types, access checks, interceptors, custom-element maps, dictionary
elements, double elements, and property/dictionary capacity beyond the scan
bound. Descriptor and element capacity slots are counted before allocation;
the output `FixedArray` and index strings are created only after the key and
scan bounds pass. Index keys are emitted in ascending order followed by
descriptor/dictionary insertion order.

## Pins

Prepared and verified by procedure commit
`3f925189837b7f0d3c334e43f9fae9eb38d8683b` in the read-only v8-build
worktree. From the verified `prepare.sh` inputs:

| Input | Pin |
| --- | --- |
| v8 crate archive | `v8` 150.4.0, sha256 `42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49` |
| V8 version | 15.0.245.2 (`v8/include/v8-version.h`) |
| GN | commit `3357c4f51b1a9e676378c695dd9c7e9911c35ee6` |
| `third_party/rust` | commit `26e8ff47f18a8d28d6187a04b6a16cb7332356f8` |
| `third_party/icu` | commit `ee5f27adc28bd3f15b2c293f726d14d2e336cbd5` |
| ICU data | `icudtl.dat` sha256 `1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612`; `icudtb.dat` sha256 `c58e1567b34e12a672b24ff4a5f21efa4c4481ab64a213dde059309b01aacde2` |
| Rust toolchain | sha256 `832de79f8d90940f4aaef023f83a00c1e7210c023f4d57f606b7bf9831c889aa`, 274393124 bytes |
| Chromium clang | sha256 `de584381536aa5ba2403033c4f8b70f3c39c2e5d7fa87c953b7fd8bfbba0ee2a`, 69635068 bytes |
| Procedure helper tree | `86590659ebb17b1e40d867e7e6268d0a407d488a` (7 files at `3f925189837b7f0d3c334e43f9fae9eb38d8683b`) |
| Patch file | sha256 `addc1c8b41a614911a4f1c8905780c3dffbaa3888ecdb159d8a444bc71ae044c`, 387 lines |
| Patched `keys.h` | sha256 `5fbf2e84da60df7f0b7b8e35d8dc73d261ab44043e6fe9e3cef6835cfd0d4179` |
| Patched `keys.cc` | sha256 `262f305d6b7918a594362ea829ce074f51a862272cc1c631da102e5bf765e56a` |
| Patched `api.cc` | sha256 `a96c9c0c8b575496da039d6abc98f4537a274f33d994946a851744faed5159ce` |

The prepared cache is verified with
`eng/v8-source/prepare.sh --verify-only`. Local stamps are drift detection, not
attacker authentication. `NEOTALES_V8_CACHE` and `NEOTALES_V8_PROCEDURE` are
caller-supplied roots: the caller must trust them. This script does not claim to
authenticate them beyond the verified procedure's checks and the patch/source
hashes below.

## Build

```sh
export NEOTALES_V8_CACHE=/path/to/verified/prepared/cache
export NEOTALES_V8_PROCEDURE=/path/to/read-only/procedure
eng/v8-source/build-bounded.sh
# then, for the Node build/tests:
export RUSTY_V8_ARCHIVE=/path/reported/by/build-bounded.sh
```

The script recreates a fresh source tree, applies the exact patch, uses the
verified `GN`, Chromium clang, Rust toolchain and build-env, sets
`V8_FROM_SOURCE=1`, and builds a release `librusty_v8.a` in a unique
`mktemp -d` run root per invocation. It requires `flock`, rejects `/` and
overlapping roots, and verifies the procedure code it executes before copying
any source. It publishes the archive plus `provenance.txt` under
`<run-root>/artifact/`; the provenance records the run root, procedure commit,
`procedure_status=clean`, `procedure_helpers=verified`, the helper tree object
and file count, patch/file hashes, builder-lock hash, archive hash and size.
`--run-root-only` and `--check-procedure-only` are test-only modes used by
`eng/v8-source/test-build-bounded.sh` to prove unique run roots and helper
verification/rejection. The build saves the pre-build procedure revision,
helper-tree object and helper file count, then re-verifies the same values
after the build and fails if the procedure switched to another clean commit;
the post-check never overwrites the identity written to provenance.
`--check-procedure-only --expect-rev/--expect-helper-tree/--expect-helper-files`
lets the regression test exercise a clean commit switch without a V8 rebuild.
The V8 tree and archive are not committed.

The procedure check is deliberately stronger than `git status`:
`git status --porcelain --untracked-files=all --ignored -- eng/v8-source` must
be empty, so an ignored helper cannot hide. Every file in the recorded
`eng/v8-source` tree must be a regular non-symlink file, must have the recorded
Git executable bit, and must byte-match the recorded blob. The known executed
helpers (`prepare.sh`, `validate_tar.py`, `verify_git_tree.py`,
`materialize_git_tree.py`) must be tracked. The check runs again after the
long build and compares against the saved pre-build revision, helper-tree
object and file count; a clean checkout that switches commits mid-build fails
before provenance is written. This catches a switch that persists to the
post-check. A switch that is reverted before the post-check is not
distinguishable by this check, so the procedure worktree must not be changed
concurrently while a build runs. This is drift and identity verification, not
attacker authentication. The pre/post comparison is script-only, so the
regression test covers it without rebuilding V8.

Observed local build (Linux x86_64, 12 jobs, 31m05s):

- `librusty_v8.a`: 186,863,610 bytes
- sha256: `e9d7d15a0b8e85409fbb8d4f592bba3d86fc6ea544a550cc750e50e36f639a8d`
- The unique-run-root script build produced this artifact and wrote
  `runs/20261124T004722Z.oLmiEC/artifact/provenance.txt`; that provenance
  records `procedure_helpers=verified`, helper tree
  `86590659ebb17b1e40d867e7e6268d0a407d488a` and 7 helper files. The same
  archive hash was observed from the immediately preceding script build; an
  earlier manual build produced `6daab0e5…`, and other builds produced
  `12735288…`, `cca39e…`, `643ddb…`, `67552f…`, `f3a244…`. The differences
  are the no-bit-reproducibility caveat, not a pin. Rebuilding the archive
  requires `cargo clean -p v8` in the consumer target because Cargo does not
  track the archive file content.
- exported symbol: `neojs_v8_get_own_enumerable_string_keys_bounded`
- ICU and Temporal enabled in the build (`V8_INTL_SUPPORT`,
  `V8_TEMPORAL_SUPPORT`, `icudtl_dat.o` present)

Consumer compatibility: the archive is a release V8 build with
`use_custom_libcxx`, `simdutf`, no pointer compression and no sandbox, matching
`deno_core` 0.413.0's v8 selection. Both consumer debug and release Rust
profiles link the same release archive:

```sh
export RUSTY_V8_ARCHIVE=/path/to/librusty_v8.a
cargo +1.100.0 test -p neojs-core --features node-compat --test node_compat --locked --offline
cargo +1.100.0 test -p neojs-core --features node-compat --test node_compat --release --locked --offline
```

After a consumer build, verify the final executable (not only a library)
contains the symbol:

```sh
nm target/debug/deps/node_compat-* | grep neojs_v8_get_own_enumerable_string_keys_bounded
# The V8 archive is built with hidden visibility, so the symbol is present as
# a local hidden function in the final executable; `readelf -sW` shows it too.
```

The builder uses a committed `builder.lock` with `--locked`; it pins
`bindgen 0.72.1` and the v8 dependency resolution. The script asserts both
resolved versions.

**No bit-for-bit reproducibility claim.** The build is repeatable on the same
host/toolchain inputs, but V8 and the Rust toolchain do not promise identical
bytes across builds. The archive hash above is an observation, not a pin.

## License

V8 source is BSD-style. The crates.io `v8` 150.4.0 archive does not ship a
standalone V8 `LICENSE` file; the BSD-style notice is in every V8 source header,
for example `v8/src/api/api.cc` lines 1-3. The Rusty V8 crate is MIT according
to its Cargo metadata. The patch retains those headers and adds no third-party
code.

## Why not a stock archive

A stock `librusty_v8.a` lacks the symbol. The `node-compat` feature therefore
fails at link time rather than silently falling back to
`Object::GetOwnPropertyNames`. The Node feature is only usable with the patched
archive.
