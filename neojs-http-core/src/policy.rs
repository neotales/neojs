//! Bounded HTTP policy defaults, header validation, and shared byte helpers.

use crate::HttpCoreError;

/// Default maximum request body size in bytes (10 MiB).
pub const DEFAULT_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Default maximum total HTTP header section size in bytes (64 KiB).
pub const DEFAULT_MAX_HEADER_BYTES: usize = 64 * 1024;

/// Returns `true` if `byte` is an RFC 9110 HTTP token character (`tchar`).
#[must_use]
pub(crate) const fn is_http_token_char(byte: u8) -> bool {
    matches!(
        byte,
        b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
            | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

/// Validates an HTTP header field name against RFC 9110 `token` syntax.
///
/// Rejects empty names, whitespace, CR/LF/NUL, control characters, delimiters,
/// and non-ASCII bytes.
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidHeader`] if `name` is empty or contains any
/// byte outside the RFC 9110 `tchar` set.
///
/// # Examples
///
/// ```
/// use neojs_http_core::validate_header_name;
///
/// assert!(validate_header_name("Content-Type").is_ok());
/// assert!(validate_header_name("x-request-id").is_ok());
/// assert!(validate_header_name("").is_err());
/// assert!(validate_header_name("Bad Header").is_err());
/// assert!(validate_header_name("Bad\r\nHeader").is_err());
/// ```
pub fn validate_header_name(name: &str) -> Result<(), HttpCoreError> {
    if name.is_empty() {
        return Err(HttpCoreError::InvalidHeader {
            reason: "header name must not be empty",
        });
    }
    if !name.bytes().all(is_http_token_char) {
        return Err(HttpCoreError::InvalidHeader {
            reason: "header name contains invalid token bytes",
        });
    }
    Ok(())
}

/// Validates an HTTP header field value against RFC 9110 `field-value` rules.
///
/// Rejects CR (`\r`), LF (`\n`), NUL (`\0`), DEL (`0x7F`), and all other ASCII
/// control characters except horizontal tab (`\t`).
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidHeader`] if `value` contains CR, LF, NUL,
/// DEL, or any disallowed control byte.
///
/// # Examples
///
/// ```
/// use neojs_http_core::validate_header_value;
///
/// assert!(validate_header_value("application/json; charset=utf-8").is_ok());
/// assert!(validate_header_value("").is_ok());
/// assert!(validate_header_value("ok\r\nEvil: injected").is_err());
/// assert!(validate_header_value("null\0byte").is_err());
/// ```
pub fn validate_header_value(value: &str) -> Result<(), HttpCoreError> {
    for byte in value.bytes() {
        if byte == b'\r' || byte == b'\n' || byte == 0x00 {
            return Err(HttpCoreError::InvalidHeader {
                reason: "header value must not contain CR, LF, or NUL",
            });
        }
        if (byte < 0x20 && byte != b'\t') || byte == 0x7F {
            return Err(HttpCoreError::InvalidHeader {
                reason: "header value contains disallowed control bytes",
            });
        }
    }
    Ok(())
}

/// Percent-decodes a single URL path segment as strict UTF-8 without NUL bytes.
///
/// Leaves `+` untouched because `+` is a literal character in URL paths.
pub(crate) fn percent_decode_segment(segment: &str) -> Result<String, &'static str> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == 0 {
            return Err("path segment contains a NUL byte");
        }
        if byte == b'%' {
            if index + 2 >= bytes.len() {
                return Err("path segment contains a truncated percent escape");
            }
            let hi = hex_value(bytes[index + 1])
                .ok_or("path segment contains an invalid percent escape")?;
            let lo = hex_value(bytes[index + 2])
                .ok_or("path segment contains an invalid percent escape")?;
            let decoded = (hi << 4) | lo;
            if decoded == 0 {
                return Err("path segment contains a percent-encoded NUL byte");
            }
            out.push(decoded);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }

    String::from_utf8(out).map_err(|_| "path segment is not valid UTF-8")
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
