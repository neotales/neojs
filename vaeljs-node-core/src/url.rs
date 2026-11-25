//! `file:` URL to filesystem path conversions backing `node:url`.
//!
//! Enforces strict scheme, host, NUL, and percent-encoded separator checks so
//! `file:` URLs cannot smuggle remote hosts on POSIX or path separators via
//! `%2f`/`%5c`.

use std::path::{Path, PathBuf};

use url::Url;

use crate::error::NodeCoreError;

/// Maximum input byte length accepted by [`file_url_to_path`],
/// [`path_to_file_url`], [`parse_url`], and [`update_url`] (`64 KiB`).
pub const MAX_URL_BYTES: usize = 64 * 1024;

/// Parsed WHATWG URL components backing `node:url`'s `URL` class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedUrl {
    /// Complete serialized URL (`url.href`).
    pub href: String,
    /// Serialized origin (`url.origin`).
    pub origin: String,
    /// Scheme including trailing colon (`"https:"`).
    pub protocol: String,
    /// Percent-encoded username (`url.username`).
    pub username: String,
    /// Percent-encoded password (`url.password`).
    pub password: String,
    /// Host and optional port (`"example.com:8080"` or `"example.com"`).
    pub host: String,
    /// Hostname without port (`"example.com"`).
    pub hostname: String,
    /// Decimal port string or `""`.
    pub port: String,
    /// Path component (`"/a/b"`).
    pub pathname: String,
    /// Query string including leading `"?"`, or `""`.
    pub search: String,
    /// Fragment including leading `"#"`, or `""`.
    pub hash: String,
}

/// Mutable URL component selector for [`update_url`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlComponent {
    /// Full URL string (`href`).
    Href,
    /// Scheme (`protocol`).
    Protocol,
    /// Username (`username`).
    Username,
    /// Password (`password`).
    Password,
    /// Host and optional port (`host`).
    Host,
    /// Hostname without port (`hostname`).
    Hostname,
    /// Port (`port`).
    Port,
    /// Path (`pathname`).
    Pathname,
    /// Query (`search`).
    Search,
    /// Fragment (`hash`).
    Hash,
}

impl UrlComponent {
    /// Parses a JS property name into a [`UrlComponent`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "href" => Some(Self::Href),
            "protocol" => Some(Self::Protocol),
            "username" => Some(Self::Username),
            "password" => Some(Self::Password),
            "host" => Some(Self::Host),
            "hostname" => Some(Self::Hostname),
            "port" => Some(Self::Port),
            "pathname" => Some(Self::Pathname),
            "search" => Some(Self::Search),
            "hash" => Some(Self::Hash),
            _ => None,
        }
    }
}

fn to_parsed_url(parsed: &Url) -> ParsedUrl {
    let hostname = parsed.host_str().unwrap_or("").to_owned();
    let port = parsed.port().map(|p| p.to_string()).unwrap_or_default();
    let host = if port.is_empty() {
        hostname.clone()
    } else {
        format!("{hostname}:{port}")
    };
    let search = match parsed.query() {
        Some(q) if !q.is_empty() => format!("?{q}"),
        _ => String::new(),
    };
    let hash = match parsed.fragment() {
        Some(f) if !f.is_empty() => format!("#{f}"),
        _ => String::new(),
    };
    ParsedUrl {
        href: parsed.as_str().to_owned(),
        origin: parsed.origin().ascii_serialization(),
        protocol: format!("{}:", parsed.scheme()),
        username: parsed.username().to_owned(),
        password: parsed.password().unwrap_or("").to_owned(),
        host,
        hostname,
        port,
        pathname: parsed.path().to_owned(),
        search,
        hash,
    }
}

fn validate_url_input(value: &str) -> Result<(), NodeCoreError> {
    if value.len() > MAX_URL_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_URL_BYTES,
            actual: Some(value.len()),
        });
    }
    if value.as_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "URL input must not contain a NUL byte",
        });
    }
    Ok(())
}

/// Parses `input` (optionally resolved against `base`) into a [`ParsedUrl`].
pub fn parse_url(input: &str, base: Option<&str>) -> Result<ParsedUrl, NodeCoreError> {
    validate_url_input(input)?;
    let parsed = if let Some(base_str) = base {
        validate_url_input(base_str)?;
        let base_url = Url::parse(base_str).map_err(|_| NodeCoreError::InvalidUrl {
            reason: "invalid base URL",
        })?;
        base_url
            .join(input)
            .map_err(|_| NodeCoreError::InvalidUrl {
                reason: "invalid URL",
            })?
    } else {
        Url::parse(input).map_err(|_| NodeCoreError::InvalidUrl {
            reason: "invalid URL",
        })?
    };
    Ok(to_parsed_url(&parsed))
}

