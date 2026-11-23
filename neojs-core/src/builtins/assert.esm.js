// The `assert` built-in module as seen by `import ... from "assert"` or
// `import ... from "neo:assert"`.
//
// The implementation lives in `neo.js`, which the runtime evaluates once per
// realm. This file only re-exports it, so every named export is the same
// function object as the matching property of `Neo.getBuiltInModule("assert")`,
// and the default export is that callable object itself. The ES module namespace
// is a distinct object from the registry object; only the exported function
// identity is shared. Nothing here reaches the private module registry: it reads
// the public module object and binds its properties.
//
// The exported list must match the properties of the `assert` function in
// `neo.js`. `neojs-core/tests/runtime.rs` asserts that the lists are identical,
// that each named export is identity-equal to the registry property, and that
// the default export is the callable `assert`. The namespace is immutable by the
// language, and the registry object it reads is frozen, so neither path can
// replace a function.
//
// The surface is a neojs-specific assertion API inspired by Node's
// `assert/strict` and Deno's `@std/assert`; it is not a compatibility layer.
// In particular, structural equality is `equals` and identity is
// `strictEquals`. There is no `assertEquals`, `deepEqual`, or `strictEqual`.
const assert = Neo.getBuiltInModule("assert");

export default assert;
export { assert };
export const AssertionError = assert.AssertionError;
export const ok = assert.ok;
export const equals = assert.equals;
export const notEquals = assert.notEquals;
export const strictEquals = assert.strictEquals;
export const notStrictEquals = assert.notStrictEquals;
export const fail = assert.fail;
export const throws = assert.throws;
export const doesNotThrow = assert.doesNotThrow;
export const rejects = assert.rejects;
export const doesNotReject = assert.doesNotReject;
