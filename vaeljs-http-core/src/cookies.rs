//! Safe HTTP `Cookie` header parsing and `Set-Cookie` formatting.

use std::fmt;
use std::fmt::Write as _;

use crate::HttpCoreError;
use crate::policy::{DEFAULT_MAX_HEADER_BYTES, is_http_token_char, validate_header_value};

/// Default maximum number of cookies accepted from a single `Cookie` header.
pub const DEFAULT_MAX_COOKIES: usize = 128;

/// Cookie `SameSite` attribute policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SameSite {
    /// Sent on top-level cross-site navigations and same-site requests.
    Lax,
    /// Sent only on same-site requests.
    Strict,
    /// Sent on both same-site and cross-site requests (requires `Secure`).
    None,
}

impl SameSite {
    /// Returns the wire-format `SameSite` attribute value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lax => "Lax",
            Self::Strict => "Strict",
            Self::None => "None",
        }
    }
}

impl fmt::Display for SameSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Builder and representation for an HTTP `Set-Cookie` header value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetCookie {
    /// Cookie name (RFC 9110 `token`).
    pub name: String,
    /// Cookie value (RFC 6265 `cookie-octet` sequence).
    pub value: String,
    /// Optional `Path` attribute.
    pub path: Option<String>,
    /// Optional `Domain` attribute.
    pub domain: Option<String>,
    /// Optional `Max-Age` attribute in seconds.
    pub max_age: Option<i64>,
    /// Whether to emit the `HttpOnly` attribute.
    pub http_only: bool,
    /// Whether to emit the `Secure` attribute.
    pub secure: bool,
    /// Optional `SameSite` attribute.
    pub same_site: Option<SameSite>,
}

impl SetCookie {
    /// Creates a new [`SetCookie`] with the given `name` and `value` and all
    /// optional attributes unset.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            ..Self::default()
        }
    }

    /// Sets the `Path` attribute.
    #[must_use]
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Sets the `Domain` attribute.
    #[must_use]
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the `Max-Age` attribute in seconds.
    #[must_use]
    pub const fn max_age(mut self, seconds: i64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// Sets the `HttpOnly` attribute flag.
    #[must_use]
    pub const fn http_only(mut self, http_only: bool) -> Self {
        self.http_only = http_only;
        self
    }

    /// Sets the `Secure` attribute flag.
    #[must_use]
    pub const fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Sets the `SameSite` attribute.
    #[must_use]
    pub const fn same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = Some(same_site);
        self
    }

    /// Validates and formats this cookie as a `Set-Cookie` header value.
    ///
    /// # Errors
    ///
    /// Returns [`HttpCoreError::InvalidCookie`] if the cookie name, value,
    /// path, or domain contains invalid or header-injecting characters.
    pub fn format(&self) -> Result<String, HttpCoreError> {
        format_set_cookie(self)
    }
}

/// Parses an HTTP `Cookie` request header into `(name, value)` pairs, enforcing
/// [`DEFAULT_MAX_COOKIES`].
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidCookie`] if `header` contains control
/// characters (including CR/LF/NUL), malformed cookie pairs, invalid cookie
/// names, or more than [`DEFAULT_MAX_COOKIES`] cookies.
///
/// # Examples
///
/// ```
/// use vaeljs_http_core::parse_cookies;
///
/// let cookies = parse_cookies("session=abc123; theme=\"dark\"").unwrap();
/// assert_eq!(
///     cookies,
///     vec![
///         ("session".to_owned(), "abc123".to_owned()),
///         ("theme".to_owned(), "dark".to_owned()),
///     ]
/// );
/// assert!(parse_cookies("a=1\r\nX-Evil: 2").is_err());
/// ```
pub fn parse_cookies(header: &str) -> Result<Vec<(String, String)>, HttpCoreError> {
    parse_cookies_with_limit(header, DEFAULT_MAX_COOKIES)
}