/// Updates one component on `href` and returns the re-serialized [`ParsedUrl`].
pub fn update_url(
    href: &str,
    component: UrlComponent,
    value: &str,
) -> Result<ParsedUrl, NodeCoreError> {
    validate_url_input(href)?;
    validate_url_input(value)?;
    if component == UrlComponent::Href {
        return parse_url(value, None);
    }
    let mut parsed = Url::parse(href).map_err(|_| NodeCoreError::InvalidUrl {
        reason: "invalid URL",
    })?;
    match component {
        UrlComponent::Href => unreachable!(),
        UrlComponent::Protocol => {
            let scheme = value.strip_suffix(':').unwrap_or(value);
            let _ = parsed.set_scheme(scheme);
        }
        UrlComponent::Username => {
            let _ = parsed.set_username(value);
        }
        UrlComponent::Password => {
            let pw = if value.is_empty() { None } else { Some(value) };
            let _ = parsed.set_password(pw);
        }
        UrlComponent::Host => {
            if let Some(colon) = value.rfind(':')
                && !value[colon + 1..].contains(']')
            {
                let host_part = &value[..colon];
                let port_part = &value[colon + 1..];
                if parsed.set_host(Some(host_part)).is_ok() {
                    if port_part.is_empty() {
                        let _ = parsed.set_port(None);
                    } else if let Ok(p) = port_part.parse::<u16>() {
                        let _ = parsed.set_port(Some(p));
                    }
                }
            } else {
                let _ = parsed.set_host(Some(value));
                let _ = parsed.set_port(None);
            }
        }
        UrlComponent::Hostname => {
            let host_opt = if value.is_empty() { None } else { Some(value) };
            let _ = parsed.set_host(host_opt);
        }
        UrlComponent::Port => {
            if value.is_empty() {
                let _ = parsed.set_port(None);
            } else if let Ok(p) = value.parse::<u16>() {
                let _ = parsed.set_port(Some(p));
            }
        }
        UrlComponent::Pathname => {
            parsed.set_path(value);
        }
        UrlComponent::Search => {
            let stripped = value.strip_prefix('?').unwrap_or(value);
            if stripped.is_empty() {
                parsed.set_query(None);
            } else {
                parsed.set_query(Some(stripped));
            }
        }
        UrlComponent::Hash => {
            let stripped = value.strip_prefix('#').unwrap_or(value);
            if stripped.is_empty() {
                parsed.set_fragment(None);
            } else {
                parsed.set_fragment(Some(stripped));
            }
        }
    }
    Ok(to_parsed_url(&parsed))
}

/// Converts a `file:` URL string into a local [`PathBuf`].
///
/// Rejects:
/// * empty strings, strings exceeding [`MAX_URL_BYTES`], or strings with NUL (`\0` / `%00`)
/// * non-`file:` URLs
/// * non-local hosts on POSIX (anything other than empty or `"localhost"`)
/// * percent-encoded path separators (`%2f`, `%2F`, `%5c`, `%5C`) in the URL path
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use vaeljs_node_core::url::{file_url_to_path, path_to_file_url};
///
/// if cfg!(windows) {
///     let p = file_url_to_path("file:///C:/hello%20world.txt").unwrap();
///     assert_eq!(p, Path::new("C:\\hello world.txt"));
/// } else {
///     let p = file_url_to_path("file:///tmp/hello%20world.txt").unwrap();
///     assert_eq!(p, Path::new("/tmp/hello world.txt"));
///     assert_eq!(path_to_file_url(&p).unwrap(), "file:///tmp/hello%20world.txt");
/// }
/// ```
pub fn file_url_to_path(url_str: &str) -> Result<PathBuf, NodeCoreError> {
    if url_str.is_empty() {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must not be empty",
        });
    }
    if url_str.len() > MAX_URL_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_URL_BYTES,
            actual: Some(url_str.len()),
        });
    }
    if url_str.as_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must not contain a NUL byte",
        });
    }

    let parsed = Url::parse(url_str).map_err(|_| NodeCoreError::InvalidUrl {
        reason: "malformed URL",
    })?;

    if parsed.scheme() != "file" {
        return Err(NodeCoreError::InvalidUrl {
            reason: "URL scheme must be 'file:'",
        });
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must not contain userinfo credentials",
        });
    }

    let raw_path = parsed.path();
    if contains_disallowed_percent_escape(raw_path) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL path must not contain encoded NUL or path separators",
        });
    }
    if is_bare_drive_url_path(raw_path) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL with a Windows drive letter must include a path separator after ':'",
        });
    }

    #[cfg(not(windows))]
    {
        if let Some(host) = parsed.host_str()
            && !host.is_empty()
            && !host.eq_ignore_ascii_case("localhost")
        {
            return Err(NodeCoreError::InvalidUrl {
                reason: "file URL host must be empty or localhost on POSIX",
            });
        }
    }

    let path = parsed
        .to_file_path()
        .map_err(|()| NodeCoreError::InvalidUrl {
            reason: "file URL could not be converted to a local filesystem path",
        })?;

    if !path.is_absolute() {
        return Err(NodeCoreError::InvalidUrl {
            reason: "file URL must resolve to an absolute local path",
        });
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidUrl {
            reason: "decoded file path must not contain a NUL byte",
        });
    }

    Ok(path)
}

