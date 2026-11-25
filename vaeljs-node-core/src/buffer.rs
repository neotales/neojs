//! Bounded binary/text encoding and decoding backing `node:buffer`.
//!
//! Supports `utf8`, `hex`, `base64`, `base64url`, `latin1`, and `ascii`
//! encodings with explicit input length bounds to prevent unbounded allocations.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{
    GeneralPurpose, GeneralPurposeConfig, STANDARD, URL_SAFE_NO_PAD,
};

use crate::error::NodeCoreError;

/// Maximum input byte length accepted by [`decode`] (`16 MiB`).
pub const MAX_DECODE_INPUT_BYTES: usize = 16 * 1024 * 1024;

const BASE64_LENIENT_PAD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

const BASE64URL_LENIENT_PAD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Supported buffer encodings matching `node:buffer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// UTF-8 text (`"utf8"` / `"utf-8"`).
    Utf8,
    /// Lowercase hexadecimal (`"hex"`).
    Hex,
    /// Standard Base64 (`"base64"`).
    Base64,
    /// URL-safe Base64 without padding (`"base64url"`).
    Base64Url,
    /// ISO-8859-1 / Latin-1 single-byte characters (`"latin1"` / `"binary"`).
    Latin1,
    /// 7-bit ASCII (`"ascii"`).
    Ascii,
}

/// Encodes raw bytes into a [`String`] using `enc`.
///
/// * [`Encoding::Utf8`] replaces invalid UTF-8 sequences with `U+FFFD`.
/// * [`Encoding::Hex`] emits lowercase hex pairs.
/// * [`Encoding::Base64`] emits standard Base64 with `=` padding.
/// * [`Encoding::Base64Url`] emits URL-safe Base64 without padding.
/// * [`Encoding::Latin1`] maps each byte `0x00..=0xFF` to Unicode `U+0000..=U+00FF`.
/// * [`Encoding::Ascii`] masks each byte with `0x7F`, matching Node.js `Buffer.toString('ascii')`.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::buffer::{Encoding, decode, encode};
///
/// let encoded = encode(b"hello?", Encoding::Base64Url);
/// assert_eq!(encoded, "aGVsbG8_");
/// assert_eq!(decode(&encoded, Encoding::Base64Url).unwrap(), b"hello?");
/// ```
#[must_use]
pub fn encode(bytes: &[u8], enc: Encoding) -> String {
    match enc {
        Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        Encoding::Hex => {
            let mut out = String::with_capacity(bytes.len() * 2);
            for &b in bytes {
                let _ = write!(out, "{b:02x}");
            }
            out
        }
        Encoding::Base64 => STANDARD.encode(bytes),
        Encoding::Base64Url => URL_SAFE_NO_PAD.encode(bytes),
        Encoding::Latin1 => {
            let mut out = String::with_capacity(bytes.len());
            for &b in bytes {
                out.push(char::from(b));
            }
            out
        }
        Encoding::Ascii => {
            let mut out = String::with_capacity(bytes.len());
            for &b in bytes {
                out.push(char::from(b & 0x7f));
            }
            out
        }
    }
}

/// Decodes `input` into raw bytes according to `enc`.
///
/// Enforces [`MAX_DECODE_INPUT_BYTES`] and fails closed on invalid hex,
/// malformed Base64/Base64Url, code points above `U+00FF` for [`Encoding::Latin1`],
/// or non-ASCII characters for [`Encoding::Ascii`].
///
/// # Example
///
/// ```
/// use vaeljs_node_core::buffer::{Encoding, decode, encode};
///
/// let bytes = decode("deadbeef", Encoding::Hex).unwrap();
/// assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);
/// assert_eq!(encode(&bytes, Encoding::Hex), "deadbeef");
/// ```
pub fn decode(input: &str, enc: Encoding) -> Result<Vec<u8>, NodeCoreError> {
    if input.len() > MAX_DECODE_INPUT_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_DECODE_INPUT_BYTES,
            actual: Some(input.len()),
        });
    }

    match enc {
        Encoding::Utf8 => Ok(input.as_bytes().to_vec()),
        Encoding::Hex => decode_hex(input),
        Encoding::Base64 => BASE64_LENIENT_PAD.decode(input.as_bytes()).map_err(|_| {
            NodeCoreError::InvalidEncoding {
                reason: "invalid base64 input",
            }
        }),
        Encoding::Base64Url => BASE64URL_LENIENT_PAD.decode(input.as_bytes()).map_err(|_| {
            NodeCoreError::InvalidEncoding {
                reason: "invalid base64url input",
            }
        }),
        Encoding::Latin1 => {
            let mut out = Vec::with_capacity(input.chars().count());
            for ch in input.chars() {
                let code = u32::from(ch);
                if code > 0xff {
                    return Err(NodeCoreError::InvalidEncoding {
                        reason: "latin1 input contains code point above U+00FF",
                    });
                }
                out.push(code as u8);
            }
            Ok(out)
        }
        Encoding::Ascii => {
            if !input.is_ascii() {
                return Err(NodeCoreError::InvalidEncoding {
                    reason: "ascii input contains non-ASCII character",
                });
            }
            Ok(input.as_bytes().to_vec())
        }
    }
}

fn decode_hex(input: &str) -> Result<Vec<u8>, NodeCoreError> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(NodeCoreError::InvalidEncoding {
            reason: "hex input must have an even number of digits",
        });
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let (pairs, _) = bytes.as_chunks::<2>();
    for pair in pairs {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, NodeCoreError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(NodeCoreError::InvalidEncoding {
            reason: "hex input contains non-hexadecimal character",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NodeCoreErrorCode;

    #[test]
    fn round_trips_all_encodings() {
        let all_bytes: Vec<u8> = (0..=255).collect();
        for enc in [
            Encoding::Hex,
            Encoding::Base64,
            Encoding::Base64Url,
            Encoding::Latin1,
        ] {
            let encoded = encode(&all_bytes, enc);
            let decoded = decode(&encoded, enc).unwrap();
            assert_eq!(decoded, all_bytes, "round-trip failed for {enc:?}");
        }

        let ascii_bytes: Vec<u8> = (0..=127).collect();
        let ascii_str = encode(&ascii_bytes, Encoding::Ascii);
        assert_eq!(decode(&ascii_str, Encoding::Ascii).unwrap(), ascii_bytes);

        let utf8 = "vaeljs 🚀 node:buffer";
        let utf8_bytes = decode(utf8, Encoding::Utf8).unwrap();
        assert_eq!(encode(&utf8_bytes, Encoding::Utf8), utf8);
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert_eq!(
            decode("abc", Encoding::Hex).unwrap_err().code(),
            NodeCoreErrorCode::InvalidEncoding
        );
        assert_eq!(
            decode("zzzz", Encoding::Hex).unwrap_err().code(),
            NodeCoreErrorCode::InvalidEncoding
        );
        assert_eq!(
            decode("!!!", Encoding::Base64).unwrap_err().code(),
            NodeCoreErrorCode::InvalidEncoding
        );
        assert_eq!(
            decode("a+b/", Encoding::Base64Url).unwrap_err().code(),
            NodeCoreErrorCode::InvalidEncoding
        );
        assert_eq!(
            decode("€", Encoding::Latin1).unwrap_err().code(),
            NodeCoreErrorCode::InvalidEncoding
        );
        assert_eq!(
            decode("é", Encoding::Ascii).unwrap_err().code(),
            NodeCoreErrorCode::InvalidEncoding
        );
    }
}
