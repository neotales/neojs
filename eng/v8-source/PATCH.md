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
path.

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

From the verified `prepare.sh` inputs:

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
| Patch file | sha256 `9dbf09cead9c8fe12e97dcc0c7cf98812183c408e0b105cf9ba4c1cd06407c3f`, 325 lines |

The prepared cache is verified with
`eng/v8-source/prepare.sh --verify-only`. Local stamps are drift detection, not
attacker authentication.

## Build

```sh
export NEOTALES_V8_CACHE=/path/to/verified/prepared/cache
eng/v8-source/build-bounded.sh
# then, for the Node build/tests:
export RUSTY_V8_ARCHIVE=/path/reported/by/build-bounded.sh
```

The script copies the prepared crate to an isolated build root, applies the
patch, uses the verified `GN`, Chromium clang, Rust toolchain and build-env,
sets `V8_FROM_SOURCE=1`, and builds a release `librusty_v8.a` offline. The V8
tree and archive are not committed.

Observed local build (Linux x86_64, 12 jobs):

- `librusty_v8.a`: 186,810,584 bytes
- sha256: `67552f675667459e7ce167cabe62f9da6207225b3b7a552620cf1eb65d95656e`
- An earlier build from the same source via the workspace path produced
  186,791,634 bytes / `f3a24493e280335f2d9801772e59d1a6c63a2d2d289b58ab940dbc06d0a997ad`;
  the difference is the no-bit-reproducibility caveat, not a pin.
- exported symbol: `neojs_v8_get_own_enumerable_string_keys_bounded`
- ICU and Temporal enabled in the build (`V8_INTL_SUPPORT`,
  `V8_TEMPORAL_SUPPORT`, `icudtl_dat.o` present)

**No bit-for-bit reproducibility claim.** The build is repeatable on the same
host/toolchain inputs, but V8 and the Rust toolchain do not promise identical
bytes across builds. The archive hash above is an observation, not a pin.

## License

V8 source is BSD-style; see `v8/LICENSE` in the pinned crate tree. The Rusty V8
crate is MIT. The patch retains V8's copyright/license headers and adds no
third-party code.

## Why not a stock archive

A stock `librusty_v8.a` lacks the symbol. The `node-compat` feature therefore
fails at link time rather than silently falling back to
`Object::GetOwnPropertyNames`. The Node feature is only usable with the patched
archive.
