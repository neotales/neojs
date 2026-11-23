# V8 bounded own-key enumeration: blocker handoff

Status: **blocked, not accepted**. The V8 `node:child_process` adapter is native
and all other review findings are fixed, but V8 still materializes the complete
own-property key array before the adapter can enforce its six-key option cap.
This document records the API and build findings so the work is not silently
written off as documentation.

## Requirement

For `options` and `options.env`, reject or accept a plain ordinary object after
bounded work and bounded allocation:

* count and scan budget applied before the key list is materialized;
* integer, ordinary string, symbol, deleted, and non-enumerable slots accounted
  for;
* Proxy, function, and other exotic objects fail closed before traps;
* no unbounded fast-property, dictionary, or elements allocation;
* preserve the ordinary Node option/environment object semantics. Replacing the
  env object with a custom builder or a different array-like interface is not an
  acceptable substitute.

## What the current V8 adapter does

* Rejects Proxy and function option/environment objects before enumeration.
* Calls `v8::Object::get_own_property_names` with
  `KeyCollectionMode::OwnOnly`, `ONLY_ENUMERABLE | SKIP_SYMBOLS`, and
  `ConvertToString`.
* Checks the returned array length, rejects above the cap, and copies at most
  the bounded key set.
* Propagates a pending V8 exception unchanged (including an allocation failure).

The array is already fully materialized by V8 at the `get_own_property_names`
call. The adapter therefore does **not** claim a pre-materialization bound.

## API findings

* The only exposed own-name call is
  `v8::Object::GetOwnPropertyNames` in `v8/include/v8-object.h` (rusty_v8
  `v8__Object__GetOwnPropertyNames` in `src/binding.cc`). It returns a
  fully-built `v8::Array`; the overloads take only context, property filter, and
  key conversion mode. There is no max-key or max-scan parameter.
* `GetPropertyNamesArgs` (mode, property filter, index filter, key conversion)
  has no work or size budget.
* V8's implementation is `FastKeyAccumulator` / `KeyAccumulator` in
  `v8/src/objects/keys.{h,cc}`. The fast path uses the map's enum cache and the
  slow path materializes a `FixedArray`; neither has a public cap.
* The Rust `v8` 150.4.0 crate does not expose `KeyAccumulator` or any equivalent
  bounded binding. `deno_core` 0.413.0 exposes no bounded helper either.
* V8 has no public post-creation heap-limit setter. `CreateParams::heap_limits`
  is an isolate-creation setting. A heap budget would bound the entire runtime,
  not one enumeration, and choosing an arbitrary budget is a product decision.

## Build/toolchain feasibility

A targeted engine or binding patch is **not feasible in this environment
without network or a preinstalled V8 build toolchain**:

* The workspace links a prebuilt `librusty_v8.a` from the v8 crate cache/build
  output; there is no V8 source build tree (`args.gn`, `build.ninja`) here.
* `rusty_v8`'s `V8_FROM_SOURCE` path requires `gn` and its downloaded toolchain.
  `gn` is not installed and no cached `gn` binary exists. `ninja`, clang, and
  Python are present, but that is not sufficient.
* The network is unavailable by policy. `V8_FROM_SOURCE=1` would invoke the
  crate's toolchain/gn download scripts.
* Even with `gn`, a full V8 source build is a large, lengthy build and would need
  vendored rusty_v8 bindings plus a vendored V8 source revision. The existing
  two vendored patches (`vendor/deno_core`, `vendor/rquickjs-sys`) do not include
  V8 or rusty_v8.

## Patch plan if the engine build is approved

1. Pin the exact V8/rusty_v8 source revision used by v8 150.4.0.
2. Add a public bounded own-key API, for example
   `Object::GetOwnPropertyNamesBounded(context, filter, key_conversion,
   max_keys, max_scan, &scanned)`.
3. Implement it in `src/objects/keys.cc` by bounding both the scan over shape
   slots/elements and the output allocation. Reject or fail closed on exotic
   objects before traps. Preserve JS key order and the existing semantics for
   ordinary objects.
4. Add `v8__Object__GetOwnPropertyNamesBounded` to rusty_v8 `src/binding.cc`
   and a safe Rust wrapper in `src/object.rs`.
5. Vendor the V8/rusty_v8 patch with provenance and checksums, separate from the
   existing deno_core and rquickjs-sys patches.
6. Switch the option/environment path to the bounded call, keeping the current
   six-key option cap and 128-entry environment cap, and add an ordinary-object
   huge-key test plus exotics/proxy tests.

## Alternatives evaluated

| Alternative | Result |
| --- | --- |
| Read only the six known option names | Cannot reject unknown fields; weakens the `shell`/`stdio` rejection contract. |
| Use a custom env builder / array interface | Changes the Node API; rejected by the review. |
| Lower the whole-isolate heap budget | Coarse, isolate-wide, not a per-enumeration bound; arbitrary product limit. Needs a runtime budget API and product approval. |
| Interrupt/watchdog terminate | Non-deterministic; key accumulation does not guarantee interrupt checks; unsafe as a work bound. |
| Patch only `deno_core` or Rust callers | They call the same V8 public API; cannot bound V8's internal allocation. |
| Patch only rusty_v8's C++ shim against the prebuilt archive | No bounded V8 public API to call; internal `KeyAccumulator` symbols are not a supported ABI. |

## Recommendation

Approve a vendored V8/rusty_v8 bounded-binding build with its toolchain, or
keep the V8 `node:child_process` compatibility claim explicitly incomplete for
large ordinary option/environment objects. Do not replace the object contract
with a builder, and do not claim a bound the current API cannot enforce.
