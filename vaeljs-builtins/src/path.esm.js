// The `path` built-in module as seen by
// `import ... from "vael:path"`.
//
// The implementations live in the runtime registry, which evaluates `path.js`
// once per realm. This file only re-exports them, so every named export is the
// same function, frozen namespace, or constant as the matching property of
// `Vael.getBuiltInModule("vael:path")`. Nothing here reaches the private module
// registry: it reads the public module object and binds its properties.
//
// The exported list must match the properties of the `path` registry object.
//
// This is a lexical path module, not Node's `path`, not `@neotales/path`, and
// not NeoLiteJS `neo:path`. It has no glob or file URL helpers, accepts no URL
// objects or coercible values, and does not read the filesystem. `posix` and
// `win32` select a fixed syntax on every host; the named exports select the
// build host's syntax. See `vaeljs-core/THIRD_PARTY_NOTICES.md` for provenance.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a function or a
// namespace. Adding a property to an exported function object is visible
// through both paths because they share the same function.
const path = Vael.getBuiltInModule("vael:path");

export const posix = path.posix;
export const win32 = path.win32;
export const join = path.join;
export const normalize = path.normalize;
export const basename = path.basename;
export const dirname = path.dirname;
export const extname = path.extname;
export const isAbsolute = path.isAbsolute;
export const parse = path.parse;
export const format = path.format;
export const common = path.common;
export const resolve = path.resolve;
export const relative = path.relative;
export const toNamespacedPath = path.toNamespacedPath;
export const SEPARATOR = path.SEPARATOR;
export const DELIMITER = path.DELIMITER;
export const SEPARATOR_PATTERN = path.SEPARATOR_PATTERN;
