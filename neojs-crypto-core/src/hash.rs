//! Incremental SHA-2 hashing, HMAC, output encodings, and one-shot helpers.
//!
//! Supported algorithms are `sha256`, `sha384`, and `sha512` (case-insensitive).
//! Output encodings are `binary` (raw digest bytes), `hex`, `base64`,
//! `base64url` (unpadded), and `latin1`.

use std::collections::HashMap;
use std::fmt;

use hmac::Hmac;
use hmac::KeyInit;
use hmac::Mac;
use sha2::Digest;
use sha2::Sha256;
use sha2::Sha384;
use sha2::Sha512;
use thiserror::Error;

/// Largest input accepted by one hash or HMAC object or one-shot call.
pub const MAX_HASH_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// Largest HMAC key accepted, in bytes.
pub const MAX_HMAC_KEY_BYTES: usize = 1024 * 1024;

/// Largest number of incremental hash and HMAC objects open at once in a
/// [`DigestTable`].
pub const MAX_OPEN_DIGESTS: usize = 1024;

/// Supported SHA-2 algorithms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum HashAlgorithm {
    /// SHA-256 (32-byte digest).
    Sha256,
    /// SHA-384 (48-byte digest).
    Sha384,
    /// SHA-512 (64-byte digest).
    Sha512,
}

impl HashAlgorithm {
    /// Parse one of `sha256`, `sha384`, or `sha512` case-insensitively.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("sha256") {
            Some(Self::Sha256)
        } else if name.eq_ignore_ascii_case("sha384") {
            Some(Self::Sha384)
        } else if name.eq_ignore_ascii_case("sha512") {
            Some(Self::Sha512)
        } else {
            None
        }
    }

    /// Digest length in bytes for this algorithm.
    #[must_use]
    pub const fn digest_len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }
}

/// Supported digest output encodings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum OutputEncoding {
    /// Raw digest bytes.
    Binary,
    /// Lowercase hexadecimal ASCII bytes.
    Hex,
    /// Standard Base64 ASCII bytes with padding.
    Base64,
    /// URL-safe Base64 ASCII bytes without padding.
    Base64Url,
    /// Raw bytes for Latin-1 (one code unit per byte in JS).
    Latin1,
}

impl OutputEncoding {
    /// Decode the numeric encoding code used by host ops.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Binary,
            1 => Self::Hex,
            2 => Self::Base64,
            3 => Self::Base64Url,
            4 => Self::Latin1,
            _ => return None,
        })
    }

    /// Parse a string encoding name (`hex`, `base64`, `base64url`, `latin1`,
    /// or `binary`) case-insensitively.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("hex") {
            Some(Self::Hex)
        } else if name.eq_ignore_ascii_case("base64") {
            Some(Self::Base64)
        } else if name.eq_ignore_ascii_case("base64url") {
            Some(Self::Base64Url)
        } else if name.eq_ignore_ascii_case("latin1") || name.eq_ignore_ascii_case("binary") {
            Some(Self::Latin1)
        } else {
            None
        }
    }
}

/// Stable machine-readable failure category for [`HashError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum HashErrorCode {
    /// The algorithm name is not one of the three supported SHA-2 names.
    Algorithm,
    /// The input or output encoding is unsupported.
    Encoding,
    /// An input or key exceeded a documented bound.
    Limit,
    /// `digest` was already called on this object.
    Digested,
    /// Too many incremental hash objects are open.
    TooManyObjects,
    /// The incremental hash object was closed.
    Closed,
    /// An internal hash invariant failed.
    Internal,
}

impl HashErrorCode {
    /// Stable `NEO_CRYPTO_*` code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Algorithm => "NEO_CRYPTO_ALGORITHM",
            Self::Encoding => "NEO_CRYPTO_ENCODING",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Digested => "NEO_CRYPTO_DIGESTED",
            Self::TooManyObjects => "NEO_CRYPTO_TOO_MANY_OBJECTS",
            Self::Closed => "NEO_CRYPTO_CLOSED",
            Self::Internal => "NEO_CRYPTO_INTERNAL",
        }
    }

    /// Fixed human-readable message for this failure category.
    #[must_use]
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Algorithm => "unsupported hash algorithm",
            Self::Encoding => "unsupported input encoding",
            Self::Limit => "input exceeds the hash size limit",
            Self::Digested => "digest has already been called",
            Self::TooManyObjects => "too many incremental hash objects are open",
            Self::Closed => "the hash object is closed",
            Self::Internal => "the hash operation failed internally",
        }
    }
}

