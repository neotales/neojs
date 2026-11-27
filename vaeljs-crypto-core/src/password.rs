//! PBKDF2-HMAC and bcrypt password hashing.
//!
//! * **PBKDF2**: accepts `sha256`, `sha384`, and `sha512` via [`HashAlgorithm`],
//!   bounded to `1..=MAX_PBKDF2_ITERATIONS`, `1..=MAX_PBKDF2_OUTPUT_BYTES`, and
//!   [`MAX_PBKDF2_INPUT_BYTES`] for password and salt.
//! * **bcrypt**: [`bcrypt_hash`] emits `$2b$` modular-crypt strings with a
//!   16-byte CSPRNG salt (`getrandom`) and cost in
//!   `MIN_BCRYPT_COST..=MAX_BCRYPT_COST` (default [`DEFAULT_BCRYPT_COST`]).
//!   [`bcrypt_verify`] accepts `$2a$`, `$2b$`, and `$2y$`, rejects passwords
//!   over [`MAX_BCRYPT_PASSWORD_BYTES`] (72) and hashes outside cost 4..=16
//!   without hashing, and verifies in constant time via the `bcrypt` crate.
//!   Password and salt copies made internally are wrapped in [`Zeroizing`].

use std::fmt;
use std::str::FromStr;
#[cfg(feature = "test-support")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "test-support")]
use std::sync::atomic::Ordering;

use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use sha2::Sha384;
use sha2::Sha512;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::hash::HashAlgorithm;

/// Largest PBKDF2 iteration count accepted in one call.
pub const MAX_PBKDF2_ITERATIONS: u32 = 1_000_000;

/// Largest PBKDF2 output length, in bytes.
pub const MAX_PBKDF2_OUTPUT_BYTES: usize = 1024;

/// Largest PBKDF2 password or salt length, in bytes.
pub const MAX_PBKDF2_INPUT_BYTES: usize = 1024 * 1024;

/// Smallest bcrypt cost accepted when hashing or verifying.
pub const MIN_BCRYPT_COST: u32 = 4;

/// Largest bcrypt cost accepted when hashing or verifying.
pub const MAX_BCRYPT_COST: u32 = 16;

/// Default bcrypt cost.
pub const DEFAULT_BCRYPT_COST: u32 = 10;

/// Largest password accepted by bcrypt, in bytes.
pub const MAX_BCRYPT_PASSWORD_BYTES: usize = 72;

/// Length of a complete bcrypt modular-crypt string in ASCII bytes.
pub const BCRYPT_HASH_BYTES: usize = 60;

#[cfg(feature = "test-support")]
static FAIL_NEXT_BCRYPT_RANDOM: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(crate) fn fail_next_bcrypt_random() {
    FAIL_NEXT_BCRYPT_RANDOM.store(true, Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_bcrypt_random_failure() -> bool {
    FAIL_NEXT_BCRYPT_RANDOM.swap(false, Ordering::SeqCst)
}

/// Stable machine-readable failure category for [`PasswordError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum PasswordErrorCode {
    /// The algorithm name is not `sha256`, `sha384`, or `sha512`.
    Algorithm,
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// The operating system CSPRNG failed.
    Random,
    /// An internal password-hashing invariant failed.
    Internal,
    /// A numeric argument is outside its documented range.
    Range,
}

impl PasswordErrorCode {
    /// Stable `VAEL_CRYPTO_*` code string, or `None` for range errors.
    #[must_use]
    pub const fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "VAEL_CRYPTO_ALGORITHM",
            Self::Limit => "VAEL_CRYPTO_LIMIT",
            Self::Random => "VAEL_CRYPTO_RANDOM",
            Self::Internal => "VAEL_CRYPTO_INTERNAL",
            Self::Range => return None,
        })
    }

    /// Fixed human-readable message for this failure category.
    #[must_use]
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Algorithm => "unsupported password hashing algorithm",
            Self::Limit => "input exceeds the password hashing size limit",
            Self::Random => "operating system random number generator failed",
            Self::Internal => "the password hashing operation failed internally",
            Self::Range => "numeric argument is outside its documented range",
        }
    }
}

impl fmt::Display for PasswordErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str().unwrap_or("RANGE_ERROR"))
    }
}

