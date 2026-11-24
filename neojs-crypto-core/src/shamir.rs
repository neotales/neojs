//! Shamir secret sharing over GF(256) with reduction polynomial `0x11b`.
//!
//! Each secret byte is the constant term of an independent degree-`threshold - 1`
//! polynomial over GF(256), with random coefficients drawn from the operating
//! system CSPRNG (`getrandom`). Share `i` (`1..=share_count`) evaluates the
//! polynomial at field element `i`.
//!
//! Secret, share, and noise buffers are wrapped in [`Zeroizing`] and cleared on
//! drop. GF(256) multiplication uses a fixed eight-round branchless mask loop
//! and inversion uses a fixed seven-step exponentiation.

use std::fmt;
#[cfg(feature = "test-support")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "test-support")]
use std::sync::atomic::Ordering;

use thiserror::Error;
use zeroize::Zeroizing;

/// Largest secret accepted by a split, in bytes.
pub const MAX_SHAMIR_SECRET_BYTES: usize = 8 * 1024 * 1024;

/// Largest number of shares in one split or combine.
pub const MAX_SHAMIR_SHARES: usize = 255;

/// Largest `secret length * share count` product accepted by a split, and the
/// largest total share-value bytes accepted by a combine.
pub const MAX_SHAMIR_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Chunk size for temporary random coefficient buffers during splitting.
pub const NOISE_BUFFER_BYTES: usize = 64 * 1024;

/// Largest packed split payload: share bytes, one index byte per share, and an
/// eight-byte header.
pub const MAX_SHAMIR_PACKED_BYTES: usize = MAX_SHAMIR_TOTAL_BYTES + MAX_SHAMIR_SHARES + 8;

/// Stable machine-readable failure category for [`ShamirError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum ShamirErrorCode {
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// The operating system CSPRNG failed.
    Random,
    /// Two shares used the same index.
    DuplicateIndex,
    /// Share values differed in length.
    LengthMismatch,
    /// A numeric argument or share count is outside its documented range.
    Range,
    /// An internal field or packing invariant failed.
    Internal,
}

impl ShamirErrorCode {
    /// Stable `NEO_CRYPTO_*` code string, or `None` for range errors.
    #[must_use]
    pub const fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Random => "NEO_CRYPTO_RANDOM",
            Self::DuplicateIndex => "NEO_CRYPTO_SHAMIR_DUPLICATE",
            Self::LengthMismatch => "NEO_CRYPTO_SHAMIR_LENGTH",
            Self::Internal => "NEO_CRYPTO_INTERNAL",
            Self::Range => return None,
        })
    }

    /// Fixed human-readable message for this failure category.
    #[must_use]
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Limit => "input exceeds the Shamir size limit",
            Self::Random => "operating system random number generator failed",
            Self::DuplicateIndex => "duplicate share index",
            Self::LengthMismatch => "all shares must have the same length",
            Self::Range => "numeric argument is outside its documented range",
            Self::Internal => "the Shamir operation failed internally",
        }
    }
}

impl fmt::Display for ShamirErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str().unwrap_or("RANGE_ERROR"))
    }
}

/// Failure returned by Shamir secret sharing operations.
///
/// Messages are fixed per [`ShamirErrorCode`] and never echo secrets or
/// share bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("{}", .code.default_message())]
#[non_exhaustive]
pub struct ShamirError {
    code: ShamirErrorCode,
}

impl ShamirError {
    /// Create an error for `code`.
    #[must_use]
    pub const fn new(code: ShamirErrorCode) -> Self {
        Self { code }
    }

    /// The stable failure category.
    #[must_use]
    pub const fn code(self) -> ShamirErrorCode {
        self.code
    }

    /// Fixed message safe to expose to callers.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.code.default_message()
    }
}

/// One share of a split secret.
///
/// The share bytes are held in [`Zeroizing`] and cleared when dropped.
pub struct ShamirShare {
    index: u8,
    value: Zeroizing<Vec<u8>>,
}