impl fmt::Display for HashErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure returned by hashing and HMAC operations.
///
/// Messages are fixed per [`HashErrorCode`] and never echo inputs or keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{}", .code.default_message())]
#[non_exhaustive]
pub struct HashError {
    code: HashErrorCode,
}

impl HashError {
    /// Create an error for `code`.
    #[must_use]
    pub const fn new(code: HashErrorCode) -> Self {
        Self { code }
    }

    /// The stable failure category.
    #[must_use]
    pub const fn code(self) -> HashErrorCode {
        self.code
    }

    /// Fixed message safe to expose to callers.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.code.default_message()
    }
}

enum HashInner {
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
}

/// Incremental bounded SHA-2 hasher.
///
/// # Example
///
/// ```
/// use neojs_crypto_core::hash::{HashAlgorithm, Hasher, OutputEncoding};
///
/// let mut hasher = Hasher::new(HashAlgorithm::Sha256);
/// hasher.update(b"abc")?;
/// let hex = hasher.finalize_encoded(OutputEncoding::Hex);
/// assert_eq!(
///     hex,
///     b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
/// );
/// # Ok::<(), neojs_crypto_core::hash::HashError>(())
/// ```
pub struct Hasher {
    total: usize,
    inner: HashInner,
}

impl Hasher {
    /// Create an empty incremental hasher for `algorithm`.
    #[must_use]
    pub fn new(algorithm: HashAlgorithm) -> Self {
        let inner = match algorithm {
            HashAlgorithm::Sha256 => HashInner::Sha256(Sha256::new()),
            HashAlgorithm::Sha384 => HashInner::Sha384(Sha384::new()),
            HashAlgorithm::Sha512 => HashInner::Sha512(Sha512::new()),
        };
        Self { total: 0, inner }
    }

    /// Remaining input bytes accepted before hitting [`MAX_HASH_INPUT_BYTES`].
    #[must_use]
    pub const fn remaining(&self) -> usize {
        MAX_HASH_INPUT_BYTES.saturating_sub(self.total)
    }

    /// Feed `data` into the hasher, enforcing [`MAX_HASH_INPUT_BYTES`].
    pub fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        let next = self
            .total
            .checked_add(data.len())
            .ok_or_else(|| HashError::new(HashErrorCode::Limit))?;
        if next > MAX_HASH_INPUT_BYTES {
            return Err(HashError::new(HashErrorCode::Limit));
        }
        match &mut self.inner {
            HashInner::Sha256(h) => h.update(data),
            HashInner::Sha384(h) => h.update(data),
            HashInner::Sha512(h) => h.update(data),
        }
        self.total = next;
        Ok(())
    }

    /// Finalize and return raw digest bytes.
    #[must_use]
    pub fn finalize(self) -> Vec<u8> {
        match self.inner {
            HashInner::Sha256(h) => h.finalize().to_vec(),
            HashInner::Sha384(h) => h.finalize().to_vec(),
            HashInner::Sha512(h) => h.finalize().to_vec(),
        }
    }

    /// Finalize and encode the digest with `encoding`.
    #[must_use]
    pub fn finalize_encoded(self, encoding: OutputEncoding) -> Vec<u8> {
        encode_digest(&self.finalize(), encoding)
    }
}

enum HmacInner {
    Sha256(Hmac<Sha256>),
    Sha384(Hmac<Sha384>),
    Sha512(Hmac<Sha512>),
}

/// Incremental bounded HMAC-SHA2 hasher.
///
/// # Example
///
/// ```
/// use neojs_crypto_core::hash::{HashAlgorithm, HmacHasher, OutputEncoding};
///
/// let mut mac = HmacHasher::new(HashAlgorithm::Sha256, b"Jefe")?;
/// mac.update(b"what do ya want for nothing?")?;
/// assert_eq!(
///     mac.finalize_encoded(OutputEncoding::Hex),
///     b"5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
/// );
/// # Ok::<(), neojs_crypto_core::hash::HashError>(())
/// ```
pub struct HmacHasher {
    total: usize,
    inner: HmacInner,
}

