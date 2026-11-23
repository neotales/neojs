// The `chars` built-in module as seen by
// `import ... from "neo:chars"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm. This file only re-exports them, so every named export is the same
// function object as the matching property of
// `Neo.getBuiltInModule("neo:chars")`. Nothing here reaches the private module
// registry: it reads the public module object and binds its properties.
//
// The exported list must match the properties of the `chars` object in
// `neo.js`. `neojs-core/tests/runtime.rs` asserts that the two lists are
// identical and that each export is identity-equal to the registry property.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a function. Adding a
// property to an exported function object is visible through both paths
// because they share the same function.
const chars = Neo.getBuiltInModule("neo:chars");

export const isChar = chars.isChar;
export const isAscii = chars.isAscii;
export const isLatin1 = chars.isLatin1;
export const isControl = chars.isControl;
export const isDigit = chars.isDigit;
export const isLetter = chars.isLetter;
export const isLetterOrDigit = chars.isLetterOrDigit;
export const isLower = chars.isLower;
export const isUpper = chars.isUpper;
export const isPunc = chars.isPunc;
export const isSpace = chars.isSpace;
export const isSymbol = chars.isSymbol;
export const toLower = chars.toLower;
export const toUpper = chars.toUpper;