/// Failure returned by PBKDF2 and bcrypt operations.
///
/// Messages are fixed per [`PasswordErrorCode`] and never echo passwords,
/// salts, derived keys, or hashes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{}", .code.default_message())]
#[non_exhaustive]
pub struct PasswordError {
    code: PasswordErrorCode,
}

impl PasswordError {
    /// Create an error for `code`.
    #[must_use]
    pub const fn new(code: PasswordErrorCode) -> Self {
        Self { code }
    }

    /// The stable failure category.
    #[must_use]
    pub const fn code(self) -> PasswordErrorCode {
        self.code
    }

    /// Fixed message safe to expose to callers.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.code.default_message()
    }
}

/// Derive `length` bytes using PBKDF2-HMAC over `algorithm`, returning a
/// [`Zeroizing`] buffer that wipes derived key bytes when dropped.
///
/// # Example
///
/// ```
/// use vaeljs_crypto_core::hash::HashAlgorithm;
/// use vaeljs_crypto_core::password::pbkdf2_zeroizing;
///
/// let dk = pbkdf2_zeroizing(HashAlgorithm::Sha256, b"passwd", b"salt", 1, 32)?;
/// assert_eq!(dk.len(), 32);
/// # Ok::<(), vaeljs_crypto_core::password::PasswordError>(())
/// ```
pub fn pbkdf2_zeroizing(
    algorithm: HashAlgorithm,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Result<Zeroizing<Vec<u8>>, PasswordError> {
    if iterations == 0 || iterations > MAX_PBKDF2_ITERATIONS {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    if length == 0 || length > MAX_PBKDF2_OUTPUT_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    if password.len() > MAX_PBKDF2_INPUT_BYTES || salt.len() > MAX_PBKDF2_INPUT_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }

    let mut output = Zeroizing::new(Vec::new());
    output
        .try_reserve_exact(length)
        .map_err(|_| PasswordError::new(PasswordErrorCode::Limit))?;
    output.resize(length, 0);
    match algorithm {
        HashAlgorithm::Sha256 => pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut output),
        HashAlgorithm::Sha384 => pbkdf2_hmac::<Sha384>(password, salt, iterations, &mut output),
        HashAlgorithm::Sha512 => pbkdf2_hmac::<Sha512>(password, salt, iterations, &mut output),
    }
    Ok(output)
}

/// Derive `length` bytes using PBKDF2-HMAC over `algorithm`.
///
/// Intermediate allocation is guarded by [`Zeroizing`] during derivation; call
/// [`pbkdf2_zeroizing`] when the caller also retains the derived key in Rust.
///
/// # Example
///
/// ```
/// use vaeljs_crypto_core::hash::HashAlgorithm;
/// use vaeljs_crypto_core::password::pbkdf2;
///
/// let dk = pbkdf2(HashAlgorithm::Sha256, b"passwd", b"salt", 1, 32)?;
/// assert_eq!(dk.len(), 32);
/// # Ok::<(), vaeljs_crypto_core::password::PasswordError>(())
/// ```
pub fn pbkdf2(
    algorithm: HashAlgorithm,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Result<Vec<u8>, PasswordError> {
    let mut output = pbkdf2_zeroizing(algorithm, password, salt, iterations, length)?;
    Ok(std::mem::take(&mut *output))
}

/// Hash `password` with bcrypt (`$2b$`) using a fresh 16-byte CSPRNG salt.
///
/// Password and salt copies are wrapped in [`Zeroizing`] and cleared on return.
///
/// # Example
///
/// ```
/// use vaeljs_crypto_core::password::{bcrypt_hash, bcrypt_verify};
///
/// let encoded = bcrypt_hash(b"correct horse battery staple", 4)?;
/// assert!(encoded.starts_with("$2b$04$"));
/// assert!(bcrypt_verify(b"correct horse battery staple", &encoded));
/// assert!(!bcrypt_verify(b"wrong", &encoded));
/// # Ok::<(), vaeljs_crypto_core::password::PasswordError>(())
/// ```
pub fn bcrypt_hash(password: &[u8], cost: u32) -> Result<String, PasswordError> {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }
    if !(MIN_BCRYPT_COST..=MAX_BCRYPT_COST).contains(&cost) {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    #[cfg(feature = "test-support")]
    if take_bcrypt_random_failure() {
        return Err(PasswordError::new(PasswordErrorCode::Random));
    }
    let mut salt = Zeroizing::new([0u8; 16]);
    getrandom::fill(&mut *salt).map_err(|_| PasswordError::new(PasswordErrorCode::Random))?;
    bcrypt_hash_with_salt(password, cost, *salt)
}

fn bcrypt_hash_with_salt(
    password: &[u8],
    cost: u32,
    salt: [u8; 16],
) -> Result<String, PasswordError> {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }
    if !(MIN_BCRYPT_COST..=MAX_BCRYPT_COST).contains(&cost) {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }

    let password = Zeroizing::new(password.to_vec());
    let salt = Zeroizing::new(salt);
    bcrypt::hash_with_salt(password.as_slice(), cost, *salt)
        .map(|parts| parts.format_for_version(bcrypt::Version::TwoB))
        .map_err(|error| match error {
            bcrypt::BcryptError::Rand(_) => PasswordError::new(PasswordErrorCode::Random),
            _ => PasswordError::new(PasswordErrorCode::Internal),
        })
}

