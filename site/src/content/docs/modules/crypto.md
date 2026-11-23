---
title: crypto
description: CSPRNG, a NeoJS AES-256-GCM envelope, stateful CBC and GCM ciphers, SHA-2, HMAC, PBKDF2, bcrypt, and Shamir secret sharing.
---

`crypto` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `neo:crypto`; the registry form is `Neo.getBuiltInModule("crypto")`.

This is a NeoJS-specific surface, not Node's `crypto` module and not the Web Crypto API. Bytes are
`Uint8Array`s and are never interpreted as text. There is no default export.

## Exports

| Group | Exports |
| --- | --- |
| Envelope and randomness | `generateEnvelopeKey`, `randomBytes`, `encrypt`, `decrypt` |
| Stateful ciphers | `createEncryptor`, `createDecryptor` |
| Hashes and MACs | `createHash`, `createHmac`, `hashAsync`, `hmacAsync` |
| Password hashing and KDF | `pbkdf2`, `pbkdf2Async`, `bcryptHash`, `bcryptHashAsync`, `bcryptVerify`, `bcryptVerifyAsync` |
| Shamir secret sharing | `shamirSplit`, `shamirCombine`, `shamirSplitAsync`, `shamirCombineAsync` |

The functions without `Async` are synchronous. The `Async` forms return a Promise and do real work
on the runtime's bounded blocking pool; they are not `Promise.resolve` wrappers around blocking
calls.

## Envelope

```js
import { decrypt, encrypt, generateEnvelopeKey } from "neo:crypto";

const key = generateEnvelopeKey();
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const context = Uint8Array.from([0x61, 0x2d, 0x64]);
const envelope = encrypt(key, plaintext, context);
const returned = decrypt(key, envelope, context);

console.log(returned.length); // 5
```

A key is exactly 32 bytes. `generateEnvelopeKey()` returns 32 bytes from the operating system
CSPRNG. `associatedData` is optional, authenticated but not encrypted, and must be supplied
unchanged to `decrypt`. `undefined` means no associated data; `null` is a `TypeError`.

The envelope is:

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 4 | Magic, ASCII `NEO1` |
| 4 | 1 | Version, `1` |
| 5 | 1 | Algorithm, `1` (AES-256-GCM) |
| 6 | 12 | Nonce |
| 18 | n | Ciphertext |
| 18 + n | 16 | Authentication tag |

`encrypt` draws a fresh 96-bit nonce for every call. The first 18 bytes and the caller's associated
data are authenticated as AES-GCM associated data. A random 96-bit nonce reaches about a 2^-32
collision probability at roughly 2^32 encryptions under one key, so retire a key well before that.

`randomBytes(length)` returns 0 through 8 MiB of CSPRNG bytes. A non-number is a `TypeError`; a
non-integer, negative, non-finite, or too-large value is a `RangeError`.

## Stateful ciphers

`createEncryptor(algorithm, key, iv[, options])` and `createDecryptor(algorithm, key, iv[, options])`
return a synchronous stateful object for `aes-256-cbc` or `aes-256-gcm`, compared
case-insensitively.

- Key: exactly 32 bytes.
- IV: exactly 16 bytes for CBC, 12 bytes for GCM.
- Caller supplies the IV/nonce and is responsible for it.
- `update(data[, inputEncoding])` buffers and returns an empty `Uint8Array`.
- `final()` performs the operation and returns the output.
- `setAAD(aad)`, `getAuthTag()`, and `setAuthTag(tag)` exist for GCM.
- `close()` (alias `dispose()`) releases native state and is idempotent.

:::danger
CBC is confidential but **not authenticated**. NeoJS computes no MAC for it. Prefer an AEAD such as
AES-256-GCM. If CBC interoperability is required, use encrypt-then-MAC with an independent MAC key,
authenticate the IV and ciphertext, verify the MAC before decrypting, and reject on any MAC failure.
Do not expose whether CBC padding is valid.
:::

No plaintext is returned by GCM before the tag is checked. The API is Node `createCipheriv`-inspired
but not Node-compatible: there is no `createCipheriv`/`createDecipheriv` export, no output encoding,
no callback or Promise form, and no claim to Node's chunk timing.

## Hash and HMAC

```js
import { createHash, createHmac, hashAsync } from "neo:crypto";

const digest = createHash("sha256").update("hello").digest("hex");
const mac = createHmac("sha256", "key").update("hello").digest("hex");
const asyncDigest = await hashAsync("sha384", "hello", "hex");

console.log(digest.length); // 64 hex characters
console.log(mac.length);    // 64 hex characters
console.log(asyncDigest.length); // 96 hex characters
```

Accepted algorithms are `sha256`, `sha384`, and `sha512`, compared case-insensitively. `update`
accepts a `Uint8Array` or a UTF-8 string; a string may name only `utf8`/`utf-8`, and a `Uint8Array`
may not name an encoding. `digest()` returns a `Uint8Array`; `digest("hex")`, `digest("base64")`,
`digest("base64url")`, and `digest("latin1")` (alias `binary`) return strings.

`digest` consumes the state. A second `digest`, or an `update` after a successful `digest`, throws
`NEO_CRYPTO_DIGESTED`. An unsupported digest encoding is rejected before the state is consumed.
There is no `copy()`.

`hashAsync(algorithm, data[, encoding])` and `hmacAsync(algorithm, key, data[, encoding])` are the
one-shot asynchronous forms. Input is copied before the Promise is returned, so mutating it
afterwards does not change the result.

## PBKDF2 and bcrypt

