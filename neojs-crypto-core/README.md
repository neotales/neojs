# neojs-crypto-core

The engine-neutral backend for the synchronous `neo:crypto` slice: the NEO1
AES-256-GCM envelope, the operating-system CSPRNG calls, the size policy, and
the stable error categories.

It has no dependency on V8, `deno_core`, QuickJS, or `rquickjs`. The V8 runtime
(`neojs-core`) and the QuickJS runtime (`neojs-lite-core`) both call this crate,
so an envelope produced through one adapter is the same bytes the other adapter
opens. The engine adapters own only argument validation, view copying, and the
JavaScript error objects; none of them reimplement the format.

## Surface

* `generate_envelope_key()` returns 32 CSPRNG bytes.
* `random_bytes(length)` returns at most 8 MiB of CSPRNG bytes.
* `encrypt(key, plaintext, associated_data, limits)` emits one NEO1 envelope.
* `decrypt(key, envelope, associated_data, limits)` opens one NEO1 envelope.

Keys are exactly 32 bytes. A wrong key length, a truncated or unknown envelope,
a failed authentication tag, a CSPRNG failure, and an exceeded bound are
distinct `CryptoErrorCode` values. No error message contains key, plaintext,
ciphertext, or associated-data bytes.

## Envelope

```text
offset  size  field
0       4     magic, ASCII "NEO1"
4       1     envelope version, 1
5       1     algorithm, 1 = AES-256-GCM
6       12    nonce
18      n     ciphertext (same length as the plaintext)
18+n    16    authentication tag
```

The header, nonce, and caller associated data are the AES-GCM associated data.
A different header, nonce, or associated data fails authentication rather than
returning plaintext. The magic, version, and algorithm are checked before the
primitive runs, so an unknown format is a format failure, not an authentication
failure.

This crate is not a sandbox, a key store, or a key-lifecycle manager. It does
not promise zeroization of Rust or JavaScript buffers. Callers use a distinct
key per purpose and retire it well before about 2^32 messages, where the
probability of a 96-bit nonce collision under one key reaches about 2^-32.

## Test vectors

The unit tests check the AES-256-GCM primitive against the published McGrew and
Viega Appendix B vectors and the NEO1 envelope against two fixtures produced
independently with Node's OpenSSL-backed AES-256-GCM. They also cover tamper
rejection, argument and size boundaries, and random-output shape. They cannot
prove that the operating-system CSPRNG is statistically random; that property
comes from `getrandom`.