impl HmacHasher {
    /// Create a keyed HMAC hasher, enforcing [`MAX_HMAC_KEY_BYTES`].
    pub fn new(algorithm: HashAlgorithm, key: &[u8]) -> Result<Self, HashError> {
        if key.len() > MAX_HMAC_KEY_BYTES {
            return Err(HashError::new(HashErrorCode::Limit));
        }
        let failed = || HashError::new(HashErrorCode::Internal);
        let inner = match algorithm {
            HashAlgorithm::Sha256 => {
                HmacInner::Sha256(Hmac::<Sha256>::new_from_slice(key).map_err(|_| failed())?)
            }
            HashAlgorithm::Sha384 => {
                HmacInner::Sha384(Hmac::<Sha384>::new_from_slice(key).map_err(|_| failed())?)
            }
            HashAlgorithm::Sha512 => {
                HmacInner::Sha512(Hmac::<Sha512>::new_from_slice(key).map_err(|_| failed())?)
            }
        };
        Ok(Self { total: 0, inner })
    }

    /// Remaining input bytes accepted before hitting [`MAX_HASH_INPUT_BYTES`].
    #[must_use]
    pub const fn remaining(&self) -> usize {
        MAX_HASH_INPUT_BYTES.saturating_sub(self.total)
    }

    /// Feed `data` into the HMAC state, enforcing [`MAX_HASH_INPUT_BYTES`].
    pub fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        let next = self
            .total
            .checked_add(data.len())
            .ok_or_else(|| HashError::new(HashErrorCode::Limit))?;
        if next > MAX_HASH_INPUT_BYTES {
            return Err(HashError::new(HashErrorCode::Limit));
        }
        match &mut self.inner {
            HmacInner::Sha256(m) => m.update(data),
            HmacInner::Sha384(m) => m.update(data),
            HmacInner::Sha512(m) => m.update(data),
        }
        self.total = next;
        Ok(())
    }

    /// Finalize and return raw MAC bytes.
    #[must_use]
    pub fn finalize(self) -> Vec<u8> {
        match self.inner {
            HmacInner::Sha256(m) => m.finalize().into_bytes().to_vec(),
            HmacInner::Sha384(m) => m.finalize().into_bytes().to_vec(),
            HmacInner::Sha512(m) => m.finalize().into_bytes().to_vec(),
        }
    }

    /// Finalize and encode the MAC with `encoding`.
    #[must_use]
    pub fn finalize_encoded(self, encoding: OutputEncoding) -> Vec<u8> {
        encode_digest(&self.finalize(), encoding)
    }
}

/// Unified incremental hash or HMAC state.
pub enum DigestState {
    /// Incremental SHA-2 hash.
    Hash(Hasher),
    /// Incremental HMAC-SHA2.
    Hmac(HmacHasher),
}

impl DigestState {
    /// Create a hash state for `algorithm`.
    #[must_use]
    pub fn hash(algorithm: HashAlgorithm) -> Self {
        Self::Hash(Hasher::new(algorithm))
    }

    /// Create an HMAC state for `algorithm` and `key`.
    pub fn hmac(algorithm: HashAlgorithm, key: &[u8]) -> Result<Self, HashError> {
        Ok(Self::Hmac(HmacHasher::new(algorithm, key)?))
    }

    /// Remaining input capacity in bytes.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        match self {
            Self::Hash(h) => h.remaining(),
            Self::Hmac(m) => m.remaining(),
        }
    }

    /// Feed `data` into the digest state.
    pub fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        match self {
            Self::Hash(h) => h.update(data),
            Self::Hmac(m) => m.update(data),
        }
    }

    /// Consume the state and return raw digest bytes.
    #[must_use]
    pub fn finalize(self) -> Vec<u8> {
        match self {
            Self::Hash(h) => h.finalize(),
            Self::Hmac(m) => m.finalize(),
        }
    }
}

/// Engine-neutral bounded table of open [`DigestState`] handles.
pub struct DigestTable {
    max_open: usize,
    entries: HashMap<u32, Option<DigestState>>,
    next_id: u32,
}

impl Default for DigestTable {
    fn default() -> Self {
        Self::with_limit(MAX_OPEN_DIGESTS)
    }
}

impl DigestTable {
    /// Create a table capping open handles at `max_open`.
    #[must_use]
    pub fn with_limit(max_open: usize) -> Self {
        Self {
            max_open,
            entries: HashMap::new(),
            next_id: 1,
        }
    }

