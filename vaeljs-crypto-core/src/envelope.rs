//! NEO1 AES-256-GCM authenticated envelope and CSPRNG helpers.
//!
//! Every value returned by [`encrypt`] has this layout:
//!
//! ```text
//! offset  size  field
//! 0       4     magic, ASCII "NEO1"
//! 4       1     envelope version, 1
//! 5       1     algorithm, 1 = AES-256-GCM
//! 6       12    nonce
//! 18      n     ciphertext (same length as the plaintext)
//! 18+n    16    authentication tag
//! ```
//!
//! The first 18 bytes (header and nonce) followed by the caller's associated
//! data are passed to AES-GCM as associated data. Magic, version, and
//! algorithm bytes are checked before the primitive runs so unknown formats
//! report [`EnvelopeErrorCode::Format`] rather than authentication failure.

use std::fmt;

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce, Tag};
use thiserror::Error;
use zeroize::Zeroizing;

/// Length of an AES-256 key in bytes.
pub const KEY_LEN: usize = 32;
/// Length of the AES-GCM nonce in bytes.
pub const NONCE_LEN: usize = 12;
/// Length of the AES-GCM authentication tag in bytes.
pub const TAG_LEN: usize = 16;

const MAGIC: &[u8; 4] = b"NEO1";
const VERSION: u8 = 1;
const ALGORITHM_AES_256_GCM: u8 = 1;
const HEADER_LEN: usize = 6;
/// Header, nonce, and tag prepended/appended around the ciphertext.
pub const OVERHEAD: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

/// Largest plaintext accepted by the public module.
pub const MAX_PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
/// Largest caller associated-data value accepted by the public module.
pub const MAX_ASSOCIATED_DATA_BYTES: usize = 8 * 1024 * 1024;
/// Largest complete envelope accepted by the public module.
pub const MAX_ENVELOPE_BYTES: usize = MAX_PLAINTEXT_BYTES + OVERHEAD;
/// Largest byte count accepted by one [`random_bytes`] call.
pub const MAX_RANDOM_BYTES: usize = 8 * 1024 * 1024;

/// Size bounds used by [`encrypt`] and [`decrypt`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CryptoLimits {
    /// Largest plaintext accepted, in bytes.
    pub max_plaintext: usize,
    /// Largest caller associated-data value accepted, in bytes.
    pub max_associated_data: usize,
}

impl Default for CryptoLimits {
    fn default() -> Self {
        Self {
            max_plaintext: MAX_PLAINTEXT_BYTES,
            max_associated_data: MAX_ASSOCIATED_DATA_BYTES,
        }
    }
}

impl CryptoLimits {
    /// Reject a plaintext length above [`Self::max_plaintext`].
    pub fn check_plaintext(self, length: usize) -> Result<(), EnvelopeError> {
        if length > self.max_plaintext {
            Err(EnvelopeError::new(EnvelopeErrorCode::Limit))
        } else {
            Ok(())
        }
    }

    /// Reject an associated-data length above [`Self::max_associated_data`].
    pub fn check_associated_data(self, length: usize) -> Result<(), EnvelopeError> {
        if length > self.max_associated_data {
            Err(EnvelopeError::new(EnvelopeErrorCode::Limit))
        } else {
            Ok(())
        }
    }

    /// Return the largest envelope length permitted under [`Self::max_plaintext`].
    pub fn max_envelope(self) -> Result<usize, EnvelopeError> {
        self.max_plaintext
            .checked_add(OVERHEAD)
            .ok_or_else(|| EnvelopeError::new(EnvelopeErrorCode::Limit))
    }
}

/// Stable machine-readable failure category for [`EnvelopeError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum EnvelopeErrorCode {
    /// The key is not exactly 32 bytes.
    Key,
    /// The envelope is truncated or names an unknown format.
    Format,
    /// The tag did not verify under the supplied key and associated data.
    Authentication,
    /// The operating system CSPRNG failed.
    Random,
    /// An input exceeded a documented bound or an allocation was refused.
    Limit,
}

