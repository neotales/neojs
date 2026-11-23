---
title: Built-in modules
description: "The fourteen modules implemented by the V8 runtime, their canonical neo: imports, and the registry and namespace identity rules."
sidebar:
  order: 3
---

These modules are implemented by the **V8 runtime only**. NeoJS Lite does not provide `neo:`
modules; it runs local ECMAScript files with no built-in module surface.

:::caution[Canonical import policy]
Import Neo built-ins with the `neo:` prefix: `neo:uuid`, `neo:path`, and so on. Bare names such as
`uuid` and `path` are reserved for future Node/package compatibility and must not be used for Neo
modules in new code.

The current `dev` tree still accepts bare names as compatibility behavior and resolves them to the
same module instance. That is not the target contract; the canonical-only policy is planned and
will apply when it is integrated. `Neo.getBuiltInModule("uuid")` keeps taking the bare registry
name.
:::

## The list

| Module | Canonical import | What it provides | Adds a capability? |
| --- | --- | --- | --- |
| [`age`](/modules/age/) | `neo:age` | X25519 identities and binary age v1 encryption | Entropy, no file or network access |
| [`assert`](/modules/assert/) | `neo:assert` | Assertions, structural equality, Promise checks | One native-Promise observation op |
| [`chars`](/modules/chars/) | `neo:chars` | Unicode code-point classification and simple case mapping | No |
| [`console`](/modules/console/) | `neo:console` | The realm's installed console object and five methods | Writes to process stdout/stderr |
| [`cron`](/modules/cron/) | `neo:cron` | UTC five-field parsing and in-process scheduling | Host-clock scheduling; keeps the event loop alive until disposed |
| [`crypto`](/modules/crypto/) | `neo:crypto` | CSPRNG, AES-256-GCM envelope, CBC/GCM ciphers, SHA-2, HMAC, PBKDF2, bcrypt, Shamir sharing | Entropy and bounded native crypto work |
| [`os`](/modules/os/) | `neo:os` | Build-target `platform` and `arch` strings | No |
| [`path`](/modules/path/) | `neo:path` | Lexical POSIX and Windows path operations | Reads the process working directory when needed |
| [`slices`](/modules/slices/) | `neo:slices` | Live views over Arrays, typed arrays, and code-point buffers | No |
| [`ssh`](/modules/ssh/) | `neo:ssh` | Synchronous SSH command execution with mandatory host verification | DNS, outbound TCP, SSH |
| [`stdio`](/modules/stdio/) | `neo:stdio` | Bounded in-memory byte streams and process stdout/stderr writers | Writes to process stdout/stderr |
| [`strings`](/modules/strings/) | `neo:strings` | Case-folded search and code-point trimming | No |
| [`test`](/modules/test/) | `neo:test` | Registration-based test runner used by `neojs test` | No; runs in the realm |
| [`uuid`](/modules/uuid/) | `neo:uuid` | RFC 9562 version 4 and version 7 generation | Entropy |

The subpath modules are `neo:uuid/v4` and `neo:uuid/v7`.

## Registry and import identity

A module can be reached by a canonical import and by its registry lookup:

```js
import { v4 } from "neo:uuid";

const registry = Neo.getBuiltInModule("uuid");

console.log(v4 === registry.v4); // true
```

Every named export is the same function or class object as the matching property of the registry
object. The ES module namespace and the registry object are different containers over those shared
values, not the same object.

`Neo.getBuiltInModule` takes **bare registry names**. `Neo.getBuiltInModule("neo:uuid")` returns
`undefined`. Registry lookup is separate from the import policy and is not a bare module import. An
unknown name returns `undefined`, and a non-string name throws `TypeError`.

The current dev tree also accepts bare built-in names and resolves them to the same module instance.
That compatibility behavior is not part of the canonical policy and is reserved for future
Node/package compatibility. Use `neo:` imports in new code.

Each module object is frozen and cached per realm, so repeated lookups return the same object. A new
`Runtime` is a new realm with new function objects. Assigning to a module namespace property throws
`TypeError`; adding a property to a shared function object is visible through every reference to that
function.

## What is not a built-in

Import maps, module type attributes, and bare package specifiers are not supported. Bare built-in
names are compatibility-only and reserved for future Node/package compatibility. `node:`, `npm:`,
`jsr:`, `http:`, `data:`, and `blob:` imports are rejected. In particular, `node:test` is not
`neo:test`.

`Deno.core` is reachable from JavaScript, but it is not a supported API and is not covered by these
pages.

## Reading order

- Start with the module you need in the sidebar.
- Read [Limits and unsupported features](/limits/) before assuming Node or Web APIs exist.
- Read [Remote loading and security](/remote-security/) before loading code over HTTPS.
