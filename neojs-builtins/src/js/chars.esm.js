// The canonical `neo:chars` ECMAScript module wrapper.
//
// Both engines serve this source through their own module loader. It reads the
// public module object installed by the realm shim and re-exports its function
// objects, so the module namespace and `Neo.getBuiltInModule("neo:chars")`
// share identity: every named export is the same function object as the
// matching registry property, and repeated imports return one module instance.
//
// The registry key is canonical. The shared module has no bare `chars` alias;
// an engine that still accepts a legacy bare name resolves it to this canonical
// module in its own loader adapter. The module namespace is immutable by the
// language and the registry object is frozen, so neither path can replace a
// function. Adding a property to an exported function object is visible through
// both paths because they share the same function.
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