/// Verify `password` against a `$2a$`, `$2b$`, or `$2y$` bcrypt hash in
/// constant time.
///
/// Malformed strings, unsupported versions (`$2x$`), costs outside
/// `MIN_BCRYPT_COST..=MAX_BCRYPT_COST`, and passwords longer than
/// [`MAX_BCRYPT_PASSWORD_BYTES`] return `false` without computing a hash.
#[must_use]
pub fn bcrypt_verify(password: &[u8], hash: &str) -> bool {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES {
        return false;
    }
    if !is_valid_bcrypt_hash(hash) {
        return false;
    }
    let password = Zeroizing::new(password.to_vec());
    bcrypt::verify(password.as_slice(), hash).unwrap_or(false)
}

/// Validate a bcrypt modular-crypt string before running verification.
///
/// Accepts `$2a$`, `$2b$`, and `$2y$` with two-digit cost in
/// `MIN_BCRYPT_COST..=MAX_BCRYPT_COST` and valid 60-byte ASCII encoding.
#[must_use]
pub fn is_valid_bcrypt_hash(hash: &str) -> bool {
    let bytes = hash.as_bytes();
    if bytes.len() != BCRYPT_HASH_BYTES || !hash.is_ascii() {
        return false;
    }
    if bytes[0] != b'$' || bytes[3] != b'$' || bytes[6] != b'$' {
        return false;
    }
    let version = &bytes[1..3];
    if version != b"2a" && version != b"2b" && version != b"2y" {
        return false;
    }
    if !bytes[4].is_ascii_digit() || !bytes[5].is_ascii_digit() {
        return false;
    }
    let cost = u32::from(bytes[4] - b'0') * 10 + u32::from(bytes[5] - b'0');
    if !(MIN_BCRYPT_COST..=MAX_BCRYPT_COST).contains(&cost) {
        return false;
    }
    bcrypt::HashParts::from_str(hash).is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    // Unit test: published RFC 7914 PBKDF2-HMAC-SHA256 vectors.
    #[test]
    fn published_pbkdf2_sha256_vectors_match() {
        type Pbkdf2Case = (&'static [u8], &'static [u8], u32, usize, &'static str);
        let cases: [Pbkdf2Case; 2] = [
            (
                b"passwd",
                b"salt",
                1,
                64,
                "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783",
            ),
            (
                b"Password",
                b"NaCl",
                80_000,
                64,
                "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d",
            ),
        ];
        for (password, salt, iterations, length, expected) in cases {
            let output = pbkdf2(HashAlgorithm::Sha256, password, salt, iterations, length).unwrap();
            assert_eq!(encode_hex(&output), expected);
            let zeroized: Zeroizing<Vec<u8>> =
                pbkdf2_zeroizing(HashAlgorithm::Sha256, password, salt, iterations, length)
                    .unwrap();
            assert_eq!(&*zeroized, &output);
        }
    }

    // Unit test: SHA-384 and SHA-512 PBKDF2 OpenSSL-matched vectors.
    #[test]
    fn sha384_and_sha512_vectors_match() {
        let cases: [(HashAlgorithm, &str); 3] = [
            (
                HashAlgorithm::Sha384,
                "cd3443723a41cf1460cca9efeede428a8898a82d2ad4d1fc5cca08ed3f4d3cb47a62a70b3cb9ce65dcbfb9fb9d425027a8be69b53e2a22674b0939e5e0a682f7",
            ),
            (
                HashAlgorithm::Sha512,
                "c74319d99499fc3e9013acff597c23c5baf0a0bec5634c46b8352b793e324723d55caa76b2b25c43402dcfdc06cdcf66f95b7d0429420b39520006749c51a04e",
            ),
            (
                HashAlgorithm::Sha256,
                "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783",
            ),
        ];
        for (algorithm, expected) in cases {
            let output = pbkdf2(algorithm, b"passwd", b"salt", 1, 64).unwrap();
            assert_eq!(encode_hex(&output), expected);
        }
    }

    // Unit test: codec exact-bytes handling for NUL and UTF-8 inputs.
    #[test]
    fn nul_and_unicode_inputs_are_exact_bytes() {
        let expected = "e6635e1a12f44a9bfe0ea9fbf1ca3b46197ab653a51824dd04ae991232b28d9155ab1b190a9cbba124272bee01a6d70b";
        let output = pbkdf2(
            HashAlgorithm::Sha256,
            "pässwörd\0".as_bytes(),
            "sält\0".as_bytes(),
            2048,
            48,
        )
        .unwrap();
        assert_eq!(encode_hex(&output), expected);

        let expected = "60d38d8523b876fa767fb807eec4c919d8c75512bb89ad7954bc0328812eb4cc";
        let output = pbkdf2(
            HashAlgorithm::Sha256,
            b"\x00\xffpass",
            b"\x00salt\xff",
            2,
            32,
        )
        .unwrap();
        assert_eq!(encode_hex(&output), expected);
    }

    // Unit test: bounds checks for PBKDF2 iterations, output length, and input sizes.
    #[test]
    fn pbkdf2_bounds_are_enforced() {
        assert_eq!(
            pbkdf2(HashAlgorithm::Sha256, b"p", b"s", 0, 32)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            pbkdf2(
                HashAlgorithm::Sha256,
                b"p",
                b"s",
                MAX_PBKDF2_ITERATIONS + 1,
                32
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            pbkdf2(HashAlgorithm::Sha256, b"p", b"s", 1, 0)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            pbkdf2(
                HashAlgorithm::Sha256,
                b"p",
                b"s",
                1,
                MAX_PBKDF2_OUTPUT_BYTES + 1
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            pbkdf2(
                HashAlgorithm::Sha256,
                &vec![0u8; MAX_PBKDF2_INPUT_BYTES + 1],
                b"s",
                1,
                16,
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Limit
        );
        assert_eq!(
            pbkdf2(
                HashAlgorithm::Sha256,
                b"p",
                &vec![0u8; MAX_PBKDF2_INPUT_BYTES + 1],
                1,
                16,
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Limit
        );
    }

    const GO_SALT: [u8; 16] = [
        101, 201, 101, 75, 19, 227, 199, 20, 239, 236, 133, 32, 30, 109, 243, 30,
    ];
    const GO_HASH: &str = "$2b$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga";

    // Unit test: private deterministic-salt path matching Go bcrypt fixture.
    #[test]
    fn bcrypt_hash_matches_the_go_fixture_byte_for_byte() {
        let hash = bcrypt_hash_with_salt(b"allmine", 10, GO_SALT).unwrap();
        assert_eq!(hash, GO_HASH);
    }

    // Unit test: bcrypt verification vectors across $2a$, $2b$, and $2y$.
    #[test]
    fn bcrypt_verifies_2a_2b_and_2y_fixtures() {
        assert!(bcrypt_verify(b"allmine", GO_HASH));
        assert!(bcrypt_verify(
            b"allmine",
            "$2a$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(bcrypt_verify(
            b"allmine",
            "$2y$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(bcrypt_verify(
            b"",
            "$2a$06$DCq7YPn5Rq63x1Lad4cll.TV4S6ytwfsfvkgY8jIucDrjc8deX1s."
        ));
        assert!(bcrypt_verify(
            b"password",
            "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W"
        ));
        assert!(!bcrypt_verify(
            b"wrong password",
            "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W"
        ));
    }

    // Unit test: bounds and format rejection before hashing.
    #[test]
    fn bcrypt_rejects_malformed_and_overcost_hashes_without_hashing() {
        let valid = GO_HASH;
        assert!(!bcrypt_verify(b"allmine", ""));
        assert!(!bcrypt_verify(b"allmine", &valid[..59]));
        assert!(!bcrypt_verify(b"allmine", &format!("{valid}0")));
        assert!(!bcrypt_verify(
            b"allmine",
            "$2x$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(!bcrypt_verify(
            b"allmine",
            "$2b$03$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(!bcrypt_verify(
            b"allmine",
            "$2b$17$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(!bcrypt_verify(
            b"allmine",
            "$2b$31$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(!bcrypt_verify(
            b"allmine",
            "$2b$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga!"
        ));
        let non_ascii = valid.replace('X', "\u{012f}");
        assert!(!bcrypt_verify(b"allmine", &non_ascii));
    }

    // Unit test: bounds check for 72-byte password limit and cost range.
    #[test]
    fn bcrypt_password_bound_is_72_bytes() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let seventy_two = vec![b'a'; MAX_BCRYPT_PASSWORD_BYTES];
        let encoded = bcrypt_hash_with_salt(&seventy_two, 4, GO_SALT).unwrap();
        assert!(bcrypt_verify(&seventy_two, &encoded));
        assert!(!bcrypt_verify(
            &[b'a'; MAX_BCRYPT_PASSWORD_BYTES + 1],
            &encoded
        ));
        assert_eq!(
            bcrypt_hash(&[b'a'; MAX_BCRYPT_PASSWORD_BYTES + 1], 4)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Limit
        );
        assert_eq!(
            bcrypt_hash(b"p", MIN_BCRYPT_COST - 1).unwrap_err().code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            bcrypt_hash(b"p", MAX_BCRYPT_COST + 1).unwrap_err().code(),
            PasswordErrorCode::Range
        );
    }

    // Unit test: codec exact-bytes handling for NUL and UTF-8 passwords.
    #[test]
    fn bcrypt_nul_and_unicode_passwords_roundtrip() {
        for password in [
            &b"a\0b"[..],
            "pässwörd".as_bytes(),
            &[0u8; 16],
            &[0xffu8; 16],
        ] {
            let hash = bcrypt_hash_with_salt(password, 4, GO_SALT).unwrap();
            assert!(bcrypt_verify(password, &hash), "{password:?}");
            assert!(!bcrypt_verify(b"other", &hash), "{password:?}");
        }
    }

    // Unit test: $2b$ prefix check on generated hashes.
    #[test]
    fn bcrypt_generated_hashes_use_2b_and_verify() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let hash = bcrypt_hash(b"correct horse battery staple", 4).unwrap();
        assert_eq!(hash.len(), BCRYPT_HASH_BYTES);
        assert!(hash.starts_with("$2b$04$"));
        assert!(bcrypt_verify(b"correct horse battery staple", &hash));
        assert!(!bcrypt_verify(b"correct horse battery stapl", &hash));
    }

    // Unit test: exact error code strings and message invariants.
    #[test]
    fn error_codes_are_stable_and_never_echo_input() {
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Algorithm)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Limit).code().as_str(),
            Some("VAEL_CRYPTO_LIMIT")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Random)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_RANDOM")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Internal)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_INTERNAL")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Range).code().as_str(),
            None
        );
        for code in [
            PasswordErrorCode::Algorithm,
            PasswordErrorCode::Limit,
            PasswordErrorCode::Random,
            PasswordErrorCode::Internal,
            PasswordErrorCode::Range,
        ] {
            let err = PasswordError::new(code);
            let message = err.message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
            assert_eq!(err.to_string(), message);
        }
    }

    // Unit test: test-support random failure injection.
    #[cfg(feature = "test-support")]
    #[test]
    fn fail_next_bcrypt_random_triggers_once() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        fail_next_bcrypt_random();
        assert_eq!(
            bcrypt_hash(b"secret", 4).unwrap_err().code(),
            PasswordErrorCode::Random
        );
        assert!(bcrypt_hash(b"secret", 4).is_ok());
    }
}
