---
title: assert
description: Callable assertions with structural equality, identity equality, synchronous throws, and Promise rejection checks.
---

`assert` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `assert`
or `neo:assert`; the registry form is `Neo.getBuiltInModule("assert")`.

The default export is the callable `assert` function. The named exports are its own properties.

## Exports

| Export | Purpose |
| --- | --- |
| `assert` (default and named) | The callable assertion; `assert.ok` is the same function |
| `AssertionError` | The `Error` subclass thrown by failed assertions |
| `ok` | Alias of the callable assertion |
| `equals`, `notEquals` | Strict structural equality and its negation |
| `strictEquals`, `notStrictEquals` | `Object.is` identity and its negation |
| `fail` | Throw an `AssertionError` |
| `throws`, `doesNotThrow` | Synchronous callback checks |
| `rejects`, `doesNotReject` | Promise-returning async checks |

```js
import assert, { equals, strictEquals } from "assert";

const value = 1;
assert(value, "optional message");
equals({ a: [1, 2] }, { a: [1, 2] }); // structural equality
const same = { a: 1 };
strictEquals(same, same); // Object.is identity
assert.throws(() => JSON.parse("{"), SyntaxError);
await assert.rejects(async () => {
    throw new Error("no");
});
```

## Equality

`equals` is strict structural equality, not `==` and not identity. Prototypes must match. Own
enumerable string and symbol properties are compared, and getters are invoked. Arrays compare length
and distinguish a hole from an own `undefined`. Built-in state is compared in addition to own
properties:

- `Date` time values.
- `RegExp` source, flags, and `lastIndex`.
- `ArrayBuffer` and `SharedArrayBuffer` bytes, `DataView` viewed bytes, and typed-array element type,
  length, and elements.
- Boxed primitive values.
- `Error` `name` and `message`.
- Map and Set contents without regard to insertion order.

`NaN` equals `NaN`, `-0` differs from `0`, and cycles are handled. Functions, genuine native
Promises, WeakMaps, WeakSets, and WeakRefs compare by identity.

`strictEquals` is exactly `Object.is`: `strictEquals(NaN, NaN)` passes, `strictEquals(0, -0)`
fails, and two distinct objects are never identical.

## Failures

Every failed assertion throws `AssertionError`, an `Error` subclass with:

- `name` set to `"AssertionError"`.
- A stable own `code` of `"NEO_ASSERT"`.
- An `operator` naming the failed assertion.
- `actual` and `expected` when the assertion had them.

The code is NeoJS-specific; it is not Node's `ERR_ASSERTION`.

Diagnostics are bounded: a formatted value is truncated at 400 characters, 50 items, and eight
levels, and cycles print as `[Circular]`. Formatting may invoke getters. This is a test helper in
the caller's realm, not an isolation boundary.

## Callback checks

`throws(callback[, expected[, message]])` returns the thrown value and throws unless the callback
throws. `doesNotThrow(callback[, message])` returns the callback's result and throws if it throws.

`expected` is an error constructor, checked with the intrinsic `instanceof` rather than a custom
`Symbol.hasInstance`, or a `RegExp` tested against the thrown error's message (or the string form of
a non-Error value), or omitted to accept any throw. A string second argument is a `TypeError`, not a
message; pass the message third.

A callback that returns a genuine native Promise is a `TypeError` that points at the Promise-aware
form, so work that has not run cannot be reported as a pass.

`rejects(callback[, expected[, message]])` returns a Promise that rejects with `AssertionError` when
the callback fulfils, and otherwise records a rejection. A synchronous throw is treated as a
rejection. The callback's return value must be a genuine native Promise or a thenable. Native
Promises are observed through their real internal settlement state, not an overridden `then` or
`Symbol.species`.

`doesNotReject(callback[, message])` resolves with the fulfilment value and rejects with
`AssertionError` if the callback throws or rejects.

## Deliberate omissions

There is no `assertEquals`, `deepEqual`, `deepStrictEqual`, `strictEqual`, `equal`, `notDeepEqual`,
`match`, `ifError`, snapshot support, or `node:assert` specifier. The surface is inspired by Node's
`assert/strict` and Deno's `@std/assert`, but it is not a compatibility layer for either.
