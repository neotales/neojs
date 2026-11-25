//! Engine-neutral RFC 9562 version 4 and version 7 UUID generation for
//! `vael:uuid`, `vael:uuid/v4`, and `vael:uuid/v7`.
//!
//! # Output
//!
//! Both [`v4`] and [`v7`] return the canonical RFC 9562 text form: 32 lowercase
//! hexadecimal digits in five hyphen-separated groups of 8, 4, 4, 4, and 12,
//! for a total of 36 characters.
//!
//! # Randomness
//!
//! Every random bit comes from the operating system CSPRNG through
//! [`getrandom::fill`]. There is no insecure fallback, no `Math.random`, and no
//! time-derived substitute. If the operating system refuses entropy, the call
//! fails with [`UuidErrorCode::Random`] (`VAEL_UUID_RANDOM`) and returns no UUID.
//!
//! The `uuid` crate supplies the RFC 9562 bit layout, version and variant
//! setting, and canonical formatting through [`uuid::Builder`]. Its convenience
//! constructors `Uuid::new_v4` and `Uuid::now_v7` are deliberately not used:
//! they read entropy through a helper that panics if `getrandom` fails.
//!
//! # Version 7 clock bounds
//!
//! The 48-bit timestamp is a format limit. A current time before the Unix
//! epoch or at or beyond `2^48` milliseconds is refused with
//! [`UuidErrorCode::Clock`] (`VAEL_UUID_CLOCK`) rather than wrapped, truncated,
//! or clamped. The clock is read before the CSPRNG is called, so an
//! unrepresentable time costs no entropy.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Builder;

/// The canonical `vael:uuid` module specifier and registry key.
pub const SPECIFIER: &str = "vael:uuid";

/// The canonical `vael:uuid/v4` subpath specifier and registry key.
pub const V4_SPECIFIER: &str = "vael:uuid/v4";

/// The canonical `vael:uuid/v7` subpath specifier and registry key.
pub const V7_SPECIFIER: &str = "vael:uuid/v7";

/// The named exports of `vael:uuid`, in the module object's property order.
pub const EXPORTS: &[&str] = &["v4", "v7"];

/// The exports of `vael:uuid/v4`, matching the V8 registry object.
pub const V4_EXPORTS: &[&str] = &["default", "v4"];

/// The exports of `vael:uuid/v7`, matching the V8 registry object.
pub const V7_EXPORTS: &[&str] = &["default", "v7"];

/// Largest value the RFC 9562 version 7 48-bit millisecond field can hold.
pub const MAX_UNIX_MILLIS: u64 = (1 << 48) - 1;

/// Bytes of random payload a version 7 UUID uses after its timestamp.
pub const V7_RANDOM_BYTES: usize = 10;

/// The shared module descriptor for `vael:uuid`.
pub const MODULE: crate::BuiltinModule = crate::BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: crate::Backend::Rust,
};

/// The shared module descriptor for `vael:uuid/v4`.
pub const V4_MODULE: crate::BuiltinModule = crate::BuiltinModule {
    specifier: V4_SPECIFIER,
    exports: V4_EXPORTS,
    backend: crate::Backend::Rust,
};

/// The shared module descriptor for `vael:uuid/v7`.
pub const V7_MODULE: crate::BuiltinModule = crate::BuiltinModule {
    specifier: V7_SPECIFIER,
    exports: V7_EXPORTS,
    backend: crate::Backend::Rust,
};

/// The failure category carried to engine bindings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UuidErrorCode {
    /// The operating system CSPRNG failed.
    Random,
    /// The system clock is outside the version 7 timestamp range.
    Clock,
    /// The engine could not allocate the result string.
    Internal,
}

impl UuidErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Random => "VAEL_UUID_RANDOM",
            Self::Clock => "VAEL_UUID_CLOCK",
            Self::Internal => "VAEL_UUID_INTERNAL",
        }
    }

    /// A fixed message that never includes entropy or a clock value.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Random => "operating system random number generator failed",
            Self::Clock => "system clock is outside the UUID version 7 timestamp range",
            Self::Internal => "could not allocate the UUID string",
        }
    }
}

/// A failure safe to describe without echoing any input.
///
/// A random failure retains the `getrandom` error so it can be diagnosed. The
/// error is only ever converted into a fixed diagnostic string: an OS error
/// number, or one of the crate's fixed categories. Its `Display` output is not
/// forwarded, so locale text or any host-provided string cannot reach
/// JavaScript. UUID bytes, entropy bytes, and clock values are never retained
/// here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UuidError {
    code: UuidErrorCode,
    source: Option<getrandom::Error>,
}