/// Parses an HTTP `Cookie` request header into `(name, value)` pairs with an
/// explicit `max_cookies` upper bound and [`DEFAULT_MAX_HEADER_BYTES`] byte
/// bound.
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidCookie`] if `header` exceeds
/// [`DEFAULT_MAX_HEADER_BYTES`], contains control characters (including
/// CR/LF/NUL), has malformed cookie pairs, invalid cookie names, values
/// violating RFC 6265 `cookie-octet` rules, or more than `max_cookies` cookies.
pub fn parse_cookies_with_limit(
    header: &str,
    max_cookies: usize,
) -> Result<Vec<(String, String)>, HttpCoreError> {
    if header.len() > DEFAULT_MAX_HEADER_BYTES {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie header exceeds maximum header byte limit",
        });
    }
    validate_header_value(header).map_err(|_| HttpCoreError::InvalidCookie {
        reason: "cookie header contains disallowed control bytes",
    })?;

    let mut cookies = Vec::new();
    for raw_pair in header.split(';') {
        let pair = trim_ows(raw_pair);
        if pair.is_empty() {
            continue;
        }
        if cookies.len() >= max_cookies {
            return Err(HttpCoreError::InvalidCookie {
                reason: "cookie count exceeds maximum allowed limit",
            });
        }

        let Some((raw_name, raw_value)) = pair.split_once('=') else {
            return Err(HttpCoreError::InvalidCookie {
                reason: "cookie pair is missing '=' separator",
            });
        };
        let name = trim_ows(raw_name);
        validate_cookie_name(name)?;

        let mut value = trim_ows(raw_value);
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value = &value[1..value.len() - 1];
        }
        validate_cookie_value(value)?;

        cookies.push((name.to_owned(), value.to_owned()));
    }

    Ok(cookies)
}

/// Validates and formats a [`SetCookie`] into a safe `Set-Cookie` header value.
///
/// Strictly rejects CR, LF, NUL, control characters, semicolons, quotes,
/// commas, backslashes, whitespace, and non-ASCII bytes in cookie names and
/// values, rejects `SameSite=None` without `Secure`, enforces
/// [`DEFAULT_MAX_HEADER_BYTES`], and validates `Path` and `Domain` attributes.
///
/// # Errors
///
/// Returns [`HttpCoreError::InvalidCookie`] if any field fails validation or
/// the formatted header exceeds [`DEFAULT_MAX_HEADER_BYTES`].
///
/// # Examples
///
/// ```
/// use vaeljs_http_core::{SameSite, SetCookie, format_set_cookie};
///
/// let cookie = SetCookie::new("sid", "xyz789")
///     .path("/")
///     .max_age(3600)
///     .http_only(true)
///     .secure(true)
///     .same_site(SameSite::Lax);
///
/// assert_eq!(
///     format_set_cookie(&cookie).unwrap(),
///     "sid=xyz789; Path=/; Max-Age=3600; HttpOnly; Secure; SameSite=Lax"
/// );
///
/// let injected = SetCookie::new("sid", "ok\r\nSet-Cookie: admin=1");
/// assert!(format_set_cookie(&injected).is_err());
/// ```
pub fn format_set_cookie(cookie: &SetCookie) -> Result<String, HttpCoreError> {
    validate_cookie_name(&cookie.name)?;
    validate_cookie_value(&cookie.value)?;

    if let Some(path) = &cookie.path {
        validate_cookie_path(path)?;
    }
    if let Some(domain) = &cookie.domain {
        validate_cookie_domain(domain)?;
    }
    if cookie.same_site == Some(SameSite::None) && !cookie.secure {
        return Err(HttpCoreError::InvalidCookie {
            reason: "SameSite=None cookies must set the Secure attribute",
        });
    }

    let base_len = cookie
        .name
        .len()
        .saturating_add(cookie.value.len())
        .saturating_add(cookie.path.as_ref().map_or(0, String::len))
        .saturating_add(cookie.domain.as_ref().map_or(0, String::len));
    if base_len > DEFAULT_MAX_HEADER_BYTES {
        return Err(HttpCoreError::InvalidCookie {
            reason: "Set-Cookie header exceeds maximum header byte limit",
        });
    }

    let mut out = String::with_capacity(base_len.saturating_add(64));
    out.push_str(&cookie.name);
    out.push('=');
    out.push_str(&cookie.value);

    if let Some(path) = &cookie.path {
        out.push_str("; Path=");
        out.push_str(path);
    }
    if let Some(domain) = &cookie.domain {
        out.push_str("; Domain=");
        out.push_str(domain);
    }
    if let Some(max_age) = cookie.max_age {
        let _ = write!(out, "; Max-Age={max_age}");
    }
    if cookie.http_only {
        out.push_str("; HttpOnly");
    }
    if cookie.secure {
        out.push_str("; Secure");
    }
    if let Some(same_site) = cookie.same_site {
        out.push_str("; SameSite=");
        out.push_str(same_site.as_str());
    }

    if out.len() > DEFAULT_MAX_HEADER_BYTES {
        return Err(HttpCoreError::InvalidCookie {
            reason: "Set-Cookie header exceeds maximum header byte limit",
        });
    }

    Ok(out)
}

fn trim_ows(value: &str) -> &str {
    value.trim_matches(|c| c == ' ' || c == '\t')
}

fn validate_cookie_name(name: &str) -> Result<(), HttpCoreError> {
    if name.is_empty() {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie name must not be empty",
        });
    }
    if !name.bytes().all(is_http_token_char) {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie name contains invalid token characters",
        });
    }
    Ok(())
}

fn validate_cookie_value(value: &str) -> Result<(), HttpCoreError> {
    let valid = value.bytes().all(|b| {
        matches!(
            b,
            0x21 | 0x23..=0x2B | 0x2D..=0x3A | 0x3C..=0x5B | 0x5D..=0x7E
        )
    });
    if !valid {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie value contains disallowed or control characters",
        });
    }
    Ok(())
}

fn validate_cookie_path(path: &str) -> Result<(), HttpCoreError> {
    if path.is_empty() {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie Path attribute must not be empty",
        });
    }
    if !path.bytes().all(|b| (0x20..0x7F).contains(&b) && b != b';') {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie Path attribute contains disallowed characters",
        });
    }
    Ok(())
}

fn validate_cookie_domain(domain: &str) -> Result<(), HttpCoreError> {
    let normalized = domain.strip_prefix('.').unwrap_or(domain);
    if normalized.is_empty() || normalized.len() > 253 {
        return Err(HttpCoreError::InvalidCookie {
            reason: "cookie Domain attribute length is invalid",
        });
    }

    for label in normalized.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(HttpCoreError::InvalidCookie {
                reason: "cookie Domain attribute contains invalid DNS labels",
            });
        }
    }
    Ok(())
}
