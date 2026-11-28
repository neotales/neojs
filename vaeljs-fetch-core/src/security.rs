//! SSRF guard, URL/header/method/token validation, redaction, and digest helpers.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use url::{Host, Url};

use crate::error::FetchError;
use crate::types::{
    MAX_ALLOWED_DOWNLOAD_BYTES, MAX_ALLOWED_REDIRECTS, MAX_ALLOWED_RESPONSE_BYTES, MAX_DNS_TIMEOUT,
    MAX_TIMEOUT,
};

/// Allowed HTTP methods for `fetch_sync`.
const ALLOWED_METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

/// Returns `true` if `name` carries credentials or session cookies that must be
/// redacted in debug output and stripped on cross-origin redirects.
#[must_use]
pub(crate) fn is_sensitive_header_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("set-cookie")
}

/// Returns `true` if `name` must be stripped when a redirect changes origin.
#[must_use]
pub(crate) fn is_cross_origin_stripped_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("cookie")
}

/// Returns `true` if `byte` is an RFC 7230 / RFC 9110 HTTP token byte (`tchar`).
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

/// Validates and uppercases an HTTP method name.
pub(crate) fn normalize_method(method: &str) -> Result<String, FetchError> {
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(FetchError::InvalidOption {
            reason: "HTTP method must be a supported ASCII method name without control bytes",
        });
    }
    let upper = method.to_ascii_uppercase();
    if ALLOWED_METHODS.contains(&upper.as_str()) {
        Ok(upper)
    } else {
        Err(FetchError::InvalidOption {
            reason: "HTTP method is not one of GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS",
        })
    }
}

/// Validates an HTTP header field name against RFC 7230 token rules.
pub(crate) fn validate_header_name(name: &str) -> Result<(), FetchError> {
    if name.is_empty() || !name.bytes().all(is_http_token_char) {
        return Err(FetchError::InvalidOption {
            reason: "header name must be a non-empty RFC 7230 token",
        });
    }
    Ok(())
}

/// Validates an HTTP header field value (no `\r`, `\n`, `\0`, or control bytes).
pub(crate) fn validate_header_value(value: &str) -> Result<(), FetchError> {
    for byte in value.bytes() {
        if byte == b'\r' || byte == b'\n' || byte == 0 {
            return Err(FetchError::InvalidOption {
                reason: "header value must not contain CR, LF, or NUL",
            });
        }
        if (byte < 0x20 && byte != b'\t') || byte == 0x7F {
            return Err(FetchError::InvalidOption {
                reason: "header value must not contain ASCII control characters",
            });
        }
    }
    Ok(())
}

/// Validates a bearer token (non-empty, no `\r`, `\n`, `\0`, or ASCII control bytes).
pub(crate) fn validate_bearer_token(token: &str) -> Result<(), FetchError> {
    if token.is_empty() {
        return Err(FetchError::InvalidOption {
            reason: "bearer_token must not be empty",
        });
    }
    for byte in token.bytes() {
        if byte == b'\r' || byte == b'\n' || byte == 0 || byte < 0x20 || byte == 0x7F {
            return Err(FetchError::InvalidOption {
                reason: "bearer_token must not contain CR, LF, NUL, or control characters",
            });
        }
    }
    Ok(())
}

/// Validates a `User-Agent` header value.
pub(crate) fn validate_user_agent(user_agent: &str) -> Result<(), FetchError> {
    validate_header_value(user_agent)
}