impl UuidError {
    /// Creates a `UuidError` for a failure category without an OS source.
    #[must_use]
    pub const fn new(code: UuidErrorCode) -> Self {
        Self { code, source: None }
    }

    /// Returns a random failure that keeps the underlying OS entropy error.
    #[must_use]
    pub const fn random(source: getrandom::Error) -> Self {
        Self {
            code: UuidErrorCode::Random,
            source: Some(source),
        }
    }

    /// Returns a random failure using `getrandom::Error::UNEXPECTED`, for
    /// engine test-support fault injection without exposing `getrandom` as a
    /// direct engine dependency.
    #[doc(hidden)]
    #[must_use]
    pub const fn unexpected_random() -> Self {
        Self::random(getrandom::Error::UNEXPECTED)
    }

    /// Returns a clock-bounds failure (`VAEL_UUID_CLOCK`).
    #[must_use]
    pub const fn clock() -> Self {
        Self::new(UuidErrorCode::Clock)
    }

    /// Returns an internal string-allocation failure (`VAEL_UUID_INTERNAL`).
    #[must_use]
    pub const fn internal() -> Self {
        Self::new(UuidErrorCode::Internal)
    }

    /// Returns the stable error code category.
    #[must_use]
    pub const fn code(self) -> UuidErrorCode {
        self.code
    }

    /// A fixed, non-secret diagnostic for the JavaScript `cause` property.
    ///
    /// Only a numeric OS error code or a fixed `getrandom` category is exposed,
    /// never the error's `Display` text. UUID bytes, entropy, and clock values
    /// are not part of this value.
    #[must_use]
    pub fn diagnostic(self) -> Option<String> {
        self.source.map(|source| {
            if let Some(errno) = source.raw_os_error() {
                format!("getrandom: OS error {errno}")
            } else if source == getrandom::Error::UNSUPPORTED {
                "getrandom: unsupported platform".to_string()
            } else if source == getrandom::Error::ERRNO_NOT_POSITIVE {
                "getrandom: non-positive OS error code".to_string()
            } else if source == getrandom::Error::UNEXPECTED {
                "getrandom: unexpected error".to_string()
            } else {
                "getrandom: internal error".to_string()
            }
        })
    }
}

impl fmt::Display for UuidError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.message())
    }
}

impl std::error::Error for UuidError {}

/// Fills `bytes` from the operating system CSPRNG.
///
/// There is no fallback. A failure leaves the buffer zeroed and returns
/// [`UuidErrorCode::Random`] with the underlying `getrandom` error retained for
/// a safe diagnostic; callers must not use the buffer.
pub fn fill_random(bytes: &mut [u8]) -> Result<(), UuidError> {
    if let Err(error) = getrandom::fill(bytes) {
        bytes.fill(0);
        return Err(UuidError::random(error));
    }
    Ok(())
}

/// Formats 16 already-random bytes as a version 4 UUID.
///
/// [`Builder::from_random_bytes`] sets the version and variant bits, so a
/// caller-supplied buffer is a complete random payload, not a finished UUID.
#[must_use]
pub fn v4_from_random(bytes: [u8; 16]) -> String {
    Builder::from_random_bytes(bytes)
        .into_uuid()
        .hyphenated()
        .to_string()
}

/// Computes a version 4 UUID with a caller-supplied entropy source.
///
/// The public path supplies [`fill_random`]. The injectable form exists so
/// engine test-support hooks and deterministic tests can prove that an entropy
/// failure is returned rather than turning into a UUID.
pub fn v4_with_entropy(
    fill: impl FnOnce(&mut [u8]) -> Result<(), UuidError>,
) -> Result<String, UuidError> {
    let mut bytes = [0u8; 16];
    fill(&mut bytes)?;
    Ok(v4_from_random(bytes))
}

/// Returns a new version 4 UUID in canonical lowercase text form.
pub fn v4() -> Result<String, UuidError> {
    v4_with_entropy(fill_random)
}

