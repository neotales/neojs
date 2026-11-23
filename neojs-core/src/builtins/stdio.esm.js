// The `stdio` built-in module as seen by
// `import ... from "neo:stdio"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm, and call private host ops registered by `neojs-core/src/stdio.rs`.
// This file only re-exports them, so every named export is the same class,
// constant, or shared stream object as the matching property of
// `Neo.getBuiltInModule("neo:stdio")`. Nothing here reaches the private module
// registry: it reads the public module object and binds its properties.
//
// The exported list must match the properties of the `stdio` object in
// `neo.js`. `neojs-core/tests/stdio.rs` asserts that the two lists are
// identical, that each export is identity-equal to the registry property, and
// that `stdout` and `stderr` are shared rather than per-import copies.
//
// This is a synchronous byte-stream foundation, not Node's `stream` or
// `process` module and not the WHATWG Streams API. There is no stdin, file,
// process, or environment access. Asynchronous APIs will be additive and
// Promise-based; no callback form exists here and none is reserved.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a class or a stream.
// The stream objects themselves are mutable by design: `close()` retires the
// shared wrapper without closing the process descriptor.
const stdio = Neo.getBuiltInModule("neo:stdio");

export const Readable = stdio.Readable;
export const Writable = stdio.Writable;
export const stdout = stdio.stdout;
export const stderr = stdio.stderr;
export const MAX_BYTES = stdio.MAX_BYTES;