    /// Number of active entries currently in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn allocate_id(&mut self) -> Result<u32, HashError> {
        if self.entries.len() >= self.max_open {
            return Err(HashError::new(HashErrorCode::TooManyObjects));
        }
        let start = self.next_id;
        loop {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 {
                self.next_id = 1;
            }
            if candidate != 0 && !self.entries.contains_key(&candidate) {
                return Ok(candidate);
            }
            if self.next_id == start {
                return Err(HashError::new(HashErrorCode::TooManyObjects));
            }
        }
    }

    /// Open a new incremental hash handle.
    pub fn create_hash(&mut self, algorithm: HashAlgorithm) -> Result<u32, HashError> {
        let id = self.allocate_id()?;
        self.entries.insert(id, Some(DigestState::hash(algorithm)));
        Ok(id)
    }

    /// Open a new incremental HMAC handle.
    pub fn create_hmac(&mut self, algorithm: HashAlgorithm, key: &[u8]) -> Result<u32, HashError> {
        if self.entries.len() >= self.max_open {
            return Err(HashError::new(HashErrorCode::TooManyObjects));
        }
        let state = DigestState::hmac(algorithm, key)?;
        let id = self.allocate_id()?;
        self.entries.insert(id, Some(state));
        Ok(id)
    }

    /// Remaining bytes permitted for handle `id`.
    pub fn remaining(&self, id: u32) -> Result<usize, HashError> {
        let slot = self
            .entries
            .get(&id)
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        let state = slot
            .as_ref()
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        Ok(state.remaining())
    }

    /// Append `data` to handle `id`.
    pub fn update(&mut self, id: u32, data: &[u8]) -> Result<(), HashError> {
        let slot = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        let state = slot
            .as_mut()
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        state.update(data)
    }

    /// Consume handle `id` and return its encoded digest.
    pub fn digest(&mut self, id: u32, encoding: OutputEncoding) -> Result<Vec<u8>, HashError> {
        let slot = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        let state = slot
            .take()
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        self.entries.remove(&id);
        Ok(encode_digest(&state.finalize(), encoding))
    }

    /// Close and release handle `id` without producing a digest.
    pub fn close(&mut self, id: u32) {
        self.entries.remove(&id);
    }
}

/// Compute a single-shot SHA-2 digest over `data`.
pub fn digest(
    algorithm: HashAlgorithm,
    data: &[u8],
    encoding: OutputEncoding,
) -> Result<Vec<u8>, HashError> {
    let mut hasher = Hasher::new(algorithm);
    hasher.update(data)?;
    Ok(hasher.finalize_encoded(encoding))
}

/// Compute a single-shot HMAC-SHA2 over `key` and `data`.
pub fn hmac(
    algorithm: HashAlgorithm,
    key: &[u8],
    data: &[u8],
    encoding: OutputEncoding,
) -> Result<Vec<u8>, HashError> {
    let mut mac = HmacHasher::new(algorithm, key)?;
    mac.update(data)?;
    Ok(mac.finalize_encoded(encoding))
}

/// Encode raw digest bytes into the requested [`OutputEncoding`].
#[must_use]
pub fn encode_digest(digest: &[u8], encoding: OutputEncoding) -> Vec<u8> {
    match encoding {
        OutputEncoding::Binary | OutputEncoding::Latin1 => digest.to_vec(),
        OutputEncoding::Hex => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut text = Vec::with_capacity(digest.len() * 2);
            for byte in digest {
                text.push(HEX[(byte >> 4) as usize]);
                text.push(HEX[(byte & 0x0f) as usize]);
            }
            text
        }
        OutputEncoding::Base64 => base64_encode(digest, false),
        OutputEncoding::Base64Url => base64_encode(digest, true),
    }
}

