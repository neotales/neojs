---
title: uuid
description: RFC 9562 version 4 and version 7 UUID generation from the operating system CSPRNG.
---

`uuid` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `neo:uuid`; the registry form is `Neo.getBuiltInModule("uuid")`.

The aggregate module exports `v4` and `v7`. The subpath modules `neo:uuid/v4` and `neo:uuid/v7`
each expose one generator as both the named and default export. The function objects are the same,
not wrappers.

```js
import { v4 as aggregateV4, v7 as aggregateV7 } from "neo:uuid";
import v4 from "neo:uuid/v4";
import { v7 } from "neo:uuid/v7";

if (v4 !== aggregateV4) throw new Error("v4 identity changed");
if (v7 !== aggregateV7) throw new Error("v7 identity changed");

console.log(v4()); // 36-character RFC 9562 version 4 text
console.log(v7()); // 36-character RFC 9562 version 7 text
```

## Output

Both functions take no arguments, are synchronous, and return canonical lowercase text: 32
hexadecimal digits in groups of 8, 4, 4, 4, and 12, separated by hyphens. There is no braces,
`urn:uuid:` prefix, uppercase, binary, or alternate output form.

A non-zero argument count, including a trailing `undefined` or a callback, throws `TypeError` before
the host op runs. No Promise is returned and no callback form is reserved.

## `v4()`

Fills 128 bits from the operating system CSPRNG, then sets the four version bits to `0100` and the
two variant bits to `10`. The remaining 122 bits are random. A version 4 UUID carries no time, node,
or sequence information, so its values have no order and must not be used as a sort key.

## `v7()`

Uses the RFC 9562 layout:

- 48 big-endian bits of Unix time in milliseconds.
- The version `0111`.
- 12 random bits (`rand_a`).
- The variant `10`.
- 62 random bits (`rand_b`).

The module uses the random-payload method and keeps no per-process counter. Ordering is therefore
only what the timestamp provides:

- Values generated in different milliseconds sort by the embedded timestamp when the same system
  clock moves forward.
- Values generated within one millisecond carry random low bits and have no defined order relative
  to one another.
- A backward system clock step produces a value that sorts before earlier ones. Values from
  different processes or machines are not ordered.

This module does **not** promise monotonicity, uniqueness, or a total order. Use a database
sequence, a Lamport clock, or an explicit ordering column when the application needs one. The 74
random payload bits make a same-millisecond collision unlikely, not impossible.

## Errors

The 48-bit timestamp is a format limit. A current time before the Unix epoch, or at or beyond 2^48
milliseconds (roughly the year 10889), fails with `NEO_UUID_CLOCK` instead of wrapping, truncating,
or clamping. The clock is read before the CSPRNG, so an unrepresentable time consumes no entropy.

An operating-system CSPRNG failure is `NEO_UUID_RANDOM`; there is no `Math.random`, time-derived, or
other insecure fallback. Messages are fixed and never include entropy or a clock value. A random
failure also has a non-enumerable `cause` string built only from a numeric OS error code or one of
`getrandom`'s fixed categories (for example `getrandom: OS error 5`); entropy bytes, UUID bytes, and
clock values are not exposed.

## Scope

This slice implements generation only. It does not parse or validate UUID text, extract a timestamp,
generate nil or max UUIDs, or implement versions 1, 3, 5, 6, or 8. It reads no file, environment
variable, network, or process state beyond the system clock, installs no global, and is not a
sandbox.

It is separate from Node's `crypto.randomUUID()`, which has its own API and produces only version 4
values.