/// Converts an absolute local [`Path`] into a canonical `file:` URL string.
///
/// Rejects empty paths, relative paths, paths containing NUL bytes, and paths
/// exceeding [`MAX_URL_BYTES`].
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use vaeljs_node_core::url::path_to_file_url;
///
/// if cfg!(windows) {
///     assert_eq!(path_to_file_url(Path::new("C:\\a b\\c.txt")).unwrap(), "file:///C:/a%20b/c.txt");
/// } else {
///     assert_eq!(path_to_file_url(Path::new("/a b/c.txt")).unwrap(), "file:///a%20b/c.txt");
/// }
/// ```
pub fn path_to_file_url(path: impl AsRef<Path>) -> Result<String, NodeCoreError> {
    let path = path.as_ref();
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() {
        return Err(NodeCoreError::InvalidPath {
            reason: "path must not be empty",
        });
    }
    if bytes.len() > MAX_URL_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_URL_BYTES,
            actual: Some(bytes.len()),
        });
    }
    if bytes.contains(&0) {
        return Err(NodeCoreError::InvalidPath {
            reason: "path must not contain a NUL byte",
        });
    }
    if !path.is_absolute() {
        return Err(NodeCoreError::InvalidPath {
            reason: "path_to_file_url requires an absolute path",
        });
    }

    let url = Url::from_file_path(path).map_err(|()| NodeCoreError::InvalidPath {
        reason: "path could not be represented as a file URL",
    })?;
    Ok(url.into())
}

fn is_bare_drive_url_path(raw_path: &str) -> bool {
    let bytes = raw_path.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'/' || !bytes[1].is_ascii_alphabetic() {
        return false;
    }
    if bytes[2] == b':' {
        return bytes.get(3).copied() != Some(b'/');
    }
    if bytes.len() >= 5
        && bytes[2] == b'%'
        && bytes[3] == b'3'
        && bytes[4].eq_ignore_ascii_case(&b'a')
    {
        return bytes.get(5).copied() != Some(b'/');
    }
    false
}

fn contains_disallowed_percent_escape(raw_path: &str) -> bool {
    let bytes = raw_path.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes[i + 1].to_ascii_lowercase();
            let lo = bytes[i + 2].to_ascii_lowercase();
            if matches!((hi, lo), (b'0', b'0') | (b'2', b'f') | (b'5', b'c')) {
                return true;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NodeCoreErrorCode;

    #[test]
    fn rejects_non_file_schemes_and_encoded_separators() {
        assert_eq!(
            file_url_to_path("https://example.com/a")
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%2fb").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%2Fb").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%5cb").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///tmp/a%00b").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///C:").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///c:rel").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            file_url_to_path("file:///C%3A").unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl
        );
        assert_eq!(
            path_to_file_url("relative/file.txt").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn posix_accepts_localhost_and_rejects_remote_host() {
        assert_eq!(
            file_url_to_path("file://localhost/tmp/test.txt").unwrap(),
            Path::new("/tmp/test.txt")
        );
        assert_eq!(
            file_url_to_path("file://remote.host/tmp/test.txt")
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidUrl
        );
    }
}