/// Validates request timeout (`> 0` and `<= 600s`).
pub(crate) fn validate_timeout(timeout: Duration) -> Result<Instant, FetchError> {
    if timeout.is_zero() {
        return Err(FetchError::InvalidOption {
            reason: "timeout must be greater than zero",
        });
    }
    if timeout > MAX_TIMEOUT {
        return Err(FetchError::InvalidOption {
            reason: "timeout must not exceed 600 seconds",
        });
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or(FetchError::InvalidOption {
            reason: "timeout overflows Instant",
        })
}

/// Validates `max_response_bytes` (`> 0` and `<= 256 MiB`).
pub(crate) fn validate_max_response_bytes(limit: usize) -> Result<(), FetchError> {
    if limit == 0 || limit > MAX_ALLOWED_RESPONSE_BYTES {
        return Err(FetchError::InvalidOption {
            reason: "max_response_bytes must be between 1 byte and 256 MiB",
        });
    }
    Ok(())
}

/// Validates `max_redirects` (`<= 20`).
pub(crate) fn validate_max_redirects(limit: usize) -> Result<(), FetchError> {
    if limit > MAX_ALLOWED_REDIRECTS {
        return Err(FetchError::InvalidOption {
            reason: "max_redirects must not exceed 20",
        });
    }
    Ok(())
}

/// Validates `max_bytes` for streaming downloads (`> 0` and `<= 4 GiB`).
pub(crate) fn validate_download_max_bytes(limit: u64) -> Result<(), FetchError> {
    if limit == 0 || limit > MAX_ALLOWED_DOWNLOAD_BYTES {
        return Err(FetchError::InvalidOption {
            reason: "max_bytes must be between 1 byte and 4 GiB",
        });
    }
    Ok(())
}

/// Parses and validates a 64-character ASCII hex SHA-256 digest up front.
pub(crate) fn parse_expected_sha256(hex: &str) -> Result<[u8; 32], FetchError> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(FetchError::InvalidOption {
            reason: "expected_sha256 must be 64 ASCII hexadecimal characters",
        });
    }
    let mut out = [0u8; 32];
    let (chunks, _) = bytes.as_chunks::<2>();
    for (idx, chunk) in chunks.iter().enumerate() {
        out[idx] = (hex_nibble(chunk[0]) << 4) | hex_nibble(chunk[1]);
    }
    Ok(out)
}

const fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

/// Constant-time comparison of two 32-byte SHA-256 digests.
#[must_use]
pub(crate) fn constant_time_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= std::hint::black_box(a[i] ^ b[i]);
    }
    diff == 0
}

/// Formats a 32-byte digest as lowercase hex.
#[must_use]
pub(crate) fn format_sha256_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Redacts credentials, query string, and fragment from a parsed URL for error
/// reporting.
#[must_use]
pub(crate) fn redact_url(url: &Url) -> String {
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.set_query(None);
    clean.set_fragment(None);
    clean.to_string()
}

/// Parses and validates an initial request URL, appending `query` pairs safely
/// and running SSRF literal-host checks.
pub(crate) fn validate_and_prepare_initial_url(
    raw: &str,
    query: &[(String, String)],
    allow_private_ips: bool,
) -> Result<Url, FetchError> {
    if raw.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(FetchError::InvalidUrl {
            reason: "URL must not contain CR, LF, or NUL".to_owned(),
        });
    }
    let mut url = Url::parse(raw).map_err(|err| FetchError::InvalidUrl {
        reason: err.to_string(),
    })?;
    validate_url_structure(&url)?;
    url.set_fragment(None);
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    check_url_ssrf(&url, allow_private_ips)?;
    Ok(url)
}

/// Resolves and validates a redirect `Location` target against `current`.
pub(crate) fn validate_redirect_url(
    current: &Url,
    location: &str,
    allow_private_ips: bool,
) -> Result<Url, FetchError> {
    if location.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(FetchError::InvalidUrl {
            reason: "redirect Location header must not contain CR, LF, or NUL".to_owned(),
        });
    }
    let mut next = current
        .join(location)
        .map_err(|err| FetchError::InvalidUrl {
            reason: format!("invalid redirect target: {err}"),
        })?;
    validate_url_structure(&next)?;
    next.set_fragment(None);
    check_url_ssrf(&next, allow_private_ips)?;
    Ok(next)
}

fn validate_url_structure(url: &Url) -> Result<(), FetchError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(FetchError::InvalidUrl {
            reason: format!("unsupported URL scheme `{}`", url.scheme()),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchError::InvalidUrl {
            reason: "URLs with embedded credentials are not supported".to_owned(),
        });
    }
    if url.host().is_none() || url.host_str().is_none_or(str::is_empty) {
        return Err(FetchError::InvalidUrl {
            reason: "URL must include a host".to_owned(),
        });
    }
    Ok(())
}

