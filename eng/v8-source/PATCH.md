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
conversion; a null `String::Utf8Value` is reported as `kException`. Test-only
`neojs_v8_reset_element_scan_probe`/`neojs_v8_element_scan_probe` counters
prove the element-slot loop is not entered before the scan-budget check.

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
| Patch file | sha256 `f5933d7169a092eb03f909138a76bad492e65c46da90b70ea50621622c1646ca`, 325 lines |
| Patched `keys.h` | sha256 `29e3de9dcb8e3c4424a3bc6b3520573cae6c1bdff29104bbbdb20e6a298e1890` |
| Patched `keys.cc` | sha256 `ca0260e09da2e91cbf979bee393da555fb0d26056b6075d22f4d11a750873561` |
| Patched `api.cc` | sha256 `be4d4ac17b905bc02cb6a0f9464b24a43df03649ff96c6356c5b9e26bcb344f4` |

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
`V8_FROM_SOURCE=1`, and builds a release `librusty_v8.a` in a unique target
directory per run. It requires `flock`, rejects `/` and overlapping roots, and
publishes the archive plus `provenance.txt` under `artifacts/<run-id>/`. The V8
tree and archive are not committed.

Observed local build (Linux x86_64, 12 jobs):

- `librusty_v8.a`: 186,838,134 bytes
- sha256: `cca39ea314e03e1378b59c3a2118e2ffb3e64f40e97e4eb2c8a66fba8807fbbe`
- The unique-target script build produced this artifact and wrote
  `artifacts/20261123T152657Z-445194/provenance.txt`; an earlier manual build
  from the same source produced `12735288…`, and other builds produced
  `643ddb…`, `67552f…`, `f3a244…`. The differences are the no-bit-
  reproducibility caveat, not a pin. Rebuilding the archive requires
  `cargo clean -p v8` in the consumer target because Cargo does not track the
  archive file content.
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
