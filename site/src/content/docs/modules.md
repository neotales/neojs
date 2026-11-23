---
title: Built-in modules
description: The thirteen modules implemented by the V8 runtime, their aliases, and the registry and namespace identity rules.
sidebar:
  order: 3
---

These modules are implemented by the **V8 runtime only**. NeoJS Lite does not provide `neo:`
modules; it runs local ECMAScript files with no built-in module surface.

## The list

| Module | Import specifiers | What it provides | Adds a capability? |
| --- | --- | --- | --- |
| [`age`](/modules/age/) | `age`, `neo:age` | X25519 identities and binary age v1 encryption | Entropy, no file or network access |
| [`assert`](/modules/assert/) | `assert`, `neo:assert` | Assertions, structural equality, Promise checks | One native-Promise observation op |
| [`chars`](/modules/chars/) | `chars`, `neo:chars` | Unicode code-point classification and simple case mapping | No |
| [`console`](/modules/console/) | `console`, `neo:console` | The realm's installed console object and five methods | Writes to process stdout/stderr |
| [`crypto`](/modules/crypto/) | `crypto`, `neo:crypto` | CSPRNG, AES-256-GCM envelope, CBC/GCM ciphers, SHA-2, HMAC, PBKDF2, bcrypt, Shamir sharing | Entropy and bounded native crypto work |
| [`os`](/modules/os/) | `os`, `neo:os` | Build-target `platform` and `arch` strings | No |
| [`path`](/modules/path/) | `path`, `neo:path` | Lexical POSIX and Windows path operations | Reads the process working directory when needed |
| [`slices`](/modules/slices/) | `slices`, `neo:slices` | Live views over Arrays, typed arrays, and code-point buffers | No |
| [`ssh`](/modules/ssh/) | `ssh`, `neo:ssh` | Synchronous SSH command execution with mandatory host verification | DNS, outbound TCP, SSH |
| [`stdio`](/modules/stdio/) | `stdio`, `neo:stdio` | Bounded in-memory byte streams and process stdout/stderr writers | Writes to process stdout/stderr |
| [`strings`](/modules/strings/) | `strings`, `neo:strings` | Case-folded search and code-point trimming | No |
| [`test`](/modules/test/) | `test`, `neo:test` | Registration-based test runner used by `neojs test` | No; runs in the realm |
| [`uuid`](/modules/uuid/) | `uuid`, `neo:uuid` | RFC 9562 version 4 and version 7 generation | Entropy |

The `uuid/v4` and `uuid/v7` subpaths are also available as `neo:uuid/v4` and `neo:uuid/v7`.

## Registry and import identity

A module can be reached in two ways:

```js
import { v4 } from "uuid";

const registry = Neo.getBuiltInModule("uuid");

console.log(v4 === registry.v4); // true
console.log(await import("neo:uuid") === await import("uuid")); // true
```

The bare name and the `neo:` name resolve to the same module instance. Every named export is the
same function or class object as the matching property of the registry object. The ES module
namespace and the registry object are different containers over those shared values, not the same
object.

`Neo.getBuiltInModule` takes **bare** names. `Neo.getBuiltInModule("neo:uuid")` returns
`undefined`; the import specifier is where the prefixed form is used. The `neo:` scheme is matched
case-insensitively, as URL schemes are, so `NEO:uuid` works. The bare name is matched exactly. An
unknown name returns `undefined`, and a non-string name throws `TypeError`.

Each module object is frozen and cached per realm, so repeated lookups return the same object. A new
`Runtime` is a new realm with new function objects. Assigning to a module namespace property throws
`TypeError`; adding a property to a shared function object is visible through every reference to that
function.

## What is not a built-in

Bare specifiers that are not in the table above, import maps, and module type attributes are not
supported. `node:`, `npm:`, `jsr:`, `http:`, `data:`, and `blob:` imports are rejected. In
particular, `node:test` is not an alias for `test`.

`Deno.core` is reachable from JavaScript, but it is not a supported API and is not covered by these
pages.

## Reading order

- Start with the module you need in the sidebar.
- Read [Limits and unsupported features](/limits/) before assuming Node or Web APIs exist.
- Read [Remote loading and security](/remote-security/) before loading code over HTTPS.
