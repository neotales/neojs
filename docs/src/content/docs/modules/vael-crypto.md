---
title: vael:crypto
description: OS CSPRNG, NEO1 AES-256-GCM envelopes, AES-256-CBC/GCM ciphers, SHA-2 hashes/HMAC, PBKDF2, bcrypt, Shamir sharing, and V8 async worker pool ops.
---

`vael:crypto` provides byte-oriented cryptographic primitives powered by `vaeljs-crypto-core`: CSPRNG generation, authenticated `NEO1` AES-256-GCM envelopes, stateful AES-256 ciphers, SHA-2 hashing and HMAC, PBKDF2, bcrypt, and GF(256) Shamir secret sharing.

:::note[V8 vs Lite Parity]
All **13 synchronous exports** work identically in both `vjs` (V8) and `vjs-lite` (QuickJS). The **7 `*Async` worker-pool exports** (`hashAsync`, `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, `bcryptVerifyAsync`, `shamirSplitAsync`, `shamirCombineAsync`) are available in **`vjs` (V8) only** and are absent from `vjs-lite`.
:::

## Synchronous Exports (`vjs` and `vjs-lite`)

| Export | Signature | Limits & Notes |
| --- | --- | --- |
| `generateEnvelopeKey` | `() => Uint8Array` | Fresh 32-byte AES-256 key from OS CSPRNG. |
| `randomBytes` | `(length: number) => Uint8Array` | `0..=8_388_608` bytes (`8 MiB`) from OS CSPRNG. |
| `encrypt` | `(key, plaintext, associatedData?) => Uint8Array` | `NEO1` AES-256-GCM envelope with fresh 96-bit nonce; 32-byte key, max `8 MiB` plaintext & AAD. |
| `decrypt` | `(key, envelope, associatedData?) => Uint8Array` | Verify header + AAD + tag before returning plaintext; envelope max `8 MiB + 34 bytes`. |
| `createEncryptor` | `(algorithm, key, iv, options?) => Cipher` | `"aes-256-cbc"` (16-byte IV, `{ padding?: boolean }`) or `"aes-256-gcm"` (12-byte IV). |
| `createDecryptor` | `(algorithm, key, iv, options?) => Cipher` | GCM decryptors must call `setAuthTag(tag)` before `final()`. |
| `createHash` | `(algorithm: HashAlgorithm) => Hash` | `"sha256"`, `"sha384"`, or `"sha512"` (case-insensitive). |
| `createHmac` | `(algorithm: HashAlgorithm, key: Uint8Array \| string) => Hash` | Key max `1 MiB`. |
| `pbkdf2` | `(algorithm, password, salt, iterations, length) => Uint8Array` | Password/salt max `1 MiB`; `iterations` `1..=1_000_000`; `length` `1..=1024`. |
| `bcryptHash` | `(password, cost?: number) => string` | `$2b$` 60-char hash; password max `72` bytes (never truncated); `cost` `4..=16` (default `10`). |
| `bcryptVerify` | `(password, hash: string) => boolean` | Constant-time verify (`$2a$`/`$2b$`/`$2y$`). Returns `false` if password > 72 bytes or cost > 16. |
| `shamirSplit` | `(secret, threshold, shareCount) => ShamirShare[]` | `secret` `1..=8 MiB`, `2 <= threshold <= shareCount <= 255`, total `<= 64 MiB`. Rejects `SharedArrayBuffer`. |
| `shamirCombine` | `(shares: readonly ShamirShare[], threshold?) => Uint8Array` | `2..=255` shares of equal non-zero length (max `8 MiB` each, `64 MiB` total). Shares are **not** authenticated. |

## Asynchronous Worker-Pool Exports (`vjs` V8 Only)

These 7 exports offload CPU-intensive work to a bounded Tokio worker pool sharing a **`64` pending jobs / `64 MiB` total** budget (rejecting with `VAEL_CRYPTO_BUSY` if saturated, or `VAEL_CRYPTO_NO_EXECUTOR` when no Tokio executor is present):

- `hashAsync(algorithm, data, encoding?)`: `Promise<Uint8Array | string>` (input max `16 MiB`)
- `hmacAsync(algorithm, key, data, encoding?)`: `Promise<Uint8Array | string>`
- `pbkdf2Async(algorithm, password, salt, iterations, length)`: `Promise<Uint8Array>`
- `bcryptHashAsync(password, cost?)`: `Promise<string>`
- `bcryptVerifyAsync(password, hash)`: `Promise<boolean>`
- `shamirSplitAsync(secret, threshold, shareCount)`: `Promise<ShamirShare[]>`
- `shamirCombineAsync(shares, threshold?)`: `Promise<Uint8Array>`

## Stateful `Cipher` and `Hash` Lifecycles

- **`Cipher`**: `update(data, encoding?)` accepts up to `8 MiB` per call and always returns an empty `Uint8Array` (all output is produced by `final()`). Supports `setAAD(aad)` (max `8 MiB`), `setAuthTag(tag)` (16 bytes, GCM decryptor), `getAuthTag()` (GCM encryptor after `final()`), and `setAutoPadding(enabled)` (CBC). Open ciphers are capped at **`64` objects / `64 MiB` buffered bytes** per runtime — call `close()` or `dispose()` in a `finally` block even after `final()`.
- **`Hash`**: `update(data, encoding?)` accepts `"utf8"` / `"utf-8"` for string encoding and enforces `MAX_HASH_INPUT_BYTES` (`16 MiB`) **cumulatively** across all `update()` calls on a `Hash`/`Hmac` instance. `digest(encoding?)` consumes the state and returns `Uint8Array` or a string (`"hex"`, `"base64"`, `"base64url"`, `"latin1"`, `"binary"`). Open digest objects are capped at **`1024` per runtime**; call `close()` or `dispose()` if abandoning before `digest()`.

## Error Codes (`CryptoErrorCode`)

`VAEL_CRYPTO_KEY` | `VAEL_CRYPTO_FORMAT` | `VAEL_CRYPTO_AUTH` | `VAEL_CRYPTO_RANDOM` | `VAEL_CRYPTO_LIMIT` | `VAEL_CRYPTO_DETACHED` | `VAEL_CRYPTO_ALGORITHM` | `VAEL_CRYPTO_IV` | `VAEL_CRYPTO_TAG` | `VAEL_CRYPTO_PADDING` | `VAEL_CRYPTO_STATE` | `VAEL_CRYPTO_CLOSED` | `VAEL_CRYPTO_UNSUPPORTED` | `VAEL_CRYPTO_TOO_MANY_OBJECTS` | `VAEL_CRYPTO_ENCODING` | `VAEL_CRYPTO_DIGESTED` | `VAEL_CRYPTO_BUSY` | `VAEL_CRYPTO_NO_EXECUTOR` *(V8 only)* | `VAEL_CRYPTO_INTERNAL` | `VAEL_CRYPTO_SHAMIR_DUPLICATE` | `VAEL_CRYPTO_SHAMIR_LENGTH` | `VAEL_CRYPTO_SHAMIR_SHARED`

## Example

```ts
import { createHash, decrypt, encrypt, generateEnvelopeKey } from "vael:crypto";
import { Buffer } from "node:buffer";

// NEO1 authenticated AES-256-GCM envelope (works on both vjs and vjs-lite)
const key = generateEnvelopeKey();
const aad = Buffer.from("tenant:42", "utf8");
const plaintext = Buffer.from("secret payload", "utf8");

const envelope = encrypt(key, plaintext, aad);
const recovered = decrypt(key, envelope, aad);
console.log(Buffer.from(recovered).toString("utf8"));

// Incremental SHA-256 digest
const h = createHash("sha256");
try {
  h.update("hello ").update("world");
  console.log(h.digest("hex"));
} finally {
  h.close();
}
```