/// Encode `input` as standard padded Base64 or unpadded URL-safe Base64.
#[must_use]
pub fn base64_encode(input: &[u8], url_safe: bool) -> Vec<u8> {
    const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let alphabet = if url_safe { URL_SAFE } else { STANDARD };
    let mut output = Vec::with_capacity(input.len().div_ceil(3) * 4);
    let (chunks, remainder) = input.as_chunks::<3>();
    for chunk in chunks {
        let value = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32;
        output.push(alphabet[((value >> 18) & 63) as usize]);
        output.push(alphabet[((value >> 12) & 63) as usize]);
        output.push(alphabet[((value >> 6) & 63) as usize]);
        output.push(alphabet[(value & 63) as usize]);
    }
    match remainder {
        [first] => {
            let value = (*first as u32) << 16;
            output.push(alphabet[((value >> 18) & 63) as usize]);
            output.push(alphabet[((value >> 12) & 63) as usize]);
            if !url_safe {
                output.push(b'=');
                output.push(b'=');
            }
        }
        [first, second] => {
            let value = ((*first as u32) << 16) | ((*second as u32) << 8);
            output.push(alphabet[((value >> 18) & 63) as usize]);
            output.push(alphabet[((value >> 12) & 63) as usize]);
            output.push(alphabet[((value >> 6) & 63) as usize]);
            if !url_safe {
                output.push(b'=');
            }
        }
        [] => {}
        _ => unreachable!("a three-byte chunk has at most a two-byte remainder"),
    }
    output
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
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    const HASH_VECTORS: [(&str, HashAlgorithm, &str); 9] = [
        (
            "",
            HashAlgorithm::Sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            HashAlgorithm::Sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            HashAlgorithm::Sha256,
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            "abc",
            HashAlgorithm::Sha384,
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
        ),
        (
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            HashAlgorithm::Sha384,
            "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039",
        ),
        (
            "",
            HashAlgorithm::Sha384,
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b",
        ),
        (
            "abc",
            HashAlgorithm::Sha512,
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            HashAlgorithm::Sha512,
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
        ),
        (
            "",
            HashAlgorithm::Sha512,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
        ),
    ];

    // Unit test: published FIPS 180-4 / NIST CAVP hash vectors.
    #[test]
    fn published_hash_vectors_match_incremental_and_one_shot() {
        for (message, algorithm, expected) in HASH_VECTORS {
            let mut state = Hasher::new(algorithm);
            let bytes = message.as_bytes();
            let split = bytes.len() / 2;
            state.update(&bytes[..split]).unwrap();
            state.update(&bytes[split..]).unwrap();
            assert_eq!(encode_hex(&state.finalize()), expected, "{message:?}");
            assert_eq!(
                digest(algorithm, bytes, OutputEncoding::Hex).unwrap(),
                expected.as_bytes()
            );
        }
    }

    // Unit test: exact algorithm name parsing rules.
    #[test]
    fn algorithm_names_are_small_and_case_insensitive() {
        assert_eq!(HashAlgorithm::parse("sha256"), Some(HashAlgorithm::Sha256));
        assert_eq!(HashAlgorithm::parse("SHA384"), Some(HashAlgorithm::Sha384));
        assert_eq!(HashAlgorithm::parse("Sha512"), Some(HashAlgorithm::Sha512));
        for name in [
            "", "sha1", "sha224", "sha2-256", "sha-256", "md5", "SHA3-256",
        ] {
            assert_eq!(HashAlgorithm::parse(name), None, "{name:?}");
        }
    }

    // Unit test: published RFC 4231 HMAC test vectors.
    #[test]
    fn published_hmac_vectors_match() {
        let cases: [(Vec<u8>, &[u8], HashAlgorithm, &str); 9] = [
            (
                vec![0x0b; 20],
                b"Hi There",
                HashAlgorithm::Sha256,
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                HashAlgorithm::Sha256,
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                HashAlgorithm::Sha256,
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
            (
                vec![0x0b; 20],
                b"Hi There",
                HashAlgorithm::Sha384,
                "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                HashAlgorithm::Sha384,
                "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                HashAlgorithm::Sha384,
                "88062608d3e6ad8a0aa2ace014c8a86f0aa635d947ac9febe83ef4e55966144b2a5ab39dc13814b94e3ab6e101a34f27",
            ),
            (
                vec![0x0b; 20],
                b"Hi There",
                HashAlgorithm::Sha512,
                "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                HashAlgorithm::Sha512,
                "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                HashAlgorithm::Sha512,
                "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb",
            ),
        ];
        for (key, data, algorithm, expected) in cases {
            let mut state = HmacHasher::new(algorithm, &key).unwrap();
            state.update(data).unwrap();
            assert_eq!(encode_hex(&state.finalize()), expected);
            assert_eq!(
                hmac(algorithm, &key, data, OutputEncoding::Hex).unwrap(),
                expected.as_bytes()
            );
        }
    }

    // Unit test: edge case for zero-length HMAC key.
    #[test]
    fn empty_hmac_key_is_accepted() {
        let mut state = HmacHasher::new(HashAlgorithm::Sha256, b"").unwrap();
        state.update(b"").unwrap();
        assert_eq!(state.finalize().len(), 32);
    }

    // Unit test: codec assertions for every digest output encoding.
    #[test]
    fn digest_encodings_are_stable() {
        let bytes = decode_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            encode_digest(&bytes, OutputEncoding::Hex),
            b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            std::str::from_utf8(&encode_digest(&bytes, OutputEncoding::Base64)).unwrap(),
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        );
        assert_eq!(
            std::str::from_utf8(&encode_digest(&bytes, OutputEncoding::Base64Url)).unwrap(),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
        assert_eq!(encode_digest(&bytes, OutputEncoding::Binary), bytes);
        assert_eq!(encode_digest(&bytes, OutputEncoding::Latin1), bytes);
    }

    // Unit test: codec base64 padding for all chunk remainders.
    #[test]
    fn base64_padding_is_correct_for_every_remainder() {
        for (input, standard, url_safe) in [
            (&b""[..], "", ""),
            (b"f", "Zg==", "Zg"),
            (b"fo", "Zm8=", "Zm8"),
            (b"foo", "Zm9v", "Zm9v"),
            (b"foob", "Zm9vYg==", "Zm9vYg"),
            (b"fooba", "Zm9vYmE=", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy", "Zm9vYmFy"),
        ] {
            assert_eq!(
                std::str::from_utf8(&base64_encode(input, false)).unwrap(),
                standard
            );
            assert_eq!(
                std::str::from_utf8(&base64_encode(input, true)).unwrap(),
                url_safe
            );
        }
    }

    // Unit test: boundary check for MAX_HASH_INPUT_BYTES.
    #[test]
    fn aggregate_input_bound_is_enforced() {
        let mut state = Hasher::new(HashAlgorithm::Sha256);
        state.update(&vec![0u8; MAX_HASH_INPUT_BYTES]).unwrap();
        assert_eq!(state.remaining(), 0);
        assert_eq!(
            state.update(&[0u8]).unwrap_err().code(),
            HashErrorCode::Limit
        );
    }

    // Unit test: boundary check for MAX_HMAC_KEY_BYTES.
    #[test]
    fn hmac_key_bound_is_enforced() {
        let key = vec![0u8; MAX_HMAC_KEY_BYTES + 1];
        let error = match HmacHasher::new(HashAlgorithm::Sha256, &key) {
            Ok(_) => panic!("an oversized key must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::Limit);
        assert!(HmacHasher::new(HashAlgorithm::Sha256, &key[..MAX_HMAC_KEY_BYTES]).is_ok());
    }

    // Unit test: DigestTable open-object cap and lifecycle.
    #[test]
    fn digest_table_enforces_cap_and_lifecycle() {
        let mut table = DigestTable::with_limit(2);
        let h1 = table.create_hash(HashAlgorithm::Sha256).unwrap();
        let h2 = table.create_hmac(HashAlgorithm::Sha256, b"key").unwrap();
        assert_eq!(
            table.create_hash(HashAlgorithm::Sha256).unwrap_err().code(),
            HashErrorCode::TooManyObjects
        );
        table.update(h1, b"abc").unwrap();
        let hex = table.digest(h1, OutputEncoding::Hex).unwrap();
        assert_eq!(
            hex,
            b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            table.update(h1, b"more").unwrap_err().code(),
            HashErrorCode::Digested
        );
        table.close(h2);
        assert!(table.is_empty());
    }

    // Unit test: exact error code strings and message invariants.
    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        for (code, expected) in [
            (HashErrorCode::Algorithm, "NEO_CRYPTO_ALGORITHM"),
            (HashErrorCode::Encoding, "NEO_CRYPTO_ENCODING"),
            (HashErrorCode::Limit, "NEO_CRYPTO_LIMIT"),
            (HashErrorCode::Digested, "NEO_CRYPTO_DIGESTED"),
            (HashErrorCode::TooManyObjects, "NEO_CRYPTO_TOO_MANY_OBJECTS"),
            (HashErrorCode::Closed, "NEO_CRYPTO_CLOSED"),
            (HashErrorCode::Internal, "NEO_CRYPTO_INTERNAL"),
        ] {
            let err = HashError::new(code);
            assert_eq!(err.code().as_str(), expected);
            let message = err.message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
            assert_eq!(err.to_string(), message);
        }
    }
}
