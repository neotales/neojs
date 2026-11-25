//! Engine-neutral synchronous cryptographic core for `vael:crypto`.
//!
//! This crate extracts the pure Rust cryptographic primitives shared by the V8
//! runtime (`vaeljs-core`) and the QuickJS runtime (`vaeljs-lite-core`) with zero
//! JavaScript engine dependencies (`deno_core`, `deno_ast`, `v8`, `rquickjs`)
//! and zero `tokio` dependencies.
//!
//! # Subsystems
//!
//! * [`envelope`]: NEO1 AES-256-GCM authenticated envelope (`encrypt`,
//!   `decrypt`) and fallible OS CSPRNG (`generate_envelope_key`, `random_bytes`).
//! * [`cipher`]: Stateful bounded [`cipher::Encryptor`], [`cipher::Decryptor`],
//!   [`cipher::CipherState`], [`cipher::CipherTable`], and
//!   [`cipher::CipherBudget`] for `aes-256-cbc` and `aes-256-gcm`.
//! * [`hash`]: Incremental [`hash::Hasher`] / [`hash::HmacHasher`],
//!   [`hash::DigestTable`], output encodings, and single-shot [`hash::digest`] /
//!   [`hash::hmac`] over `sha256`, `sha384`, and `sha512`.
//! * [`password`]: PBKDF2-HMAC and bcrypt (`$2b$` hashing and constant-time
//!   `$2a$`/`$2b$`/`$2y$` verification) with [`Zeroizing`] password/salt copies.
//! * [`shamir`]: GF(256) Shamir secret sharing (`shamir_split`,
//!   `shamir_combine`, `shamir_split_packed`) with [`Zeroizing`] buffers.
//!
//! # Example
//!
//! ```
//! use vaeljs_crypto_core::envelope::{CryptoLimits, decrypt, encrypt, generate_envelope_key};
//! use vaeljs_crypto_core::hash::{HashAlgorithm, OutputEncoding, digest};
//! use vaeljs_crypto_core::shamir::{shamir_combine, shamir_split};
//!
//! let key = generate_envelope_key()?;
//! let env = encrypt(&key, b"hello".to_vec(), b"ctx", CryptoLimits::default())?;
//! assert_eq!(decrypt(&key, env, b"ctx", CryptoLimits::default())?, b"hello");
//!
//! let hex = digest(HashAlgorithm::Sha256, b"abc", OutputEncoding::Hex)?;
//! assert_eq!(
//!     hex,
//!     b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
//! );
//!
//! let shares = shamir_split(b"secret", 2, 3)?;
//! assert_eq!(&*shamir_combine(&shares[..2])?, b"secret");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cipher;
pub mod envelope;
pub mod hash;
pub mod password;
pub mod shamir;

pub use zeroize::Zeroizing;

/// Test-only CSPRNG fault-injection hooks compiled only with `test-support`.
#[cfg(feature = "test-support")]
pub mod test_support {
    /// Make the next bcrypt salt draw fail with
    /// [`crate::password::PasswordErrorCode::Random`].
    pub fn fail_next_bcrypt_random() {
        crate::password::fail_next_bcrypt_random();
    }

    /// Make the next Shamir split noise draw fail with
    /// [`crate::shamir::ShamirErrorCode::Random`].
    pub fn fail_next_shamir_random() {
        crate::shamir::fail_next_shamir_random();
    }
}
