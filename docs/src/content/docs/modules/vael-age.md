---
title: vael:age
description: Synchronous binary age v1 X25519 encryption and decryption with authenticated header MACs and STREAM chunks.
---

`vael:age` provides synchronous binary **age v1** encryption and decryption for X25519 keys, backed by `neotales-age`.

:::note[Availability]
`vael:age` is available in **`vjs` (V8) only**. It is **not** available in `vjs-lite`.
:::

## Exports

| Export | Signature | Description |
| --- | --- | --- |
| `generateIdentity` | `() => AgeIdentity` | Generate an X25519 `{ identity, recipient }` pair (`AGE-SECRET-KEY-1...` / `age1...`) using the OS CSPRNG. |
| `identityToRecipient` | `(identity: string) => string` | Derive the Bech32 `age1...` recipient for an `AGE-SECRET-KEY-1...` identity (max `512` bytes, trims whitespace). |
| `encrypt` | `(recipient: string, plaintext: Uint8Array) => Uint8Array` | Encrypt `plaintext` to one X25519 `recipient` as a binary age v1 file. |
| `decrypt` | `(identity: string, ciphertext: Uint8Array) => Uint8Array` | Verify header MAC and all STREAM payload tags, then return plaintext. |

`AgeIdentity` is `{ identity: string; recipient: string }`.

## Format Interop & Scope

- **Standard binary age v1**: Files produced by `encrypt()` are standard binary age v1 files interoperable with `age` and `rage` CLI tools using X25519 keys.
- **Explicitly unsupported formats**: ASCII armor, passphrase (`scrypt`) files, SSH keys, plugins, and post-quantum keys are **not** supported. Non-X25519 keys fitting the 512-byte bound fail with `VAEL_AGE_UNSUPPORTED`, and ASCII-armored or passphrase files fail with `VAEL_AGE_FORMAT`.
- **Distinct from `vael:crypto` `NEO1`**: `vael:age` implements asymmetric X25519 age v1 files, whereas `vael:crypto`'s `encrypt`/`decrypt` functions use a symmetric 32-byte key with VaelJS's `NEO1` AES-256-GCM envelope format.

## Size Limits & Guarantees

- `encrypt()` accepts at most **`8 MiB` (`8_388_608` bytes)** of plaintext and produces at most **`9 MiB` (`9_437_184` bytes)** of ciphertext.
- `decrypt()` accepts at most **`9 MiB`** of ciphertext, a header up to `64 KiB`, at most `1024` recipient stanzas (`64 KiB` max stanza body), and returns at most **`8 MiB`** of plaintext.
- `decrypt()` authenticates the header MAC and every STREAM chunk tag before returning any plaintext; failed checks discard the buffer without leaking unauthenticated bytes.
- Error messages are static and never echo identities, recipients, or payload bytes.

## Error Codes (`AgeErrorCode`)

Wrong argument types or wrong argument counts (including trailing `undefined`) throw `TypeError`. Operational failures throw `Error` with `code` set to one of:

| Code | Meaning |
| --- | --- |
| `"VAEL_AGE_IDENTITY"` | Malformed or invalid age X25519 secret identity string. |
| `"VAEL_AGE_RECIPIENT"` | Invalid X25519 recipient string or low-order public key point. |
| `"VAEL_AGE_UNSUPPORTED"` | Valid age key or stanza of a non-X25519 type (such as post-quantum). |
| `"VAEL_AGE_FORMAT"` | Truncated, malformed, armored, or passphrase-encrypted age input. |
| `"VAEL_AGE_IDENTITY_MISMATCH"` | Identity did not match any recipient stanza in the file. |
| `"VAEL_AGE_AUTH"` | Header MAC or STREAM payload authentication tag verification failed. |
| `"VAEL_AGE_DETACHED"` | Supplied `Uint8Array` is backed by a detached `ArrayBuffer`. |
| `"VAEL_AGE_LIMIT"` | Input or output exceeded the `8 MiB` plaintext / `9 MiB` ciphertext / header bounds. |
| `"VAEL_AGE_RANDOM"` | OS CSPRNG failed. |
| `"VAEL_AGE_INTERNAL"` | Unexpected backend failure. |

## Example

```ts
import { decrypt, encrypt, generateIdentity, identityToRecipient } from "vael:age";

const { identity, recipient } = generateIdentity();
if (identityToRecipient(identity) !== recipient) {
  throw new Error("recipient mismatch");
}

const secret = new TextEncoder().encode("deployment token");
const ciphertext = encrypt(recipient, secret);
const plaintext = decrypt(identity, ciphertext);

console.log(new TextDecoder().decode(plaintext));
```