/// Converts a [`SystemTime`] to Unix milliseconds.
///
/// A time before the Unix epoch and a time at or beyond `2^48` milliseconds
/// are refused instead of wrapped. The 48-bit field is a UUID version 7 limit.
pub fn unix_millis(time: SystemTime) -> Result<u64, UuidError> {
    let elapsed = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| UuidError::clock())?;
    let millis = u64::try_from(elapsed.as_millis()).map_err(|_| UuidError::clock())?;
    if millis > MAX_UNIX_MILLIS {
        return Err(UuidError::clock());
    }
    Ok(millis)
}

/// Formats a millisecond timestamp and 10 random bytes as a version 7 UUID.
///
/// [`Builder::from_unix_timestamp_millis`] sets the version and variant bits and
/// embeds the low 48 bits of `millis`. The range check here prevents the builder
/// from silently dropping higher bits.
pub fn v7_from(
    millis: u64,
    counter_random_bytes: [u8; V7_RANDOM_BYTES],
) -> Result<String, UuidError> {
    if millis > MAX_UNIX_MILLIS {
        return Err(UuidError::clock());
    }
    Ok(
        Builder::from_unix_timestamp_millis(millis, &counter_random_bytes)
            .into_uuid()
            .hyphenated()
            .to_string(),
    )
}

/// Computes a version 7 UUID at `millis` with a caller-supplied entropy source.
///
/// The clock value is checked before any bytes are requested. The public path
/// reads the system clock first, then draws entropy.
pub fn v7_with_entropy(
    millis: u64,
    fill: impl FnOnce(&mut [u8]) -> Result<(), UuidError>,
) -> Result<String, UuidError> {
    if millis > MAX_UNIX_MILLIS {
        return Err(UuidError::clock());
    }
    let mut bytes = [0u8; V7_RANDOM_BYTES];
    fill(&mut bytes)?;
    v7_from(millis, bytes)
}

/// Computes a version 7 UUID using the current system clock and a
/// caller-supplied entropy source.
pub fn v7_now_with_entropy(
    fill: impl FnOnce(&mut [u8]) -> Result<(), UuidError>,
) -> Result<String, UuidError> {
    let millis = unix_millis(SystemTime::now())?;
    v7_with_entropy(millis, fill)
}

