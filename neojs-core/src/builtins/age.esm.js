// The `age` built-in module as seen by `import ... from "age"` or
// `import ... from "neo:age"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm, and call private host ops registered by `neojs-core/src/age.rs`. This
// file only re-exports them, so every named export is the same function object
// as the matching property of `Neo.getBuiltInModule("age")`. Nothing here
// reaches the private module registry: it reads the public module object and
// binds its properties.
//
// The exported list must match the properties of the `age` object in `neo.js`.
// `neojs-core/tests/age.rs` asserts that the two lists are identical and that
// each export is identity-equal to the registry property.
//
// This is the synchronous binary age v1 X25519 slice only. Asynchronous APIs
// will be additive and Promise-based; ASCII armor, passphrases, SSH keys,
// plugins, and post-quantum recipients are separate work and are not exported
// here. Do not infer them from this list.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a function. Adding a
// property to an exported function object is visible through both paths because
// they share the same function.
const age = Neo.getBuiltInModule("age");

export const generateIdentity = age.generateIdentity;
export const identityToRecipient = age.identityToRecipient;
export const encrypt = age.encrypt;
export const decrypt = age.decrypt;
