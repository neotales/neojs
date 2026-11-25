// The `assert` built-in module as seen by
// `import ... from "vael:assert"`.
//
// The implementation lives in the runtime registry, which evaluates `assert.js`
// once per realm. This file only re-exports it, so every named export is the
// same function object as the matching property of
// `Vael.getBuiltInModule("vael:assert")`, and the default export is that callable
// object itself. The ES module namespace is a distinct object from the registry
// object; only the exported function identity is shared. Nothing here reaches
// the private module registry: it reads the public module object and binds its
// properties.
//
// The surface is a vaeljs-specific assertion API inspired by Node's
// `assert/strict` and Deno's `@std/assert`; it is not a compatibility layer.
// In particular, structural equality is `equals` and identity is
// `strictEquals`. There is no `assertEquals`, `deepEqual`, or `strictEqual`.
const assert = Vael.getBuiltInModule("vael:assert");

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
