//! RFC 9562 version 4 and version 7 UUID generation for `neo:uuid`.
//!
//! The JavaScript module object is installed by [`crate::builtins`] and is
//! exposed as `Neo.getBuiltInModule("uuid")`, `import ... from "uuid"`, and
//! `import ... from "neo:uuid"`. The `uuid/v4` and `uuid/v7` subpaths are thin
//! modules over the same function objects and also provide a default export.
//! This file registers the two private host ops it calls: one for [`v4`] and
//! one for [`v7`].
//!
//! # Output
//!
//! Both functions return the canonical RFC 9562 text form: 32 lowercase
//! hexadecimal digits in five hyphen-separated groups of 8, 4, 4, 4, and 12,
//! for a total of 36 characters. There are no braces, `urn:uuid:` prefix,
//! uppercase, or alternate output option in this slice. The canonical text
//! order is the same as byte order, so the version 7 timestamp prefix also
//! sorts correctly when the strings are compared.
//!
//! # Version 4
//!
//! [`v4`] fills a 128-bit value with the operating system CSPRNG, then sets the
//! four version bits to `0100` and the two variant bits to `10`. The remaining
//! 122 bits are random. A version 4 UUID carries no time, node, or sequence
//! information, so its values have no order and must not be used as a sort key.
//! The probability of a collision is negligible only while the generator is a
//! real CSPRNG; this module does not fall back to a time or `Math.random`
//! source.
//!
//! # Version 7
//!
//! [`v7`] uses the RFC 9562 layout:
//!
//! ```text
//! bits 127..80  48-bit big-endian Unix timestamp in milliseconds
//! bits  79..76  4-bit version, 0111
//! bits  75..64  12 random bits (rand_a)
//! bits  63..62  2-bit variant, 10
//! bits  61..0   62 random bits (rand_b)
//! ```
//!
//! This is the random-payload method: there is no per-process counter in
//! `rand_a` or `rand_b`. As a result:
//!
//! * Values generated in different milliseconds sort by their embedded
//!   timestamp when the same system clock produced them and it moved forward.
//! * Values generated within one millisecond carry random low bits and have
//!   **no defined order relative to each other**.
//! * A backward system clock step produces values that sort before earlier
//!   ones. UUIDs from different processes or machines are not ordered.
//! * This module does **not** promise monotonicity, uniqueness, or a total
//!   order. Use a database sequence, a Lamport clock, or an explicit ordering
//!   field when the application needs one. The 74 random payload bits make a
//!   same-millisecond collision unlikely but do not make it impossible.
//!
//! The 48-bit timestamp is a format limit. A current time before the Unix
//! epoch or at or beyond 2^48 milliseconds (roughly the year 10889) is refused
//! with `NEO_UUID_CLOCK` rather than wrapped, truncated, or clamped. The clock
//! is read before the CSPRNG is called, so an unrepresentable time costs no
//! entropy.
//!
//! # Randomness
//!
//! Every random bit comes from the operating system CSPRNG through
//! `getrandom::fill`. There is no insecure fallback, no `Math.random`, and no
//! time-derived substitute. If the operating system refuses entropy, the call
//! fails with `NEO_UUID_RANDOM` and returns no UUID.
//!
//! The `uuid` crate supplies the RFC 9562 bit layout, version and variant
//! setting, and canonical formatting through [`uuid::Builder`]. Its convenience
//! constructors `Uuid::new_v4` and `Uuid::now_v7` are deliberately not used:
//! they read entropy through a helper that panics if `getrandom` fails. neojs
//! needs a returned error instead of an abort, so it fills its own buffer and
//! passes it to the builder. The crate is MIT OR Apache-2.0; its notices are in
//! `neojs-core/THIRD_PARTY_NOTICES.md`.
//!
//! # Errors
//!
//! The host ops return a null-prototype result object. The JavaScript wrapper
//! turns a failure into an ordinary `Error` with a stable own `code`:
//!
//! * `NEO_UUID_RANDOM` — the operating system CSPRNG failed.
//! * `NEO_UUID_CLOCK` — the system clock is before the Unix epoch or beyond the
//!   48-bit version 7 timestamp range.
//! * `NEO_UUID_INTERNAL` — the engine could not allocate the 36-character
//!   result string.
//!
//! Messages are fixed and never include entropy, the clock value, or any other
//! input. A failed call returns no partial or placeholder UUID.
//!
//! # Scope
//!
//! This slice implements generation only. It does not parse or validate UUID
//! text, extract a timestamp, compare versions, generate nil or max UUIDs, or
//! implement versions 1, 3, 5, 6, or 8. Those are additive work. It reads no
//! file, environment variable, network, or process state beyond the system
//! clock, and it installs no global. It is not a sandbox.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use deno_core::op2;
use deno_core::v8;
use uuid::Builder;