impl ShamirShare {
    /// Construct a share from a one-based index and raw share bytes.
    #[must_use]
    pub fn new(index: u8, value: Vec<u8>) -> Self {
        Self {
            index,
            value: Zeroizing::new(value),
        }
    }

    /// Construct a share from a one-based index and an already-zeroizing buffer.
    #[must_use]
    pub fn from_zeroizing(index: u8, value: Zeroizing<Vec<u8>>) -> Self {
        Self { index, value }
    }

    /// One-based field evaluation index (`1..=255`).
    #[must_use]
    pub const fn index(&self) -> u8 {
        self.index
    }

    /// Borrow the share bytes.
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// Extract the share bytes without copying, preserving [`Zeroizing`].
    #[must_use]
    pub fn into_value(self) -> Zeroizing<Vec<u8>> {
        self.value
    }
}

impl fmt::Debug for ShamirShare {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShamirShare")
            .field("index", &self.index)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

#[cfg(feature = "test-support")]
static FAIL_NEXT_SHAMIR_RANDOM: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(crate) fn fail_next_shamir_random() {
    FAIL_NEXT_SHAMIR_RANDOM.store(true, Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_shamir_random_failure() -> bool {
    FAIL_NEXT_SHAMIR_RANDOM.swap(false, Ordering::SeqCst)
}

fn fill_random(bytes: &mut [u8]) -> Result<(), ShamirError> {
    #[cfg(feature = "test-support")]
    if take_shamir_random_failure() {
        return Err(ShamirError::new(ShamirErrorCode::Random));
    }
    getrandom::fill(bytes).map_err(|_| ShamirError::new(ShamirErrorCode::Random))
}

fn zeroed_vec(length: usize) -> Result<Vec<u8>, ShamirError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

/// Multiply a value length by a share count and enforce [`MAX_SHAMIR_TOTAL_BYTES`].
pub fn check_total(length: usize, count: usize) -> Result<usize, ShamirError> {
    let total = length
        .checked_mul(count)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    if total > MAX_SHAMIR_TOTAL_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    Ok(total)
}

/// Compute `(chunk_bytes, noise_length)` for one split's temporary noise buffer.
pub fn noise_size(secret_length: usize, threshold: usize) -> Result<(usize, usize), ShamirError> {
    if secret_length == 0 || !(2..=MAX_SHAMIR_SHARES).contains(&threshold) {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    let stride = threshold - 1;
    let chunk_bytes = (NOISE_BUFFER_BYTES / stride).max(1).min(secret_length);
    let noise_length = chunk_bytes
        .checked_mul(stride)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    Ok((chunk_bytes, noise_length))
}

/// Validate split parameters and return the total share bytes (`secret_length * share_count`).
pub fn validate_split(
    secret_length: usize,
    threshold: usize,
    share_count: usize,
) -> Result<usize, ShamirError> {
    if secret_length == 0
        || !(2..=MAX_SHAMIR_SHARES).contains(&threshold)
        || !(threshold..=MAX_SHAMIR_SHARES).contains(&share_count)
    {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    if secret_length > MAX_SHAMIR_SECRET_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    check_total(secret_length, share_count)
}

/// Validate a combine share set and return the secret length.
pub fn validate_combine(shares: &[ShamirShare]) -> Result<usize, ShamirError> {
    if shares.len() < 2 || shares.len() > MAX_SHAMIR_SHARES {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    let Some(first) = shares.first() else {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    };
    let length = first.value.len();
    if length == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    if length > MAX_SHAMIR_SECRET_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    check_total(length, shares.len())?;
    let mut seen = [false; 256];
    for share in shares {
        if share.index == 0 {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        let slot = usize::from(share.index);
        if seen[slot] {
            return Err(ShamirError::new(ShamirErrorCode::DuplicateIndex));
        }
        seen[slot] = true;
        if share.value.is_empty() {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        if share.value.len() != length {
            return Err(ShamirError::new(ShamirErrorCode::LengthMismatch));
        }
    }
    Ok(length)
}

/// Split `secret` into `share_count` shares so any `threshold` recover it.
///
/// # Example
///
/// ```
/// use neojs_crypto_core::shamir::{shamir_combine, shamir_split};
///
/// let shares = shamir_split(b"top secret", 3, 5)?;
/// let recovered = shamir_combine(&shares[1..4])?;
/// assert_eq!(&*recovered, b"top secret");
/// # Ok::<(), neojs_crypto_core::shamir::ShamirError>(())
/// ```
pub fn shamir_split(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
) -> Result<Vec<ShamirShare>, ShamirError> {
    split_with(secret, threshold, share_count, fill_random)
}

fn split_with<F>(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
    mut fill: F,
) -> Result<Vec<ShamirShare>, ShamirError>
where
    F: FnMut(&mut [u8]) -> Result<(), ShamirError>,
{
    validate_split(secret.len(), threshold, share_count)?;

    let mut shares = Vec::new();
    shares
        .try_reserve_exact(share_count)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    for position in 0..share_count {
        let index =
            u8::try_from(position + 1).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
        shares.push(ShamirShare::new(index, zeroed_vec(secret.len())?));
    }

    let stride = threshold - 1;
    let (chunk_bytes, noise_length) = noise_size(secret.len(), threshold)?;
    let mut noise = Zeroizing::new(zeroed_vec(noise_length)?);

    for start in (0..secret.len()).step_by(chunk_bytes) {
        let width = chunk_bytes.min(secret.len() - start);
        let used = width * stride;
        fill(&mut noise[..used])?;
        for offset in 0..width {
            let constant = secret[start + offset];
            let coefficients = &noise[offset * stride..(offset + 1) * stride];
            for share in &mut shares {
                share.value[start + offset] = evaluate(constant, coefficients, share.index);
            }
        }
    }
    Ok(shares)
}

/// Parsed view of a packed worker split payload produced by
/// [`shamir_split_packed`].
pub struct SplitPayload<'a> {
    /// Number of shares in the packed payload.
    pub count: usize,
    /// Secret / share byte length.
    pub length: usize,
    /// Packed body: `count` repeated `[index: u8, value: [u8; length]]` records.
    pub body: &'a [u8],
}

impl fmt::Debug for SplitPayload<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SplitPayload")
            .field("count", &self.count)
            .field("length", &self.length)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// Split directly into a single packed [`Zeroizing`] buffer for worker transfer.
///
/// Header layout: `count: u32` (LE), `length: u32` (LE), followed by `count`
/// records of `[index: u8, value: [u8; length]]`.
pub fn shamir_split_packed(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
) -> Result<Zeroizing<Vec<u8>>, ShamirError> {
    let total = validate_split(secret.len(), threshold, share_count)?;

    let stride = secret
        .len()
        .checked_add(1)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    let capacity = 8usize
        .checked_add(share_count)
        .and_then(|bytes| bytes.checked_add(total))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    let mut packed = Zeroizing::new(zeroed_vec(capacity)?);

    let count = u32::try_from(share_count).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
    let length =
        u32::try_from(secret.len()).map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    packed[0..4].copy_from_slice(&count.to_le_bytes());
    packed[4..8].copy_from_slice(&length.to_le_bytes());
    for position in 0..share_count {
        let index =
            u8::try_from(position + 1).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
        packed[8 + position * stride] = index;
    }

    let poly_stride = threshold - 1;
    let (chunk_bytes, noise_length) = noise_size(secret.len(), threshold)?;
    let mut noise = Zeroizing::new(zeroed_vec(noise_length)?);
    for start in (0..secret.len()).step_by(chunk_bytes) {
        let width = chunk_bytes.min(secret.len() - start);
        let used = width * poly_stride;
        fill_random(&mut noise[..used])?;
        for offset in 0..width {
            let constant = secret[start + offset];
            let coefficients = &noise[offset * poly_stride..(offset + 1) * poly_stride];
            for position in 0..share_count {
                let base = 8 + position * stride;
                let index = packed[base];
                let value = evaluate(constant, coefficients, index);
                packed[base + 1 + start + offset] = value;
            }
        }
    }
    Ok(packed)
}

/// Parse the header and body of a packed payload produced by
/// [`shamir_split_packed`].
pub fn parse_split_payload(bytes: &[u8]) -> Result<SplitPayload<'_>, ShamirError> {
    if bytes.len() < 8 {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    let count = u32::from_le_bytes(
        bytes[0..4]
            .try_into()
            .map_err(|_| ShamirError::new(ShamirErrorCode::Internal))?,
    ) as usize;
    let length = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| ShamirError::new(ShamirErrorCode::Internal))?,
    ) as usize;
    if !(2..=MAX_SHAMIR_SHARES).contains(&count) || length == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    if length > MAX_SHAMIR_SECRET_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    let total = count
        .checked_mul(length)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
    if total > MAX_SHAMIR_TOTAL_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    let expected = count
        .checked_mul(length)
        .and_then(|body| body.checked_add(count))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
    if bytes.len() != 8 + expected {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    Ok(SplitPayload {
        count,
        length,
        body: &bytes[8..],
    })
}

/// Recover the secret from `shares` with Lagrange interpolation at `x = 0`.
///
/// The returned buffer is wrapped in [`Zeroizing`].
pub fn shamir_combine(shares: &[ShamirShare]) -> Result<Zeroizing<Vec<u8>>, ShamirError> {
    let length = validate_combine(shares)?;
    let basis = compute_basis(shares)?;
    let mut secret = Zeroizing::new(zeroed_vec(length)?);
    for (byte_index, slot) in secret.as_mut_slice().iter_mut().enumerate() {
        let mut value = 0u8;
        for (share, coefficient) in shares.iter().zip(basis.iter()) {
            value ^= mul(share.value[byte_index], *coefficient);
        }
        *slot = value;
    }
    Ok(secret)
}

fn compute_basis(shares: &[ShamirShare]) -> Result<Vec<u8>, ShamirError> {
    let mut basis = zeroed_vec(shares.len())?;
    for (outer, share) in shares.iter().enumerate() {
        let xi = share.index;
        let mut coefficient = 1u8;
        for (inner, other) in shares.iter().enumerate() {
            if outer == inner {
                continue;
            }
            let denominator = xi ^ other.index;
            coefficient = mul(coefficient, div(other.index, denominator)?);
        }
        basis[outer] = coefficient;
    }
    Ok(basis)
}

fn evaluate(constant: u8, coefficients: &[u8], point: u8) -> u8 {
    let mut result = 0u8;
    for coefficient in coefficients.iter().rev() {
        result = mul(result, point) ^ coefficient;
    }
    mul(result, point) ^ constant
}

fn div(left: u8, right: u8) -> Result<u8, ShamirError> {
    if right == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    Ok(mul(left, inverse(right)))
}

fn inverse(value: u8) -> u8 {
    let mut result = 1u8;
    let mut factor = mul(value, value);
    for _ in 0..7 {
        result = mul(result, factor);
        factor = mul(factor, factor);
    }
    result
}

fn mul(left: u8, right: u8) -> u8 {
    let mut a = left;
    let mut b = right;
    let mut product = 0u8;
    for _ in 0..8 {
        let bit = b & 1;
        product ^= a & 0u8.wrapping_sub(bit);
        let carry = a >> 7;
        a = a.wrapping_shl(1);
        a ^= 0x1b & 0u8.wrapping_sub(carry);
        b >>= 1;
    }
    product
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    // Unit test: GF(256) multiplication and multiplicative inverse over all 255 nonzero elements.
    #[test]
    fn gf256_matches_known_products_and_inverses() {
        assert_eq!(mul(0, 5), 0);
        assert_eq!(mul(5, 0), 0);
        assert_eq!(mul(3, 3), 5);
        assert_eq!(mul(0x57, 0x83), 0xc1);
        assert_eq!(div(5, 3).unwrap(), 3);
        assert_eq!(div(0, 1).unwrap(), 0);
        assert!(div(1, 0).is_err());
        for value in 1..=255u8 {
            assert_eq!(mul(value, inverse(value)), 1, "inverse of {value}");
        }
    }

    // Unit test: codec exact-bytes interop vector matching PLAB encoding.
    #[test]
    fn independent_combine_vector_matches_the_plab_encoding() {
        let shares = [
            ShamirShare::new(1, vec![0x51]),
            ShamirShare::new(2, vec![0x64]),
        ];
        assert_eq!(&*shamir_combine(&shares).unwrap(), &[0x42]);
    }

    // Unit test: private deterministic-noise path not reachable through public API.
    #[test]
    fn independent_three_of_five_vector_matches() {
        let secret = b"shamir-interop";
        let noise = decode_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b");
        let expected = [
            (1u8, "7269606c68732c686f7564736e71"),
            (2, "77607d7d5d4a01490a1c19023b28"),
            (3, "76617c7c5c4b00480b1d18033a29"),
            (4, "63502105d9eacda125177e4184b3"),
            (5, "62512004d8ebcca024167f4085b2"),
        ];
        let mut position = 0usize;
        let shares = split_with(secret, 3, 5, |bytes| {
            let end = position + bytes.len();
            bytes.copy_from_slice(&noise[position..end]);
            position = end;
            Ok(())
        })
        .unwrap();
        for (share, (index, value)) in shares.iter().zip(expected) {
            assert_eq!(share.index(), index);
            assert_eq!(encode_hex(share.value()), value);
        }
        assert_eq!(&*shamir_combine(&shares).unwrap(), secret);
    }

    // Unit test: combinatorial threshold-subset reconstruction.
    #[test]
    fn every_threshold_subset_reconstructs() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let secret = b"shamir-interop";
        let shares = shamir_split(secret, 3, 5).unwrap();
        for first in 0..shares.len() {
            for second in first + 1..shares.len() {
                for third in second + 1..shares.len() {
                    let subset = [
                        ShamirShare::new(shares[first].index(), shares[first].value().to_vec()),
                        ShamirShare::new(shares[second].index(), shares[second].value().to_vec()),
                        ShamirShare::new(shares[third].index(), shares[third].value().to_vec()),
                    ];
                    assert_eq!(&*shamir_combine(&subset).unwrap(), secret);
                }
            }
        }
    }

    // Unit test: packed split codec roundtrip.
    #[test]
    fn packed_split_roundtrips_through_combine() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let secret = b"packed shamir secret";
        let packed = shamir_split_packed(secret, 3, 5).unwrap();
        let payload = parse_split_payload(&packed).unwrap();
        let dbg = format!("{payload:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert_eq!(payload.count, 5);
        assert_eq!(payload.length, secret.len());
        let stride = 1 + payload.length;
        let shares: Vec<ShamirShare> = (0..payload.count)
            .map(|i| {
                let record = &payload.body[i * stride..(i + 1) * stride];
                ShamirShare::new(record[0], record[1..].to_vec())
            })
            .collect();
        assert_eq!(&*shamir_combine(&shares[2..5]).unwrap(), secret);
    }

    // Unit test: bounds check for total product without allocating.
    #[test]
    fn total_bound_is_checked_without_allocating() {
        assert_eq!(
            check_total(MAX_SHAMIR_TOTAL_BYTES / 2 + 1, 2)
                .unwrap_err()
                .code(),
            ShamirErrorCode::Limit
        );
        assert_eq!(check_total(1, 255).unwrap(), 255);
        assert_eq!(
            check_total(MAX_SHAMIR_SECRET_BYTES, 2).unwrap(),
            MAX_SHAMIR_SECRET_BYTES * 2
        );
    }

    // Unit test: bounds check for invalid split shapes.
    #[test]
    fn split_rejects_invalid_shapes_without_allocating_output() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            shamir_split(b"", 2, 3).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            shamir_split(b"secret", 1, 3).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            shamir_split(b"secret", 3, 2).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            shamir_split(b"secret", 2, 256).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            shamir_split(&vec![0u8; MAX_SHAMIR_SECRET_BYTES + 1], 2, 2)
                .unwrap_err()
                .code(),
            ShamirErrorCode::Limit
        );
    }

    // Unit test: bounds check for noise_size and parse_split_payload secret length.
    #[test]
    fn noise_size_rejects_zero_secret_and_out_of_range_threshold() {
        assert_eq!(noise_size(0, 2).unwrap_err().code(), ShamirErrorCode::Range);
        assert_eq!(
            noise_size(16, 0).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            noise_size(16, 1).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            noise_size(16, MAX_SHAMIR_SHARES + 1).unwrap_err().code(),
            ShamirErrorCode::Range
        );

        let mut header = [0u8; 8];
        header[0..4].copy_from_slice(&2u32.to_le_bytes());
        header[4..8].copy_from_slice(&((MAX_SHAMIR_SECRET_BYTES + 1) as u32).to_le_bytes());
        assert_eq!(
            parse_split_payload(&header).unwrap_err().code(),
            ShamirErrorCode::Limit
        );
    }

    // Unit test: bounds and validation checks for combine share sets.
    #[test]
    fn combine_rejects_malformed_sets() {
        assert_eq!(
            shamir_combine(&[ShamirShare::new(1, b"x".to_vec())])
                .unwrap_err()
                .code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            shamir_combine(&[
                ShamirShare::new(1, b"x".to_vec()),
                ShamirShare::new(1, b"y".to_vec())
            ])
            .unwrap_err()
            .code(),
            ShamirErrorCode::DuplicateIndex
        );
        assert_eq!(
            shamir_combine(&[
                ShamirShare::new(1, b"x".to_vec()),
                ShamirShare::new(3, b"yy".to_vec())
            ])
            .unwrap_err()
            .code(),
            ShamirErrorCode::LengthMismatch
        );
        assert_eq!(
            shamir_combine(&[
                ShamirShare::new(0, b"x".to_vec()),
                ShamirShare::new(2, b"y".to_vec())
            ])
            .unwrap_err()
            .code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            shamir_combine(&[
                ShamirShare::new(1, Vec::new()),
                ShamirShare::new(2, Vec::new())
            ])
            .unwrap_err()
            .code(),
            ShamirErrorCode::Range
        );
    }

    // Unit test: exact error code strings and message invariants.
    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        for (code, expected) in [
            (ShamirErrorCode::Limit, Some("NEO_CRYPTO_LIMIT")),
            (ShamirErrorCode::Random, Some("NEO_CRYPTO_RANDOM")),
            (
                ShamirErrorCode::DuplicateIndex,
                Some("NEO_CRYPTO_SHAMIR_DUPLICATE"),
            ),
            (
                ShamirErrorCode::LengthMismatch,
                Some("NEO_CRYPTO_SHAMIR_LENGTH"),
            ),
            (ShamirErrorCode::Internal, Some("NEO_CRYPTO_INTERNAL")),
            (ShamirErrorCode::Range, None),
        ] {
            let err = ShamirError::new(code);
            assert_eq!(err.code().as_str(), expected);
            let message = err.message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
            assert_eq!(err.to_string(), message);
        }
    }

    // Unit test: test-support random failure injection.
    #[cfg(feature = "test-support")]
    #[test]
    fn fail_next_shamir_random_triggers_once() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        fail_next_shamir_random();
        assert_eq!(
            shamir_split(b"secret", 2, 3).unwrap_err().code(),
            ShamirErrorCode::Random
        );
        assert!(shamir_split(b"secret", 2, 3).is_ok());

        fail_next_shamir_random();
        assert_eq!(
            shamir_split_packed(b"secret", 2, 3).unwrap_err().code(),
            ShamirErrorCode::Random
        );
        assert!(shamir_split_packed(b"secret", 2, 3).is_ok());
    }
}
