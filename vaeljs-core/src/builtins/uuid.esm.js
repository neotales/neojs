// The `uuid` built-in module as seen by
// `import ... from "vael:uuid"`.
//
// The implementations live in `vael.js`, which the runtime evaluates once per
// realm, and call private host ops registered by `vaeljs-core/src/uuid.rs`.
// This file only re-exports them, so every named export is the same function
// object as the matching property of `Vael.getBuiltInModule("vael:uuid")`. Nothing
// here reaches the private module registry: it reads the public module object
// and binds its properties.
//
// The exported list must match the properties of the `uuid` object in
// `vael.js`. `vaeljs-core/tests/uuid.rs` asserts that the two lists are identical
// and that each export is identity-equal to the registry property.
//
// Both functions return canonical lowercase RFC 9562 text (36 characters) and
// are synchronous. `v4` is random and has no order. `v7` embeds the current
// Unix time in milliseconds in its first 48 bits; values from different
// milliseconds sort by that timestamp, but values within one millisecond are
// random, and a clock step backwards can produce an earlier-sorting value. This
// module does not promise monotonicity. The `uuid/v4` and `uuid/v7` subpath
// modules are thin views over the same two function objects and also provide a
// default export.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a function.
const uuid = Vael.getBuiltInModule("vael:uuid");

export const v4 = uuid.v4;
export const v7 = uuid.v7;