/// Largest value the RFC 9562 version 7 48-bit millisecond field can hold.
const MAX_UNIX_MILLIS: u64 = (1 << 48) - 1;

/// Bytes of random payload a version 7 UUID uses after its timestamp.
const V7_RANDOM_BYTES: usize = 10;

/// Test-only, deterministically consumed injection for the next UUID entropy
/// draw. It is compiled only with the off-by-default `test-support` feature, so
/// a normal build has no injection point.
#[cfg(feature = "test-support")]
static FAIL_NEXT_UUID_RANDOM: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(super) fn fail_next_uuid_random() {
    FAIL_NEXT_UUID_RANDOM.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_uuid_random_failure() -> bool {
    FAIL_NEXT_UUID_RANDOM.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// The failure category carried to the JavaScript wrapper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UuidErrorCode {
    /// The operating system CSPRNG failed.
    Random,
    /// The system clock is outside the version 7 timestamp range.
    Clock,
    /// The engine could not allocate the result string.
    Internal,
}

impl UuidErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    fn as_str(self) -> &'static str {
        match self {
            Self::Random => "NEO_UUID_RANDOM",
            Self::Clock => "NEO_UUID_CLOCK",
            Self::Internal => "NEO_UUID_INTERNAL",
        }
    }

    /// A fixed message that never includes entropy or a clock value.
    fn message(self) -> &'static str {
        match self {
            Self::Random => "operating system random number generator failed",
            Self::Clock => "system clock is outside the UUID version 7 timestamp range",
            Self::Internal => "could not allocate the UUID string",
        }
    }
}

/// A failure safe to describe without echoing any input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UuidError {
    code: UuidErrorCode,
}

impl UuidError {
    fn new(code: UuidErrorCode) -> Self {
        Self { code }
    }

    fn random() -> Self {
        Self::new(UuidErrorCode::Random)
    }

    fn clock() -> Self {
        Self::new(UuidErrorCode::Clock)
    }
}

/// Fills `bytes` from the operating system CSPRNG.
///
/// There is no fallback. A failure leaves the buffer zeroed and returns
/// [`UuidErrorCode::Random`]; callers must not use the buffer.
fn fill_random(bytes: &mut [u8]) -> Result<(), UuidError> {
    #[cfg(feature = "test-support")]
    if take_uuid_random_failure() {
        return Err(UuidError::random());
    }
    getrandom::fill(bytes).map_err(|_| UuidError::random())
}

/// Formats 16 already-random bytes as a version 4 UUID.
///
/// [`Builder::from_random_bytes`] sets the version and variant bits, so a
/// caller-supplied buffer is a complete random payload, not a finished UUID.
fn v4_from_random(bytes: [u8; 16]) -> String {
    Builder::from_random_bytes(bytes)
        .into_uuid()
        .hyphenated()
        .to_string()
}

/// Computes a version 4 UUID with a caller-supplied entropy source.
///
/// The public path supplies [`fill_random`]. The injectable form exists so a
/// deterministic test can prove that an entropy failure is returned rather than
/// turning into a UUID.
fn v4_with_entropy(
    fill: impl FnOnce(&mut [u8]) -> Result<(), UuidError>,
) -> Result<String, UuidError> {
    let mut bytes = [0u8; 16];
    fill(&mut bytes)?;
    Ok(v4_from_random(bytes))
}