/// Returns a new version 7 UUID in canonical lowercase text form.
pub fn v7() -> Result<String, UuidError> {
    v7_now_with_entropy(fill_random)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use uuid::{Uuid, Variant, Version};

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "hex length");
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    fn bytes16(value: &str) -> [u8; 16] {
        decode_hex(value).try_into().expect("16 bytes")
    }

    fn bytes10(value: &str) -> [u8; 10] {
        decode_hex(value).try_into().expect("10 bytes")
    }

    /// The RFC 9562 Appendix A.4 example value, reproduced byte for byte from
    /// its random payload.
    #[test]
    fn v4_matches_the_rfc_9562_example() {
        let random = bytes16("919108f752d143209bacf847db4148a8");
        assert_eq!(
            v4_from_random(random),
            "919108f7-52d1-4320-9bac-f847db4148a8"
        );
    }

    /// The RFC 9562 Appendix A.7 example value.
    #[test]
    fn v7_matches_the_rfc_9562_example() {
        let millis = 0x017f_22e2_79b0;
        let random = bytes10("ccc398c4dc0c0c07398f");
        assert_eq!(
            v7_from(millis, random).unwrap(),
            "017f22e2-79b0-7cc3-98c4-dc0c0c07398f"
        );
    }

    #[test]
    fn v4_sets_the_version_and_rfc_variant() {
        for random in [
            [0u8; 16],
            [0xff; 16],
            bytes16("0123456789abcdef0123456789abcdef"),
        ] {
            let text = v4_from_random(random);
            let uuid = Uuid::parse_str(&text).unwrap();
            assert_eq!(uuid.get_version(), Some(Version::Random));
            assert_eq!(uuid.get_variant(), Variant::RFC4122);
            assert_eq!(uuid.as_bytes()[6] >> 4, 4);
            assert_eq!(uuid.as_bytes()[8] >> 6, 0b10);
            assert_eq!(text.len(), 36);
        }
    }

    #[test]
    fn v7_sets_the_version_variant_and_timestamp() {
        let millis = 1_700_000_000_123;
        let text = v7_from(millis, [0x11; V7_RANDOM_BYTES]).unwrap();
        let uuid = Uuid::parse_str(&text).unwrap();
        assert_eq!(uuid.get_version(), Some(Version::SortRand));
        assert_eq!(uuid.get_variant(), Variant::RFC4122);
        assert_eq!(uuid.as_bytes()[6] >> 4, 7);
        assert_eq!(uuid.as_bytes()[8] >> 6, 0b10);

        let timestamp = uuid.get_timestamp().expect("version 7 timestamp");
        assert_eq!(timestamp.to_unix(), (1_700_000_000, 123_000_000));

        let bytes = uuid.as_bytes();
        let embedded = u64::from_be_bytes([
            0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        ]);
        assert_eq!(embedded, millis);
    }

    #[test]
    fn v7_refuses_a_timestamp_that_does_not_fit_48_bits() {
        assert_eq!(
            v7_from(MAX_UNIX_MILLIS + 1, [0; V7_RANDOM_BYTES]),
            Err(UuidError::clock())
        );
        assert!(v7_from(MAX_UNIX_MILLIS, [0; V7_RANDOM_BYTES]).is_ok());
    }

    #[test]
    fn unix_millis_handles_the_epoch_and_both_ends() {
        assert_eq!(unix_millis(UNIX_EPOCH).unwrap(), 0);
        assert_eq!(
            unix_millis(UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)).unwrap(),
            1_700_000_000_123
        );
        assert_eq!(
            unix_millis(UNIX_EPOCH - Duration::from_millis(1)),
            Err(UuidError::clock())
        );
        assert_eq!(
            unix_millis(UNIX_EPOCH + Duration::from_millis(MAX_UNIX_MILLIS)),
            Ok(MAX_UNIX_MILLIS)
        );
        assert_eq!(
            unix_millis(UNIX_EPOCH + Duration::from_millis(MAX_UNIX_MILLIS + 1)),
            Err(UuidError::clock())
        );
    }

    #[test]
    fn an_entropy_failure_is_returned_and_produces_no_uuid() {
        let failure = UuidError::random(getrandom::Error::UNEXPECTED);
        assert_eq!(v4_with_entropy(|_| Err(failure)), Err(failure));
        assert_eq!(v7_with_entropy(0, |_| Err(failure)), Err(failure));
    }

    #[test]
    fn a_random_failure_retains_a_fixed_non_secret_diagnostic() {
        let failure = UuidError::random(getrandom::Error::UNEXPECTED);
        assert_eq!(failure.code(), UuidErrorCode::Random);
        assert_eq!(
            failure.diagnostic().as_deref(),
            Some("getrandom: unexpected error")
        );

        let unsupported = UuidError::random(getrandom::Error::UNSUPPORTED);
        assert_eq!(
            unsupported.diagnostic().as_deref(),
            Some("getrandom: unsupported platform")
        );

        // A clock or allocation failure has no OS entropy source to report.
        assert_eq!(UuidError::clock().diagnostic(), None);
        assert_eq!(UuidError::internal().diagnostic(), None);
    }

    #[test]
    fn an_invalid_clock_is_checked_before_entropy_is_requested() {
        let mut called = false;
        let result = v7_with_entropy(MAX_UNIX_MILLIS + 1, |_| {
            called = true;
            Ok(())
        });
        assert_eq!(result, Err(UuidError::clock()));
        assert!(
            !called,
            "entropy was requested for an unrepresentable clock"
        );
    }

    #[test]
    fn generated_values_are_canonical_lowercase_text() {
        for text in [v4().unwrap(), v7().unwrap()] {
            assert_eq!(text.len(), 36);
            assert!(
                text.bytes().all(|byte| {
                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || byte == b'-'
                }),
                "not canonical lowercase text: {text}"
            );
            assert_eq!(text.as_bytes()[8], b'-');
            assert_eq!(text.as_bytes()[13], b'-');
            assert_eq!(text.as_bytes()[18], b'-');
            assert_eq!(text.as_bytes()[23], b'-');
        }
    }

    #[test]
    fn error_codes_and_messages_are_stable() {
        assert_eq!(UuidErrorCode::Random.as_str(), "VAEL_UUID_RANDOM");
        assert_eq!(UuidErrorCode::Clock.as_str(), "VAEL_UUID_CLOCK");
        assert_eq!(UuidErrorCode::Internal.as_str(), "VAEL_UUID_INTERNAL");
        for code in [
            UuidErrorCode::Random,
            UuidErrorCode::Clock,
            UuidErrorCode::Internal,
        ] {
            assert!(!code.message().is_empty());
        }
    }
}