/// Checks literal IP hosts and `localhost` domains against SSRF policy.
pub(crate) fn check_url_ssrf(url: &Url, allow_private_ips: bool) -> Result<(), FetchError> {
    if allow_private_ips {
        return Ok(());
    }
    let blocked = match url.host() {
        Some(Host::Ipv4(v4)) => is_disallowed_ip(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => is_disallowed_ip(IpAddr::V6(v6)),
        Some(Host::Domain(domain)) => is_localhost_domain(domain),
        None => true,
    };
    if blocked {
        return Err(FetchError::SsrfBlocked {
            url: redact_url(url),
        });
    }
    Ok(())
}

fn is_localhost_domain(domain: &str) -> bool {
    let trimmed = domain.trim_end_matches('.');
    trimmed.eq_ignore_ascii_case("localhost")
        || trimmed
            .rsplit_once('.')
            .is_some_and(|(_, tld)| tld.eq_ignore_ascii_case("localhost"))
}

/// Returns `true` when `ip` belongs to a private, loopback, link-local,
/// cloud-metadata, CGNAT, ULA, unspecified, multicast, broadcast, NAT64, or
/// IPv4-mapped/compatible reserved range.
#[must_use]
pub(crate) fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_ipv4(v4),
        IpAddr::V6(v6) => is_disallowed_ipv6(v6),
    }
}

#[must_use]
fn is_disallowed_ipv4(v4: Ipv4Addr) -> bool {
    let [a, b, c, _d] = v4.octets();
    a == 0 // 0.0.0.0/8 (unspecified / "this network")
        || a == 10 // 10.0.0.0/8 (RFC 1918)
        || (a == 100 && (64..=127).contains(&b)) // 100.64.0.0/10 (CGNAT, RFC 6598)
        || a == 127 // 127.0.0.0/8 (loopback)
        || (a == 169 && b == 254) // 169.254.0.0/16 (link-local & cloud metadata)
        || (a == 172 && (16..=31).contains(&b)) // 172.16.0.0/12 (RFC 1918)
        || (a == 192 && b == 0 && c == 0) // 192.0.0.0/24 (IETF protocol assignments)
        || (a == 192 && b == 168) // 192.168.0.0/16 (RFC 1918)
        || a >= 224 // 224.0.0.0/4 multicast + 240.0.0.0/4 reserved + 255.255.255.255 broadcast
}

#[must_use]
fn is_disallowed_ipv6(v6: Ipv6Addr) -> bool {
    if let Some(mapped_v4) = v6.to_ipv4_mapped() {
        return is_disallowed_ipv4(mapped_v4);
    }
    if let Some(compat_v4) = v6.to_ipv4() {
        return is_disallowed_ipv4(compat_v4);
    }
    let segs = v6.segments();
    // NAT64 well-known prefix 64:ff9b::/96 (RFC 6052).
    if segs[0] == 0x0064
        && segs[1] == 0xff9b
        && segs[2] == 0
        && segs[3] == 0
        && segs[4] == 0
        && segs[5] == 0
    {
        let embedded = Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            (segs[6] & 0xff) as u8,
            (segs[7] >> 8) as u8,
            (segs[7] & 0xff) as u8,
        );
        return is_disallowed_ipv4(embedded);
    }
    // NAT64 local-use prefix 64:ff9b:1::/48 (RFC 8215).
    if segs[0] == 0x0064 && segs[1] == 0xff9b && segs[2] == 0x0001 {
        return true;
    }
    // 6to4 prefix 2002::/16 carrying an embedded IPv4 address in segments 1..=2.
    if segs[0] == 0x2002 {
        let embedded = Ipv4Addr::from((u32::from(segs[1]) << 16) | u32::from(segs[2]));
        if is_disallowed_ipv4(embedded) {
            return true;
        }
    }
    v6.is_unspecified() // ::
        || v6.is_loopback() // ::1
        || v6.is_multicast() // ff00::/8
        || (segs[0] & 0xfe00) == 0xfc00 // fc00::/7 (ULA)
        || (segs[0] & 0xffc0) == 0xfe80 // fe80::/10 (link-local)
        || (segs[0] & 0xffc0) == 0xfec0 // fec0::/10 (deprecated site-local)
}

pub(crate) fn filter_resolved_addrs(
    addrs: Vec<SocketAddr>,
    allow_private_ips: bool,
) -> io::Result<Vec<SocketAddr>> {
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no socket addresses resolved",
        ));
    }
    if !allow_private_ips && addrs.iter().any(|addr| is_disallowed_ip(addr.ip())) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "connection to private or non-public IP address refused",
        ));
    }
    Ok(addrs)
}

/// Custom `ureq::Resolver` enforcing bounded DNS lookup (`min(remaining, 10s)`)
/// and blocking disallowed IP addresses when `allow_private_ips == false`.
pub(crate) struct SsrfGuardResolver {
    pub(crate) allow_private_ips: bool,
    pub(crate) deadline: Instant,
    pub(crate) max_dns_timeout: Duration,
}

