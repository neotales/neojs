---
title: stdio
description: Bounded in-memory byte streams and writers to the process stdout and stderr, synchronous only.
---

`stdio` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `neo:stdio`; the registry form is `Neo.getBuiltInModule("stdio")`.

It is a synchronous byte-stream foundation, not Node's `stream` or `process` module and not the
WHATWG Streams API. No Node or Web compatibility is promised.

## Exports

| Export | Meaning |
| --- | --- |
| `Readable` | Reader over a copied byte snapshot |
| `Writable` | Memory-backed or process-descriptor writer |
| `stdout`, `stderr` | One shared memory-free writer per process stream |
| `MAX_BYTES` | `8388608` (8 MiB) |

There is no default export and no `stdin`.

## Example

```js
import { Readable, Writable, stdout, stderr } from "neo:stdio";

const writer = new Writable();
writer.writeSync(Uint8Array.from([0x00, 0xff]));
writer.writeSync("é"); // 2, because the string is UTF-8
const bytes = writer.toUint8Array(); // fresh, caller-owned copy

const reader = new Readable(bytes);
const target = new Uint8Array(16);
let count;
while ((count = reader.readSync(target)) !== 0) {
    // use target.subarray(0, count); 0 is end of file
}

stderr.writeSync("diagnostic\n");
stdout.writeSync("result\n");
```

## Reading

`new Readable(bytes)` copies the current byte range of an attached `Uint8Array` (at most
`MAX_BYTES`) into reader-owned storage. Mutating, detaching, or resizing the caller's buffer
afterwards does not change the reader.

`readSync(target)` copies at most `target.byteLength` bytes and returns the count. It can copy fewer
bytes than the target holds, so loop until it returns `0`. Exactly `0` means end of file; end of
file is not an exception. An empty target throws `NEO_STDIO_ARGUMENT` rather than returning an
ambiguous zero, even at end of file.

`remaining` is the unread byte count. `closed` reports `close()`. `close()` releases the snapshot,
is idempotent, and never detaches a caller's buffer. Reads after it throw `NEO_STDIO_CLOSED`.

## Writing

`new Writable()` creates a memory-backed writer implemented as one coalesced, owned buffer whose
capacity is bounded by `MAX_BYTES`.

- An empty write retains nothing.
- `writeSync(chunk)` accepts a `Uint8Array` or a string encoded as UTF-8 and returns the byte count,
  not UTF-16 code units.
- A memory writer is all-or-nothing: it appends the whole chunk or throws without changing the
  stream, and never returns a short count.
- `length` is the retained byte count.
- `toUint8Array()` copies only the used prefix into a fresh caller-owned array.
- `end()` retires the writer but keeps memory-backed content readable.
- `close()` is terminal and releases the content.
- Both are idempotent. A closed writer reports `length = 0` and throws `NEO_STDIO_CLOSED`.

## Process streams

`stdout` and `stderr` are one shared memory-free `Writable` each, bound to the current process's
standard output and standard error. Writing to them adds no capability beyond the installed
`console`, which already writes to those descriptors.

`writeSync` blocks the isolate until the operating system accepts every byte or an error occurs. It
does not yield to the event loop, has no timeout, and cannot be cancelled. It returns the full byte
count on success. A throw can mean that a prefix was already written, because a system write can be
partially completed before failing; there is no rollback or retry.

`stdout.length` and `stderr.length` are always `0` because a host writer retains nothing, and
`toUint8Array()` throws `NEO_STDIO_NOT_MEMORY` (or `NEO_STDIO_CLOSED` after `close()`). Their
`kind` is `"stdout"` or `"stderr"`. `end()` and `close()` retire only this wrapper; they do not close
the process descriptor, so `console` keeps working.

There is no `stdin`: reading the process's input would block on input and add an input capability
this slice does not grant.

## Limits and errors

`MAX_BYTES` is 8 MiB. It bounds a reader's snapshot, a single `writeSync` chunk, and a memory
writer's retained content. The limit is per stream; there is no aggregate budget in this slice, and
a `Uint8Array` returned by `toUint8Array()` is JavaScript-owned and outside it. A refused copy or
append happens before any state changes.

Sizes are checked from V8's internal view length, not a JavaScript getter. A detached view throws
`NEO_STDIO_DETACHED`; a forged, proxied, or non-view value throws `TypeError`. Offset views are
copied from their own byte range; shared and resizable backing buffers are accepted because the copy
runs without executing JavaScript.

Other failures are an ordinary `Error` with `name = "StdioError"` and a stable own `code`:

`NEO_STDIO_ARGUMENT`, `NEO_STDIO_DETACHED`, `NEO_STDIO_LIMIT`, `NEO_STDIO_CLOSED`,
`NEO_STDIO_ENDED`, `NEO_STDIO_NOT_MEMORY`, and `NEO_STDIO_WRITE`.

There is no exported error constructor; test `error.name === "StdioError"` and the `code`. Messages
never include the rejected bytes; a write failure names the stream and the operating-system error.
A closed stream reports its closed code before validating the argument. Argument counts are strict,
and a callback or other extra argument is rejected rather than ignored.

## Scope

This slice is synchronous only. It returns no Promise and takes no callback, and it does not wrap
blocking work in an already-resolved Promise to look asynchronous. A future asynchronous surface
will be additive and Promise-based.

There is no `pipe`, `on`, event, backpressure, or `Buffer` surface, and none is promised. The module
is not a permission, sandbox, or isolation boundary; `Deno.core` remains reachable.