/// Returns a new version 4 UUID in canonical lowercase text form.
fn v4() -> Result<String, UuidError> {
    v4_with_entropy(fill_random)
}

/// Converts a [`SystemTime`] to Unix milliseconds.
///
/// A time before the Unix epoch and a time at or beyond 2^48 milliseconds are
/// refused instead of wrapped. The 48-bit field is a UUID version 7 limit.
fn unix_millis(time: SystemTime) -> Result<u64, UuidError> {
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
fn v7_from(millis: u64, counter_random_bytes: [u8; V7_RANDOM_BYTES]) -> Result<String, UuidError> {
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
/// The clock value is checked by [`v7_from`] before any bytes are used. The
/// public path reads the clock first, then draws entropy.
fn v7_with_entropy(
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

/// Returns a new version 7 UUID in canonical lowercase text form.
fn v7() -> Result<String, UuidError> {
    let millis = unix_millis(SystemTime::now())?;
    v7_with_entropy(millis, fill_random)
}

/// Creates the null-prototype result object the JavaScript wrapper reads.
///
/// A null prototype means a script cannot intercept these property writes or
/// reads through `Object.prototype`.
fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

fn set_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    set(scope, object, name, value.into());
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: UuidError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    set_string(scope, object, "name", "Error");
    set_string(scope, object, "code", error.code.as_str());
    set_string(scope, object, "message", error.code.message());
}

/// Turns a Rust string result into `{ ok: true, value }` or `{ ok: false, ... }`.
fn outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<String, UuidError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(value) => match v8::String::new(scope, &value) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_failure(scope, object, UuidError::new(UuidErrorCode::Internal)),
        },
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

#[op2]
fn op_neo_uuid_v4<'s, 'i>(scope: &mut v8::PinScope<'s, 'i>) -> v8::Local<'s, v8::Value> {
    outcome(scope, v4())
}

#[op2]
fn op_neo_uuid_v7<'s, 'i>(scope: &mut v8::PinScope<'s, 'i>) -> v8::Local<'s, v8::Value> {
    outcome(scope, v7())
}

deno_core::extension!(
    neojs_uuid,
    ops = [op_neo_uuid_v4, op_neo_uuid_v7],
    docs = "RFC 9562 UUID version 4 and 7 generation behind `Neo.getBuiltInModule(\"uuid\")`."
);

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use uuid::Uuid;
    use uuid::Variant;
    use uuid::Version;

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
    /// its random payload. `Builder::from_random_bytes` only sets bits that are
    /// already `0100` and `10` in the vector.
    #[test]
    fn v4_matches_the_rfc_9562_example() {
        let random = bytes16("919108f752d143209bacf847db4148a8");
        assert_eq!(
            v4_from_random(random),
            "919108f7-52d1-4320-9bac-f847db4148a8"
        );
    }

    /// The RFC 9562 Appendix A.7 example value. Its 10 bytes after the
    /// timestamp are `cc c3 98 c4 dc 0c 0c 07 39 8f`.
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
        assert_eq!(
            v4_with_entropy(|_| Err(UuidError::random())),
            Err(UuidError::random())
        );
        assert_eq!(
            v7_with_entropy(0, |_| Err(UuidError::random())),
            Err(UuidError::random())
        );
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
        assert_eq!(UuidErrorCode::Random.as_str(), "NEO_UUID_RANDOM");
        assert_eq!(UuidErrorCode::Clock.as_str(), "NEO_UUID_CLOCK");
        assert_eq!(UuidErrorCode::Internal.as_str(), "NEO_UUID_INTERNAL");
        for code in [
            UuidErrorCode::Random,
            UuidErrorCode::Clock,
            UuidErrorCode::Internal,
        ] {
            assert!(!code.message().is_empty());
        }
    }
}