impl EnvelopeErrorCode {
    /// Stable `VAEL_CRYPTO_*` code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Key => "VAEL_CRYPTO_KEY",
            Self::Format => "VAEL_CRYPTO_FORMAT",
            Self::Authentication => "VAEL_CRYPTO_AUTH",
            Self::Random => "VAEL_CRYPTO_RANDOM",
            Self::Limit => "VAEL_CRYPTO_LIMIT",
        }
    }

    /// Fixed human-readable message for this failure category.
    #[must_use]
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Key => "key must be exactly 32 bytes",
            Self::Format => "malformed or unsupported crypto envelope",
            Self::Authentication => "authentication failed",
            Self::Random => "operating system random number generator failed",
            Self::Limit => "input exceeds the crypto size limit",
        }
    }
}

impl fmt::Display for EnvelopeErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure returned by envelope encryption, decryption, and CSPRNG calls.
///
/// Messages are fixed per [`EnvelopeErrorCode`] and never echo keys,
/// plaintexts, ciphertexts, or entropy bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{}", .code.default_message())]
#[non_exhaustive]
pub struct EnvelopeError {
    code: EnvelopeErrorCode,
}

impl EnvelopeError {
    /// Create an error for `code`.
    #[must_use]
    pub const fn new(code: EnvelopeErrorCode) -> Self {
        Self { code }
    }

    /// The stable failure category.
    #[must_use]
    pub const fn code(self) -> EnvelopeErrorCode {
        self.code
    }

    /// Fixed message safe to expose to callers.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.code.default_message()
    }
}

fn aad_capacity(associated_data_len: usize) -> Result<usize, EnvelopeError> {
    HEADER_LEN
        .checked_add(NONCE_LEN)
        .and_then(|fixed| fixed.checked_add(associated_data_len))
        .ok_or_else(|| EnvelopeError::new(EnvelopeErrorCode::Limit))
}

fn cipher(key: &[u8]) -> Result<Aes256Gcm, EnvelopeError> {
    if key.len() != KEY_LEN {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Key));
    }
    Aes256Gcm::new_from_slice(key).map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Key))
}

#[cfg(test)]
fn seal_detached(
    key: &[u8],
    nonce: &[u8],
    associated_data: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; TAG_LEN]), EnvelopeError> {
    if nonce.len() != NONCE_LEN {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Format));
    }
    let cipher = cipher(key)?;
    let mut buffer = plaintext.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), associated_data, &mut buffer)
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Limit))?;
    let mut tag_bytes = [0u8; TAG_LEN];
    tag_bytes.copy_from_slice(tag.as_slice());
    Ok((buffer, tag_bytes))
}

#[cfg(test)]
fn open_detached(
    key: &[u8],
    nonce: &[u8],
    associated_data: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, EnvelopeError> {
    if nonce.len() != NONCE_LEN || tag.len() != TAG_LEN {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Format));
    }
    let cipher = cipher(key)?;
    let mut buffer = ciphertext.to_vec();
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(nonce),
            associated_data,
            &mut buffer,
            Tag::from_slice(tag),
        )
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Authentication))?;
    Ok(buffer)
}