`pbkdf2(algorithm, password, salt, iterations, length)` returns `length` raw bytes. The algorithm is
`sha256`, `sha384`, or `sha512`. `iterations` is 1 through 1,000,000; `length` is 1 through 1,024
bytes; one password or salt is at most 1 MiB.

`bcryptHash(password[, cost])` returns a 60-character `$2b$` modular-crypt string. The password is
exact bytes or a UTF-8 string, at most 72 bytes; a longer input throws `NEO_CRYPTO_LIMIT` and is
never truncated. `cost` is 4 through 16 and defaults to 10. The salt comes from the operating
system CSPRNG.

`bcryptVerify(password, hash)` accepts `$2a$`, `$2b$`, and `$2y$` hashes. It returns `false` for a
malformed string, an unsupported version (including `$2x$`), a cost outside 4 through 16, or a
password longer than 72 bytes. A well-formed verification runs on a worker in the async form.

The password hashing destinations owned by the module are cleared with `zeroize` when the operation
finishes. This is best-effort clearing, not a complete zeroization promise: V8's internal string
storage, crate internals, and JavaScript heap copies are outside it.

## Shamir secret sharing

`shamirSplit(secret, threshold, shareCount)` splits a `Uint8Array` secret into frozen
`{ index, value }` records over GF(256). `shamirCombine(shares[, threshold])` interpolates at
`x = 0` and returns a new `Uint8Array`.

`threshold` and `shareCount` are integers from 2 through 255 with `shareCount >= threshold`. A
secret is at most 8 MiB, and `secret.length * shareCount` is at most 64 MiB. Every non-constant
coefficient comes from the operating system CSPRNG.

:::caution
Shares are not authenticated. Tampered shares, shares mixed from different splits, a wrong index, or
fewer than the original threshold reconstruct a different byte string, often without an error.
Compare the result with an authenticated envelope or an out-of-band digest when a wrong
reconstruction must be detected.
:::

## Limits

| Item | Limit |
| --- | --- |
| Envelope plaintext | 8 MiB |
| Envelope associated data | 8 MiB |
| Envelope ciphertext | 8 MiB + 34 bytes |
| `randomBytes` | 8 MiB |
| One stateful cipher input | 8 MiB |
| One GCM associated data | 8 MiB |
| Open cipher objects | 64 per runtime |
| Combined cipher buffers | 64 MiB |
| Incremental hash/HMAC input | 16 MiB |
| HMAC key | 1 MiB |
| Open hash/HMAC objects | 1,024 per runtime |
| Pending async jobs | 64 |
| Pending async copied bytes | 64 MiB |
| PBKDF2 iterations | 1,000,000 |
| PBKDF2 output | 1,024 bytes |
| bcrypt password | 72 bytes |

There are no finalizers for cipher, hash, or HMAC objects: close or digest them explicitly. `close()`
releases a slot deterministically.

## Errors

A non-`Uint8Array` byte argument is a `TypeError`. A `randomBytes` length that is not a number is a
`TypeError`; one that is not an integer or is out of range is a `RangeError`. Other failures are
`Error` objects with a stable own `code`:

| Code | Meaning |
| --- | --- |
| `NEO_CRYPTO_KEY` | Key length is wrong |
| `NEO_CRYPTO_IV` | IV/nonce length is wrong |
| `NEO_CRYPTO_ALGORITHM` | Unsupported algorithm name |
| `NEO_CRYPTO_FORMAT` | Truncated or unknown envelope |
| `NEO_CRYPTO_AUTH` | Wrong key, wrong associated data, or tampering |
| `NEO_CRYPTO_PADDING` | CBC padding failure |
| `NEO_CRYPTO_TAG` | Missing GCM authentication tag |
| `NEO_CRYPTO_STATE` | Method used after `final()` |
| `NEO_CRYPTO_CLOSED` | Method used after `close()`/`dispose()` |
| `NEO_CRYPTO_DIGESTED` | Second digest, or update after digest |
| `NEO_CRYPTO_RANDOM` | Operating system CSPRNG failure |
| `NEO_CRYPTO_LIMIT` | Input above a documented bound or refused allocation |
| `NEO_CRYPTO_DETACHED` | Transferred buffer |
| `NEO_CRYPTO_TOO_MANY_OBJECTS` | Open-object cap reached |
| `NEO_CRYPTO_BUSY` | Async job or byte cap reached |
| `NEO_CRYPTO_SHAMIR_SHARED` | A `SharedArrayBuffer`-backed share input |
| `NEO_CRYPTO_NO_EXECUTOR` | Async form used without the required Tokio executor |

Messages never include key, plaintext, password, salt, derived key, or ciphertext bytes.

## Async executor caveat

Under `Runtime::run_file_async`, asynchronous hashing, password hashing, and Shamir work need a Tokio
**current-thread** runtime to host `deno_core`'s async-op driver. A non-Tokio executor or a
multi-thread Tokio runtime gets `NEO_CRYPTO_NO_EXECUTOR` instead. `Runtime::run_file` drives the
async-op driver, so the asynchronous forms work there. The synchronous API works on every executor.

## Scope

`crypto.subtle`, `createCipheriv`/`createDecipheriv`, Node's full `Cipher` contract, AES modes other
than CBC and GCM, other KDFs, PHC/MCF/`crypt(3)` parsing, and a PLAB-compatible surface are **not
implemented**. `generateEnvelopeKey` is the envelope key generator; `generateKey` is deliberately
not exported. The module does not store, derive, wrap, rotate, or persist keys, and it does not
promise complete zeroization. It is not a sandbox.
