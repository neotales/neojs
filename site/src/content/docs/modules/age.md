---
title: age
description: X25519 identity generation and binary age v1 encryption and decryption, synchronous and byte-oriented.
---

`age` is available in every V8 runtime; there is no secure-context or TLS gate. It is **not**
available in NeoJS Lite.

Import it as `age` or `neo:age`. The registry form is `Neo.getBuiltInModule("age")`.

## Exports

| Export | Signature | Result |
| --- | --- | --- |
| `generateIdentity` | `generateIdentity()` | `{ identity, recipient }` with fresh X25519 key text |
| `identityToRecipient` | `identityToRecipient(identity)` | The matching `age1...` recipient |
| `encrypt` | `encrypt(recipient, plaintext)` | A standard **binary** age v1 file as `Uint8Array` |
| `decrypt` | `decrypt(identity, ciphertext)` | The original plaintext as `Uint8Array` |

All four are synchronous. They take and return bytes; bytes are never interpreted as text. The API
supports X25519 identities and recipients and binary age v1 only.

## Example

```js
import { decrypt, encrypt, generateIdentity } from "age";

const pair = generateIdentity(); // { identity, recipient }
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const ciphertext = encrypt(pair.recipient, plaintext);
const returned = decrypt(pair.identity, ciphertext);

console.log(pair.identity.startsWith("AGE-SECRET-KEY-1")); // true
console.log(returned.length); // 5
```

The identity is the secret half. Keep it secret. `generateIdentity` draws from the operating system
CSPRNG. Leading and trailing whitespace is trimmed from identity and recipient text; whitespace
inside the value is rejected.

## Limits

These are NeoJS limits, not age format limits.

| Operation | Limit |
| --- | --- |
| `encrypt` plaintext | 8 MiB |
| `encrypt` output | 8 MiB + 1 MiB |
| `decrypt` ciphertext | 8 MiB + 1 MiB |
| Decryption header | 64 KiB |
| Recipient stanzas | 1,024 |
| One stanza body | 64 KiB |
| `decrypt` plaintext | 8 MiB |
| Key text | 512 bytes |

A `Uint8Array` view is read through its own range, so an offset view copies only its bytes. A
detached view is `NEO_AGE_DETACHED`, not empty input. `SharedArrayBuffer`-backed and resizable
`ArrayBuffer` views are accepted.

## Errors

Every failure other than a `TypeError` is an `Error` with a stable own `code`:

| Code | Meaning |
| --- | --- |
| `NEO_AGE_IDENTITY` | The identity text is not a valid age X25519 identity |
| `NEO_AGE_RECIPIENT` | The recipient is invalid, or its public key is a low-order point |
| `NEO_AGE_UNSUPPORTED` | A valid age key of another kind that fits the key-text bound |
| `NEO_AGE_FORMAT` | Truncated, malformed, or unsupported file, including ASCII armor or a passphrase file |
| `NEO_AGE_IDENTITY_MISMATCH` | No supplied identity matched the file's recipients |
| `NEO_AGE_AUTH` | A header MAC or payload authentication tag failed |
| `NEO_AGE_LIMIT` | An input or output exceeded a documented bound |
| `NEO_AGE_RANDOM` | The operating system CSPRNG failed |
| `NEO_AGE_DETACHED` | A transferred `Uint8Array` view |
| `NEO_AGE_INTERNAL` | The backend reported an unexpected failure |

Messages are fixed text and never include an identity, recipient, plaintext, ciphertext, or key
bytes. A failed `decrypt` never returns partial or unauthenticated plaintext: the header MAC is
verified before payload decryption, every STREAM chunk is authenticated, and any failure discards
the working buffer.

## Scope

This module does **not** implement ASCII armor, passphrases (scrypt), SSH keys, plugins, tagged HPKE
recipients, or post-quantum recipients. A valid age key of another kind is rejected with
`NEO_AGE_UNSUPPORTED`; an armored file is `NEO_AGE_FORMAT`. It is not a general `age` command or a
compatibility layer for those formats.

The module does not promise zeroization. Identity text, plaintext, ciphertext, and intermediate key
material are copied into Rust buffers and the JavaScript heap; those copies are reclaimed normally
but are not guaranteed to be erased first. It does not store, derive, wrap, rotate, or persist keys,
and it performs no filesystem or network access. It is not a sandbox or a permission boundary.

The backend is the MIT-licensed `neotales-age` crate. Binary age v1 output is standard; interop is
checked against the `age` 1.3.2 command-line tool in the repository test suite.
