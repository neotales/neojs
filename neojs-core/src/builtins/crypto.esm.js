// The `crypto` built-in module as seen by `import ... from "crypto"` or
// `import ... from "neo:crypto"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm, and call private host ops registered by `neojs-core/src/crypto.rs` and
// `neojs-core/src/crypto_hash.rs`. This file only re-exports them, so every
// named export is the same function
// object as the matching property of `Neo.getBuiltInModule("crypto")`. Nothing
// here reaches the private module registry: it reads the public module object
// and binds its properties.
//
// The exported list must match the properties of the `crypto` object in
// `neo.js`. `neojs-core/tests/crypto.rs` and
// `neojs-core/tests/crypto_hash.rs` assert that the two lists are identical and
// that each export is identity-equal to the registry property.
//
// This is the synchronous envelope-and-randomness slice, bounded stateful
// AES-256-CBC/AES-256-GCM ciphers, SHA-2 hashing, HMAC, PBKDF2, and bcrypt.
// `createEncryptor`/`createDecryptor` are stateful and synchronous: `update`
// buffers and `final` performs the operation. They are Node-inspired, not Node
// compatible, and have no Promise form. Incremental `createHash`/`createHmac`,
// `pbkdf2`, `bcryptHash`, and `bcryptVerify` are synchronous; `hashAsync`,
// `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, and `bcryptVerifyAsync` return
// a Promise resolved by a bounded worker pool. WebCrypto's `subtle`, other
// KDFs, and PLAB-compatible surfaces are separate, additive work; they are not
// exported here and must not be inferred from this list.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a function. Adding a
// property to an exported function object is visible through both paths
// because they share the same function.
const crypto = Neo.getBuiltInModule("crypto");

export const bcryptHash = crypto.bcryptHash;
export const bcryptHashAsync = crypto.bcryptHashAsync;
export const bcryptVerify = crypto.bcryptVerify;
export const bcryptVerifyAsync = crypto.bcryptVerifyAsync;
export const createDecryptor = crypto.createDecryptor;
export const createEncryptor = crypto.createEncryptor;
export const createHash = crypto.createHash;
export const createHmac = crypto.createHmac;
export const decrypt = crypto.decrypt;
export const encrypt = crypto.encrypt;
export const generateEnvelopeKey = crypto.generateEnvelopeKey;
export const hashAsync = crypto.hashAsync;
export const hmacAsync = crypto.hmacAsync;
export const pbkdf2 = crypto.pbkdf2;
export const pbkdf2Async = crypto.pbkdf2Async;
export const randomBytes = crypto.randomBytes;

