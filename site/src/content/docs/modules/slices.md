---
title: slices
description: Live mutable and read-only views over Arrays, typed arrays, and Unicode code-point buffers.
---

`slices` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `slices`
or `neo:slices`; the registry form is `Neo.getBuiltInModule("slices")`.

It is plain ECMAScript: it calls no host op, adds no capability, and is not a sandbox or a
cross-realm or cross-thread zero-copy bridge.

## Exports

| Export | Kind |
| --- | --- |
| `Slice` | Mutable array or typed-array view class |
| `ReadOnlySlice` | Read-only view class |
| `CharSlice` | Mutable Unicode code-point view over a `Uint32Array` |
| `ReadOnlyCharSlice` | Read-only code-point view |
| `slice`, `readonlySlice` | Constructors for array/typed-array views |
| `charSlice`, `readonlyCharSlice` | Constructors for code-point views |

There is no default export.

```js
import { slice, readonlySlice } from "slices";

const base = [0, 1, 2, 3, 4];
const view = slice(base, 1, 5); // [1, 2, 3, 4] as a live view
const inner = view.slice(1, 3); // [2, 3], relative to `view`
inner.set(0, 99);
console.log(base[2]); // 99
console.log(readonlySlice(base, 1, 3).at(1)); // 99
```

## Views are live aliases

Construction, `slice`, `at`, `set`, `indexOf`, `includes`, and iteration do not copy. A view aliases
the target, so a Proxy's traps and accessor elements run.

`copy` is the snapshot operation. It returns a new plain Array for an Array target, or a new
intrinsic typed array of the same element type for a typed-array target. The copy never aliases.
Views do not subclass Array or typed-array classes, and `copy` builds its result directly, so
`Symbol.species` is never consulted.

`indexOf` uses strict equality, so `NaN` is never found; `includes` uses SameValueZero, so `NaN` is
found.

A view's `length` is fixed at construction. Splicing, truncating, or extending the target afterwards
does not move or resize the view: it keeps addressing the same absolute indices. Reading an index
the target no longer has returns `undefined`; writing it follows ordinary Array semantics and can
extend the Array. A hole reads as `undefined` and becomes an own `undefined` element in `copy`.

## Bounds

Ranges are explicit half-open `[start, end)` bounds, defaulting to `0` and the target length. A
bound must be a number and an integer. Negative, reversed, and out-of-range bounds throw
`RangeError`; a non-number throws `TypeError`. Nothing is clamped, and negative indices are not
supported. An empty view at the end of the target is valid.

Nested `slice` bounds are relative to the parent and stay aliases of the same target; they use the
parent's fixed range rather than re-reading a target that may have shrunk.

## Typed arrays

The element type and the target's own offset and length are preserved, and every view shares the
target's buffer, so overlapping views alias each other. These are supported:

`Int8Array`, `Uint8Array`, `Uint8ClampedArray`, `Int16Array`, `Uint16Array`, `Int32Array`,
`Uint32Array`, `Float32Array`, `Float64Array`, `BigInt64Array`, `BigUint64Array`, and
`Float16Array` when the V8 build provides it.

BigInt element types require BigInt values; assigning a number throws the engine's `TypeError`. A
detached or shrunk typed array leaves reads `undefined` and writes ignored; `copy` validates the
fixed range and throws `RangeError` instead of inventing zeros. A resizable `ArrayBuffer` that is
grown back makes the fixed range readable again.

A `DataView`, a `Symbol.toStringTag` lookalike, and a Proxy of a typed array are rejected with
`TypeError`; the brand comes from the engine's internal slot, not a replaceable property or
`Symbol.hasInstance`. A Proxy of an Array is an Array and is accepted, and its traps run.

## Character buffers

`CharSlice` is a live view over a caller-owned `Uint32Array` of Unicode code points;
`ReadOnlyCharSlice` is its read-only form.

- `CharSlice.fromString(text)` allocates one buffer, combining surrogate pairs into one code point
  and preserving a lone surrogate as one code unit.
- `toString()` builds the result in bounded chunks.
- `slice` allocates only a view/state object and no character buffer.
- `toUpper`/`toLower` mutate in place and allocate no new character buffer, though simple case
  mapping may allocate a temporary string per mapped code point.
- `set` writes one code point through to the buffer.
- `toArray()` returns an independent `Uint32Array` snapshot.

The transforms use the same simple case mapping as [`chars`](/modules/chars/), so `ß` stays `ß`
rather than expanding to `SS`; lone surrogates and values outside the Unicode range are left
unchanged. `ReadOnlyCharSlice` has no `set` or in-place transform: make a `CharSlice` over the same
buffer to mutate it.

A `CharSlice` constructor does not scan its input buffer. Writing an invalid value directly into the
`Uint32Array` makes `toString` throw `RangeError`. A view whose buffer was detached or shrunk makes
`toArray` and `toString` (and, for the mutable `CharSlice`, `toUpper` and `toLower`) throw
`RangeError` when the fixed range no longer fits, including an empty range at a nonzero offset.

## Scope

This module is informed by `@neotales/slices` but is not a port and does not claim full or
byte-for-byte parity with it. It does not copy that package's `Uint8Array`/`Uint16Array` character
conversions, which reinterpret bytes without respecting a view's offset and length, and it uses
explicit `[start, end)` bounds rather than the reference's offset/length confusion. There is no
`slices.Slice`-compatible `map`/`update`/`replace` surface in this slice.

Views are aliases within one realm, not a Rust zero-copy bridge. A view is not a snapshot, a
sandbox, or an isolation boundary.
