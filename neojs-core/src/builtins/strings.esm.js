// The `strings` built-in module as seen by `import ... from "strings"` or
// `import ... from "neo:strings"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm. This file only re-exports them, so every named export is the same
// function object as the matching property of
// `Neo.getBuiltInModule("strings")`. Nothing here reaches the private module
// registry: it reads the public module object and binds its properties.
//
// The exported list must match the properties of the `strings` object in
// `neo.js`. `neojs-core/tests/runtime.rs` asserts that the two lists are
// identical and that each export is identity-equal to the registry property.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a function. Adding a
// property to an exported function object is visible through both paths
// because they share the same function.
const strings = Neo.getBuiltInModule("strings");

export const equalFold = strings.equalFold;
export const startsWithFold = strings.startsWithFold;
export const endsWithFold = strings.endsWithFold;
export const indexOfFold = strings.indexOfFold;
export const lastIndexOfFold = strings.lastIndexOfFold;
export const trim = strings.trim;
export const trimStart = strings.trimStart;
export const trimEnd = strings.trimEnd;
