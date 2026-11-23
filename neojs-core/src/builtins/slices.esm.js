// The `slices` built-in module as seen by `import ... from "slices"` or
// `import ... from "neo:slices"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm. This file only re-exports them, so every named export is the same
// class or function object as the matching property of
// `Neo.getBuiltInModule("slices")`. The ES module namespace is a distinct object
// from that registry object; only the exported class and function identities
// are shared. Nothing here reaches the private module registry: it reads the
// public module object and binds its properties.
//
// The exported list must match the properties of the `slices` object in
// `neo.js`. `neojs-core/tests/slices.rs` asserts that the two lists are
// identical and that each export is identity-equal to the registry property.
//
// The namespace object is immutable by the language, and the registry object it
// reads is frozen, so neither path can replace a class or function. Adding a
// property to an exported class or function is visible through both paths
// because they share the same object. View instances themselves are live
// aliases of the caller's Array or typed array, not copies.
const slices = Neo.getBuiltInModule("slices");

export const CharSlice = slices.CharSlice;
export const ReadOnlyCharSlice = slices.ReadOnlyCharSlice;
export const ReadOnlySlice = slices.ReadOnlySlice;
export const Slice = slices.Slice;
export const charSlice = slices.charSlice;
export const readonlyCharSlice = slices.readonlyCharSlice;
export const readonlySlice = slices.readonlySlice;
export const slice = slices.slice;