/// Encrypt `plaintext` into a fresh versioned NEO1 envelope using a random
/// 96-bit nonce from the operating system CSPRNG.
///
/// `key` must be exactly 32 bytes. `plaintext` and `associated_data` are
/// checked against `limits` before allocation. Fails with
/// [`EnvelopeErrorCode::Key`], [`EnvelopeErrorCode::Limit`], or
/// [`EnvelopeErrorCode::Random`].
///
/// # Example
///
/// ```
/// use vaeljs_crypto_core::envelope::{CryptoLimits, decrypt, encrypt, generate_envelope_key};
///
/// let key = generate_envelope_key()?;
/// let envelope = encrypt(&key, b"secret payload".to_vec(), b"aad", CryptoLimits::default())?;
/// let plaintext = decrypt(&key, envelope, b"aad", CryptoLimits::default())?;
/// assert_eq!(plaintext, b"secret payload");
/// # Ok::<(), vaeljs_crypto_core::envelope::EnvelopeError>(())
/// ```
pub fn encrypt(
    key: &[u8],
    plaintext: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
) -> Result<Vec<u8>, EnvelopeError> {
    let plaintext = Zeroizing::new(plaintext);
    if key.len() != KEY_LEN {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Key));
    }
    limits.check_plaintext(plaintext.len())?;
    limits.check_associated_data(associated_data.len())?;

    let mut nonce = [0u8; NONCE_LEN];
    fill_random(&mut nonce)?;
    encrypt_with_nonce(key, plaintext, associated_data, limits, nonce)
}

fn encrypt_with_nonce(
    key: &[u8],
    mut plaintext: Zeroizing<Vec<u8>>,
    associated_data: &[u8],
    limits: CryptoLimits,
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, EnvelopeError> {
    if key.len() != KEY_LEN {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Key));
    }
    limits.check_plaintext(plaintext.len())?;
    limits.check_associated_data(associated_data.len())?;

    let cipher = cipher(key)?;
    let envelope_capacity = OVERHEAD
        .checked_add(plaintext.len())
        .ok_or_else(|| EnvelopeError::new(EnvelopeErrorCode::Limit))?;
    let mut envelope = Vec::new();
    envelope
        .try_reserve_exact(envelope_capacity)
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Limit))?;
    envelope.extend_from_slice(MAGIC);
    envelope.push(VERSION);
    envelope.push(ALGORITHM_AES_256_GCM);
    envelope.extend_from_slice(&nonce);

    let mut aad = Vec::new();
    aad.try_reserve_exact(aad_capacity(associated_data.len())?)
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Limit))?;
    aad.extend_from_slice(&envelope);
    aad.extend_from_slice(associated_data);

    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut plaintext)
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Limit))?;

    envelope.extend_from_slice(&plaintext);
    envelope.extend_from_slice(tag.as_slice());
    Ok(envelope)
}

/// Decrypt and authenticate one versioned NEO1 envelope.
///
/// `key` must be exactly 32 bytes. On success, reuses `envelope`'s buffer for
/// the plaintext. Fails with [`EnvelopeErrorCode::Key`],
/// [`EnvelopeErrorCode::Format`], [`EnvelopeErrorCode::Limit`], or
/// [`EnvelopeErrorCode::Authentication`], never returning unauthenticated bytes.
pub fn decrypt(
    key: &[u8],
    mut envelope: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
) -> Result<Vec<u8>, EnvelopeError> {
    if key.len() != KEY_LEN {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Key));
    }
    if envelope.len() < OVERHEAD {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Format));
    }
    if envelope.len() > limits.max_envelope()? {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Limit));
    }
    limits.check_associated_data(associated_data.len())?;
    if &envelope[..MAGIC.len()] != MAGIC {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Format));
    }
    if envelope[MAGIC.len()] != VERSION {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Format));
    }
    if envelope[MAGIC.len() + 1] != ALGORITHM_AES_256_GCM {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Format));
    }

    let ciphertext_len = envelope.len() - OVERHEAD;
    if ciphertext_len > limits.max_plaintext {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Limit));
    }
    let cipher = cipher(key)?;
    let mut aad = Vec::new();
    aad.try_reserve_exact(aad_capacity(associated_data.len())?)
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Limit))?;
    aad.extend_from_slice(&envelope[..HEADER_LEN + NONCE_LEN]);
    aad.extend_from_slice(associated_data);

    let tag_start = envelope.len() - TAG_LEN;
    let (body, tag) = envelope.split_at_mut(tag_start);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&body[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
    let ciphertext = &mut body[HEADER_LEN + NONCE_LEN..];
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            &aad,
            ciphertext,
            Tag::from_slice(tag),
        )
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Authentication))?;

    envelope.drain(..HEADER_LEN + NONCE_LEN);
    envelope.truncate(ciphertext_len);
    Ok(envelope)
}

