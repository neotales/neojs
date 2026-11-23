// The `chars` built-in module as seen by `import ... from "chars"` or
// `import ... from "neo:chars"`.
//
// The implementation lives in the engine-neutral `neojs-builtins` crate. The
// V8 realm shim splices that expression in before the entry module, so every
// named export here is the same function object as the matching property of
// `Neo.getBuiltInModule("chars")`. The ES module namespace is a distinct object
// from that registry object; only the exported function identities are shared.
// Nothing here reaches the private module registry: it reads the public module
// object and binds its properties.
//
// The exported list must match the properties of the `chars` object in
// `neo.js`. `neojs-core/tests/runtime.rs` asserts that the two lists are
// identical and that each export is identity-equal to the registry property.
//
// The namespace object is immutable by the language, and the registry object it
// reads is frozen, so neither path can replace a function. Adding a property to
// an exported function object is visible through both paths because they share
// the same function.
const chars = Neo.getBuiltInModule("chars");

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
