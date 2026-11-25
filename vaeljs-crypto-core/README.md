# vaeljs-crypto-core

Engine-neutral synchronous cryptographic core for `vael:crypto`, shared by the V8
runtime (`vaeljs-core`) and the QuickJS runtime (`vaeljs-lite-core`).

This crate carries zero JavaScript engine dependencies (`deno_core`, `deno_ast`,
`v8`, and `rquickjs` are absent) and zero `tokio` dependencies.

```sh
cargo test --locked -p vaeljs-crypto-core
cargo test --locked -p vaeljs-crypto-core --all-features
cargo clippy --locked -p vaeljs-crypto-core --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p vaeljs-crypto-core --no-deps --all-features
cargo tree --locked -p vaeljs-crypto-core -e normal
```

## Subsystems

* **`envelope`**: NEO1 AES-256-GCM authenticated envelope (`encrypt`, `decrypt`)
  and fallible OS CSPRNG (`generate_envelope_key`, `random_bytes`).
* **`cipher`**: Stateful, bounded `Encryptor` / `Decryptor`, `CipherState`,
  `CipherTable`, and `CipherBudget` for `aes-256-cbc` (PKCS#7 or strict
  no-padding) and `aes-256-gcm` (AAD and 16-byte authentication tag).
* **`hash`**: Incremental `Hasher` / `HmacHasher`, `DigestState`,
  `DigestTable`, and single-shot `digest` / `hmac` over `sha256`, `sha384`, and
  `sha512`, with `binary`, `hex`, `base64`, `base64url`, and `latin1` outputs.
* **`password`**: PBKDF2-HMAC (`sha256`, `sha384`, `sha512`) and bcrypt
  (`$2b$` hashing; `$2a$`/`$2b$`/`$2y$` constant-time verification capped at
  cost 16 and 72-byte passwords) with `Zeroizing` buffers on password and salt
  copies.
* **`shamir`**: Bytewise GF(256) Shamir secret sharing (`shamir_split`,
  `shamir_combine`, `shamir_split_packed`) using reduction polynomial `0x11b`
  and one-based share indexes, with `Zeroizing` buffers for secrets, noise, and
  shares.

## Security and resource contract

* `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.
* Every random value (`generate_envelope_key`, envelope nonces, `random_bytes`,
  bcrypt salts, Shamir polynomial coefficients) comes exclusively from
  fallible `getrandom` calls with no insecure fallback.
* Every public input, output, buffer, and open-handle table enforces documented
  size and count caps (`MAX_PLAINTEXT_BYTES`, `MAX_OPEN_CIPHERS`,
  `MAX_OPEN_CIPHER_BYTES`, `MAX_HASH_INPUT_BYTES`, `MAX_HMAC_KEY_BYTES`,
  `MAX_OPEN_DIGESTS`, `MAX_PBKDF2_ITERATIONS`, `MAX_PBKDF2_OUTPUT_BYTES`,
  `MAX_BCRYPT_COST`, `MAX_SHAMIR_SECRET_BYTES`, `MAX_SHAMIR_TOTAL_BYTES`) and
  uses fallible `try_reserve_exact` allocations.
* All error types and error codes are `#[non_exhaustive]` and map deterministically
  to the stable `VAEL_CRYPTO_*` error codes without echoing secret or input
  bytes.