fn fill_random(bytes: &mut [u8]) -> Result<(), EnvelopeError> {
    getrandom::fill(bytes).map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Random))
}

/// Return 32 random bytes from the operating system CSPRNG.
///
/// Fails with [`EnvelopeErrorCode::Random`] if OS entropy is unavailable.
pub fn generate_envelope_key() -> Result<[u8; KEY_LEN], EnvelopeError> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    fill_random(&mut *key)?;
    Ok(*key)
}

/// Return `length` random bytes from the operating system CSPRNG.
///
/// Rejects lengths above [`MAX_RANDOM_BYTES`] with [`EnvelopeErrorCode::Limit`]
/// before allocating, uses fallible reservation, and maps CSPRNG errors to
/// [`EnvelopeErrorCode::Random`].
pub fn random_bytes(length: usize) -> Result<Vec<u8>, EnvelopeError> {
    if length > MAX_RANDOM_BYTES {
        return Err(EnvelopeError::new(EnvelopeErrorCode::Limit));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    bytes
        .try_reserve_exact(length)
        .map_err(|_| EnvelopeError::new(EnvelopeErrorCode::Limit))?;
    bytes.resize(length, 0);
    fill_random(&mut bytes)?;
    Ok(std::mem::take(&mut *bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "hex length");
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    fn encode_hex(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            text.push_str(&format!("{byte:02x}"));
        }
        text
    }

    fn default_limits() -> CryptoLimits {
        CryptoLimits::default()
    }

    struct Vector {
        name: &'static str,
        key: &'static str,
        nonce: &'static str,
        aad: &'static str,
        plaintext: &'static str,
        ciphertext: &'static str,
        tag: &'static str,
    }

    const VECTORS: [Vector; 4] = [
        Vector {
            name: "case 13 (empty)",
            key: "0000000000000000000000000000000000000000000000000000000000000000",
            nonce: "000000000000000000000000",
            aad: "",
            plaintext: "",
            ciphertext: "",
            tag: "530f8afbc74536b9a963b4f1c4cb738b",
        },
        Vector {
            name: "case 14 (one zero block)",
            key: "0000000000000000000000000000000000000000000000000000000000000000",
            nonce: "000000000000000000000000",
            aad: "",
            plaintext: "00000000000000000000000000000000",
            ciphertext: "cea7403d4d606b6e074ec5d3baf39d18",
            tag: "d0d1c8a799996bf0265b98b5d48ab919",
        },
        Vector {
            name: "case 15 (four blocks)",
            key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
            nonce: "cafebabefacedbaddecaf888",
            aad: "",
            plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
            ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015ad",
            tag: "b094dac5d93471bdec1a502270e3cc6c",
        },
        Vector {
            name: "case 16 (four blocks with AAD)",
            key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
            nonce: "cafebabefacedbaddecaf888",
            aad: "feedfacedeadbeeffeedfacedeadbeefabaddad2",
            plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
            ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662",
            tag: "76fc6ece0f4e1768cddf8853bb2d551b",
        },
    ];

    // Unit test: published AES-256-GCM test vectors (McGrew/Viega Appendix B).
    #[test]
    fn published_aes_256_gcm_vectors_match() {
        for vector in &VECTORS {
            let key = decode_hex(vector.key);
            let nonce = decode_hex(vector.nonce);
            let aad = decode_hex(vector.aad);
            let plaintext = decode_hex(vector.plaintext);
            let (ciphertext, tag) = seal_detached(&key, &nonce, &aad, &plaintext).unwrap();
            assert_eq!(
                encode_hex(&ciphertext),
                vector.ciphertext,
                "{}",
                vector.name
            );
            assert_eq!(encode_hex(&tag), vector.tag, "{}", vector.name);
            assert_eq!(
                open_detached(&key, &nonce, &aad, &ciphertext, &tag).unwrap(),
                plaintext,
                "{}",
                vector.name
            );
        }
    }

    const INTEROP_KEY: &str = "8f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
    const INTEROP_AAD: &str = "00010203040506070809";
    const INTEROP_PLAINTEXT: &str =
        "6e656f6a732063727970746f2063726f73732d696d706c656d656e746174696f6e2066697874757265";
    const INTEROP_ENVELOPE: &str = "4e454f310101a1b2c3d4e5f60718293a4b5c6da183558bece799386fca37bf3f217b09605d495c551d38c28e68bd5cbdd804db19b8ed0a913ffff9b52d67946d74dd47c5b93bdec06d68df";

    const REFERENCE_ENVELOPE: &str = "4e454f310101000102030405060708090a0b2967b921a697bb6bf92eb7e2df9d1d1feca6a75299032b094a0230ed39f87f14b520fd7a392dc47ab999";
    const REFERENCE_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const REFERENCE_AAD: &str = "636f6e746578742d6279746573";

    // Unit test: codec exact-bytes interop fixture check.
    #[test]
    fn independent_envelope_fixtures_decrypt() {
        let key = decode_hex(REFERENCE_KEY);
        let envelope = decode_hex(REFERENCE_ENVELOPE);
        let aad = decode_hex(REFERENCE_AAD);
        assert_eq!(
            decrypt(&key, envelope, &aad, default_limits()).unwrap(),
            b"neo:crypto interop fixture"
        );

        let key = decode_hex(INTEROP_KEY);
        let aad = decode_hex(INTEROP_AAD);
        let envelope = decode_hex(INTEROP_ENVELOPE);
        assert_eq!(
            decrypt(&key, envelope, &aad, default_limits()).unwrap(),
            decode_hex(INTEROP_PLAINTEXT)
        );
    }

    fn nonce_from_envelope(envelope: &[u8]) -> [u8; NONCE_LEN] {
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        nonce
    }

    // Unit test: private deterministic-nonce path not reachable through public API.
    #[test]
    fn envelope_encryption_matches_independent_fixtures_byte_for_byte() {
        let key = decode_hex(INTEROP_KEY);
        let aad = decode_hex(INTEROP_AAD);
        let plaintext = decode_hex(INTEROP_PLAINTEXT);
        let expected = decode_hex(INTEROP_ENVELOPE);
        let emitted = encrypt_with_nonce(
            &key,
            Zeroizing::new(plaintext),
            &aad,
            default_limits(),
            nonce_from_envelope(&expected),
        )
        .unwrap();
        assert_eq!(encode_hex(&emitted), INTEROP_ENVELOPE);

        let key = decode_hex(REFERENCE_KEY);
        let aad = decode_hex(REFERENCE_AAD);
        let expected = decode_hex(REFERENCE_ENVELOPE);
        let emitted = encrypt_with_nonce(
            &key,
            Zeroizing::new(b"neo:crypto interop fixture".to_vec()),
            &aad,
            default_limits(),
            nonce_from_envelope(&expected),
        )
        .unwrap();
        assert_eq!(emitted, expected);
    }

    // Unit test: NEO1 wire layout and codec assertions.
    #[test]
    fn envelope_has_the_documented_layout() {
        let key = [7u8; KEY_LEN];
        let plaintext = b"layout".to_vec();
        let envelope = encrypt(&key, plaintext.clone(), b"", default_limits()).unwrap();
        assert_eq!(&envelope[..4], b"NEO1");
        assert_eq!(envelope[4], VERSION);
        assert_eq!(envelope[5], ALGORITHM_AES_256_GCM);
        assert_eq!(envelope.len(), OVERHEAD + plaintext.len());
        let ciphertext = &envelope[HEADER_LEN + NONCE_LEN..envelope.len() - TAG_LEN];
        assert_eq!(ciphertext.len(), plaintext.len());
        assert_ne!(ciphertext, plaintext.as_slice());
        assert_ne!(
            &envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN],
            &[0u8; NONCE_LEN]
        );
        assert_eq!(
            decrypt(&key, envelope, b"", default_limits()).unwrap(),
            plaintext
        );
    }

    // Unit test: codec roundtrip for empty, binary, and UTF-8 inputs.
    #[test]
    fn roundtrips_empty_binary_and_unicode_payloads() {
        let key = [0x5au8; KEY_LEN];
        let unicode = "Grüße, 世界, 🦀".as_bytes();
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        for (plaintext, aad) in [
            (&b""[..], &b""[..]),
            (unicode, "context: ünïcode".as_bytes()),
            (&all_bytes, &[0u8, 1, 2, 0xff]),
        ] {
            let envelope = encrypt(&key, plaintext.to_vec(), aad, default_limits()).unwrap();
            assert_eq!(
                decrypt(&key, envelope, aad, default_limits()).unwrap(),
                plaintext
            );
        }
    }

    // Unit test: codec integrity check rejecting every single bit flip.
    #[test]
    fn every_single_bit_flip_is_rejected() {
        let key = [0x11u8; KEY_LEN];
        let envelope = encrypt(
            &key,
            b"\x00\xffbinary".to_vec(),
            b"associated",
            default_limits(),
        )
        .unwrap();
        for byte in 0..envelope.len() {
            for bit in 0..8 {
                let mut tampered = envelope.clone();
                tampered[byte] ^= 1 << bit;
                let error = decrypt(&key, tampered, b"associated", default_limits())
                    .expect_err("a changed envelope must fail");
                assert!(
                    matches!(
                        error.code(),
                        EnvelopeErrorCode::Authentication | EnvelopeErrorCode::Format
                    ),
                    "byte {byte} bit {bit} produced {:?}",
                    error.code()
                );
            }
        }
    }

    // Unit test: codec authentication failure on wrong key or AAD.
    #[test]
    fn wrong_key_and_wrong_associated_data_are_rejected() {
        let key = [0x22u8; KEY_LEN];
        let envelope = encrypt(&key, b"secret".to_vec(), b"context", default_limits()).unwrap();
        assert_eq!(
            decrypt(
                &[0x23u8; KEY_LEN],
                envelope.clone(),
                b"context",
                default_limits()
            )
            .unwrap_err()
            .code(),
            EnvelopeErrorCode::Authentication
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"other", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Authentication
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Authentication
        );
        assert_eq!(
            decrypt(&key, envelope, b"context", default_limits()).unwrap(),
            b"secret"
        );
    }

    // Unit test: boundary check for 32-byte key length.
    #[test]
    fn key_length_is_enforced() {
        for length in [0usize, 1, 16, 31, 33, 64] {
            let key = vec![0x33u8; length];
            assert_eq!(
                encrypt(&key, b"x".to_vec(), b"", default_limits())
                    .unwrap_err()
                    .code(),
                EnvelopeErrorCode::Key
            );
            assert_eq!(
                decrypt(&key, vec![0u8; OVERHEAD], b"", default_limits())
                    .unwrap_err()
                    .code(),
                EnvelopeErrorCode::Key
            );
        }
        assert!(encrypt(&[0x33u8; KEY_LEN], b"x".to_vec(), b"", default_limits()).is_ok());
    }

    // Unit test: codec rejection of malformed headers and tags.
    #[test]
    fn malformed_envelopes_are_rejected() {
        let key = [0x44u8; KEY_LEN];
        assert_eq!(
            decrypt(&key, Vec::new(), b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Format
        );
        assert_eq!(
            decrypt(&key, vec![0u8; OVERHEAD - 1], b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Format
        );

        let envelope = encrypt(&key, b"payload".to_vec(), b"", default_limits()).unwrap();
        let mut bad_magic = envelope.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            decrypt(&key, bad_magic, b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Format
        );

        let mut bad_version = envelope.clone();
        bad_version[4] = VERSION + 1;
        assert_eq!(
            decrypt(&key, bad_version, b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Format
        );

        let mut bad_algorithm = envelope.clone();
        bad_algorithm[5] = 2;
        assert_eq!(
            decrypt(&key, bad_algorithm, b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Format
        );

        let mut bad_tag = envelope;
        let last = bad_tag.len() - 1;
        bad_tag[last] ^= 1;
        assert_eq!(
            decrypt(&key, bad_tag, b"", default_limits())
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Authentication
        );
    }

    // Unit test: bounds check at exact limit and +1 over limit.
    #[test]
    fn small_limits_are_checked_at_the_boundary() {
        let limits = CryptoLimits {
            max_plaintext: 8,
            max_associated_data: 5,
        };
        let key = [0x77u8; KEY_LEN];

        let envelope = encrypt(&key, vec![0u8; 8], b"12345", limits).unwrap();
        assert_eq!(envelope.len(), OVERHEAD + 8);
        assert!(decrypt(&key, envelope.clone(), b"12345", limits).is_ok());

        assert_eq!(
            encrypt(&key, vec![0u8; 9], b"", limits).unwrap_err().code(),
            EnvelopeErrorCode::Limit
        );
        assert_eq!(
            encrypt(&key, vec![0u8; 1], b"123456", limits)
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Limit
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"123456", limits)
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Limit
        );

        let mut oversized = envelope;
        oversized.push(0);
        assert_eq!(
            decrypt(&key, oversized, b"12345", limits)
                .unwrap_err()
                .code(),
            EnvelopeErrorCode::Limit
        );
    }

    // Unit test: bounds check for random_bytes and generate_envelope_key.
    #[test]
    fn random_bytes_is_bounded_and_filled() {
        assert!(random_bytes(0).unwrap().is_empty());

        let first = random_bytes(32).unwrap();
        let second = random_bytes(32).unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(second.len(), 32);
        assert_ne!(first, second);

        assert_eq!(
            random_bytes(MAX_RANDOM_BYTES + 1).unwrap_err().code(),
            EnvelopeErrorCode::Limit
        );
        assert_eq!(
            random_bytes(MAX_RANDOM_BYTES).unwrap().len(),
            MAX_RANDOM_BYTES
        );

        let key = generate_envelope_key().unwrap();
        assert_eq!(key.len(), KEY_LEN);
        assert_ne!(key, [0u8; KEY_LEN]);
    }

    // Unit test: exact error code strings and message invariants.
    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        assert_eq!(
            EnvelopeError::new(EnvelopeErrorCode::Key).code().as_str(),
            "VAEL_CRYPTO_KEY"
        );
        assert_eq!(
            EnvelopeError::new(EnvelopeErrorCode::Format)
                .code()
                .as_str(),
            "VAEL_CRYPTO_FORMAT"
        );
        assert_eq!(
            EnvelopeError::new(EnvelopeErrorCode::Authentication)
                .code()
                .as_str(),
            "VAEL_CRYPTO_AUTH"
        );
        assert_eq!(
            EnvelopeError::new(EnvelopeErrorCode::Random)
                .code()
                .as_str(),
            "VAEL_CRYPTO_RANDOM"
        );
        assert_eq!(
            EnvelopeError::new(EnvelopeErrorCode::Limit).code().as_str(),
            "VAEL_CRYPTO_LIMIT"
        );
        for code in [
            EnvelopeErrorCode::Key,
            EnvelopeErrorCode::Format,
            EnvelopeErrorCode::Authentication,
            EnvelopeErrorCode::Random,
            EnvelopeErrorCode::Limit,
        ] {
            let err = EnvelopeError::new(code);
            let message = err.message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
            assert_eq!(err.to_string(), message);
        }
    }
}