impl SsrfGuardResolver {
    pub(crate) fn new(allow_private_ips: bool, deadline: Instant) -> Self {
        Self {
            allow_private_ips,
            deadline,
            max_dns_timeout: MAX_DNS_TIMEOUT,
        }
    }
}

const MAX_INFLIGHT_DNS_LOOKUPS: usize = 64;
static INFLIGHT_DNS_LOOKUPS: AtomicUsize = AtomicUsize::new(0);

struct DnsSlotGuard;

impl DnsSlotGuard {
    fn try_acquire() -> Option<Self> {
        let mut cur = INFLIGHT_DNS_LOOKUPS.load(Ordering::Relaxed);
        loop {
            if cur >= MAX_INFLIGHT_DNS_LOOKUPS {
                return None;
            }
            match INFLIGHT_DNS_LOOKUPS.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self),
                Err(actual) => cur = actual,
            }
        }
    }
}

impl Drop for DnsSlotGuard {
    fn drop(&mut self) {
        INFLIGHT_DNS_LOOKUPS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ureq::Resolver for SsrfGuardResolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let dns_timeout = remaining.min(self.max_dns_timeout);
        if dns_timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "DNS resolution timed out",
            ));
        }
        let slot = DnsSlotGuard::try_acquire().ok_or_else(|| {
            io::Error::new(io::ErrorKind::WouldBlock, "too many concurrent DNS lookups")
        })?;
        let host = netloc.to_owned();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("vaeljs-fetch-dns".to_owned())
            .spawn(move || {
                let _slot = slot;
                let res = host.to_socket_addrs().map(|iter| iter.collect::<Vec<_>>());
                let _ = tx.send(res);
            })?;
        match rx.recv_timeout(dns_timeout) {
            Ok(Ok(addrs)) => filter_resolved_addrs(addrs, self.allow_private_ips),
            Ok(Err(err)) => Err(err),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "DNS resolution timed out",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other(
                "DNS resolution worker exited unexpectedly",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::Resolver as _;

    #[test]
    fn ip_blocklist_covers_all_required_ranges() {
        let blocked = [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.168.1.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456:789a::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::0a00:0001",
            "64:ff9b:1:0:a9fe:a9fe:0808:0808",
            "fec0::1",
            "2002:7f00:1::",
        ];
        for raw in blocked {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(is_disallowed_ip(ip), "expected {raw} to be blocked");
        }

        let allowed = [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "100.63.255.254",
            "100.128.0.1",
            "172.32.0.1",
            "2606:2800:220:1:248:1893:25c8:1946",
            "64:ff9b::0101:0101",
            "2002:0101:0101::",
        ];
        for raw in allowed {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(!is_disallowed_ip(ip), "expected {raw} to be allowed");
        }
    }

    #[test]
    fn mixed_public_and_private_resolved_addrs_are_rejected() {
        let addrs = vec![
            "93.184.216.34:443".parse().unwrap(),
            "169.254.169.254:443".parse().unwrap(),
        ];
        let err = filter_resolved_addrs(addrs.clone(), false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(filter_resolved_addrs(addrs, true).unwrap().len(), 2);
    }

    #[test]
    fn ssrf_guard_resolver_blocks_loopback_and_enforces_zero_timeout() {
        let zero = SsrfGuardResolver {
            allow_private_ips: true,
            deadline: Instant::now(),
            max_dns_timeout: Duration::from_secs(10),
        };
        let err = zero.resolve("127.0.0.1:80").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);

        let public_only = SsrfGuardResolver::new(false, Instant::now() + Duration::from_secs(5));
        let err = public_only.resolve("127.0.0.1:80").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

        let allowed = SsrfGuardResolver::new(true, Instant::now() + Duration::from_secs(5));
        let resolved = allowed.resolve("127.0.0.1:80").unwrap();
        assert_eq!(resolved, vec!["127.0.0.1:80".parse().unwrap()]);
    }

    #[test]
    fn constant_time_digest_equality_checks_every_byte() {
        let a = [0x42u8; 32];
        let mut b = [0x42u8; 32];
        assert!(constant_time_eq_32(&a, &b));
        b[31] = 0x43;
        assert!(!constant_time_eq_32(&a, &b));
    }

    #[test]
    fn url_redaction_removes_userinfo_query_and_fragment() {
        let url =
            Url::parse("https://user:secret@api.example.com:8443/v1/items?token=abc#frag").unwrap();
        assert_eq!(redact_url(&url), "https://api.example.com:8443/v1/items");
    }
}
