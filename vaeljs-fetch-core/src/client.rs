//! Blocking HTTP fetch and streaming atomic download implementation.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroize;

use crate::error::FetchError;
use crate::security::{
    SsrfGuardResolver, constant_time_eq_32, format_sha256_hex, is_cross_origin_stripped_header,
    is_sensitive_header_name, normalize_method, parse_expected_sha256, redact_url,
    validate_and_prepare_initial_url, validate_bearer_token, validate_download_max_bytes,
    validate_header_name, validate_header_value, validate_max_redirects,
    validate_max_response_bytes, validate_redirect_url, validate_timeout, validate_user_agent,
};
use crate::types::{
    DEFAULT_USER_AGENT, DOWNLOAD_CHUNK_BYTES, DownloadOptions, DownloadProgress, DownloadResult,
    FetchBody, FetchOptions, FetchRequest, FetchResponse,
};

/// Monotonic counter ensuring unique sibling temporary file names within a process.
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum number of `AlreadyExists` collisions tolerated when creating a temp file.
const MAX_TEMP_CREATE_ATTEMPTS: usize = 64;

/// Performs a synchronous HTTP/HTTPS fetch with SSRF protection, explicit
/// redirect handling, and bounded body reading.
///
/// # Errors
///
/// Returns [`FetchError`] if URL or option validation fails, the SSRF policy
/// blocks the destination or any redirect hop, DNS/request timeouts expire,
/// redirects exceed `opts.max_redirects`, the response body exceeds
/// `opts.max_response_bytes`, or a transport error occurs.
pub fn fetch_sync(url: &str, opts: &FetchOptions) -> Result<FetchResponse, FetchError> {
    let method = normalize_method(&opts.method)?;
    let deadline = validate_timeout(opts.timeout)?;
    validate_max_response_bytes(opts.max_response_bytes)?;
    validate_max_redirects(opts.max_redirects)?;

    let mut active_headers = prepare_headers(
        &opts.headers,
        opts.bearer_token.as_deref().map(String::as_str),
        opts.user_agent.as_deref(),
    )?;

    let body_bytes = prepare_body(&method, &opts.body, &mut active_headers)?;
    let initial_url = validate_and_prepare_initial_url(url, &opts.query, opts.allow_private_ips)?;

    let agent = build_agent(opts.allow_private_ips, deadline, opts.timeout);
    let (response, final_url, redirected) = execute_with_redirects(
        &agent,
        initial_url,
        method,
        active_headers,
        body_bytes,
        opts.max_redirects,
        opts.allow_private_ips,
        deadline,
    )?;

    let status = response.status();
    let status_text = response.status_text().to_owned();
    let headers = collect_response_headers(&response);

    if let Some(declared) = parse_content_length(&response)
        && declared > opts.max_response_bytes as u64
    {
        return Err(FetchError::BodyTooLarge {
            url: redact_url(&final_url),
            limit: opts.max_response_bytes as u64,
        });
    }

    let body = read_bounded_body(
        response.into_reader(),
        opts.max_response_bytes,
        deadline,
        &final_url,
    )?;

    Ok(FetchResponse {
        status,
        status_text,
        ok: (200..=299).contains(&status),
        url: final_url.to_string(),
        redirected,
        headers,
        body,
    })
}

/// Executes a [`FetchRequest`] via [`fetch_sync`].
///
/// # Errors
///
/// Propagates any [`FetchError`] returned by [`fetch_sync`].
pub fn fetch_request_sync(request: &FetchRequest) -> Result<FetchResponse, FetchError> {
    fetch_sync(&request.url, &request.options)
}

/// Streams an HTTP/HTTPS `GET` response directly to disk via a sibling
/// temporary file, verifying optional SHA-256 integrity and atomically
/// committing `dest_path`.
///
/// Non-2xx HTTP status codes return [`FetchError::HttpStatus`] without creating
/// or modifying `dest_path`. Any error during streaming, size overflow, timeout,
/// or checksum mismatch deletes the temporary file via an RAII drop guard.
///
/// # Errors
///
/// Returns [`FetchError`] on invalid options or URL, SSRF block, timeout,
/// non-2xx HTTP status, `max_bytes` overflow, SHA-256 mismatch, existing
/// destination when `overwrite` is `false`, symlink/reparse destination, or I/O
/// failure.
pub fn download_sync(
    url: &str,
    dest_path: &Path,
    opts: &DownloadOptions,
    mut on_progress: Option<&mut dyn FnMut(DownloadProgress)>,
) -> Result<DownloadResult, FetchError> {
    let deadline = validate_timeout(opts.timeout)?;
    validate_max_redirects(opts.max_redirects)?;
    validate_download_max_bytes(opts.max_bytes)?;

    let expected_digest = opts
        .expected_sha256
        .as_deref()
        .map(parse_expected_sha256)
        .transpose()?;

    let parent_dir = validate_destination_upfront(dest_path, opts.overwrite)?;

    let active_headers = prepare_headers(
        &opts.headers,
        opts.bearer_token.as_deref().map(String::as_str),
        opts.user_agent.as_deref(),
    )?;

    let initial_url = validate_and_prepare_initial_url(url, &opts.query, opts.allow_private_ips)?;

    let agent = build_agent(opts.allow_private_ips, deadline, opts.timeout);
    let (response, final_url, _redirected) = execute_with_redirects(
        &agent,
        initial_url,
        "GET".to_owned(),
        active_headers,
        None,
        opts.max_redirects,
        opts.allow_private_ips,
        deadline,
    )?;

    let status = response.status();
    if !(200..=299).contains(&status) {
        return Err(FetchError::HttpStatus {
            status,
            url: redact_url(&final_url),
        });
    }

    let total_bytes = parse_content_length(&response);
    if let Some(declared) = total_bytes
        && declared > opts.max_bytes
    {
        return Err(FetchError::BodyTooLarge {
            url: redact_url(&final_url),
            limit: opts.max_bytes,
        });
    }

    let response_headers = collect_response_headers(&response);
    let content_type = find_header(&response_headers, "content-type").map(str::to_owned);
    let etag = find_header(&response_headers, "etag").map(str::to_owned);

    // Bind RAII temp guard before the file handle so the OS handle closes before
    // `TempFileGuard::drop` unlinks the file on any early error return.
    let (mut guard, file) = create_unique_temp_file(parent_dir)?;

    let mut hasher = Sha256::new();
    let mut bytes_written: u64 = 0;

    {
        let mut file = file;
        let mut reader = response.into_reader();
        let mut buf = vec![0u8; DOWNLOAD_CHUNK_BYTES];

        loop {
            if Instant::now() >= deadline {
                return Err(FetchError::Timeout {
                    url: redact_url(&final_url),
                });
            }
            let read_len = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) if is_timeout_io_error(&err) => {
                    return Err(FetchError::Timeout {
                        url: redact_url(&final_url),
                    });
                }
                Err(_) => {
                    return Err(FetchError::Network {
                        url: redact_url(&final_url),
                        reason: "network I/O error while reading response stream",
                    });
                }
            };

            let next_written =
                bytes_written
                    .checked_add(read_len as u64)
                    .ok_or(FetchError::BodyTooLarge {
                        url: redact_url(&final_url),
                        limit: opts.max_bytes,
                    })?;
            if next_written > opts.max_bytes {
                return Err(FetchError::BodyTooLarge {
                    url: redact_url(&final_url),
                    limit: opts.max_bytes,
                });
            }

            file.write_all(&buf[..read_len]).map_err(map_io_error)?;
            hasher.update(&buf[..read_len]);
            bytes_written = next_written;

            if let Some(cb) = on_progress.as_deref_mut() {
                cb(DownloadProgress {
                    bytes_written,
                    total_bytes,
                });
            }
        }

        file.flush().map_err(map_io_error)?;
        file.sync_all().map_err(map_io_error)?;
    }

    let actual_digest: [u8; 32] = hasher.finalize().into();
    let actual_hex = format_sha256_hex(&actual_digest);

    if let Some(expected) = expected_digest
        && !constant_time_eq_32(&actual_digest, &expected)
    {
        return Err(FetchError::ChecksumMismatch {
            expected: format_sha256_hex(&expected),
            actual: actual_hex,
        });
    }

    commit_temp_file(&mut guard, dest_path, parent_dir, opts.overwrite)?;

    Ok(DownloadResult {
        path: dest_path.to_path_buf(),
        status,
        url: final_url.to_string(),
        bytes_written,
        sha256: actual_hex,
        content_type,
        etag,
    })
}

fn prepare_headers(
    raw_headers: &[(String, String)],
    bearer_token: Option<&str>,
    user_agent: Option<&str>,
) -> Result<Vec<(String, String)>, FetchError> {
    let mut headers = Vec::with_capacity(raw_headers.len() + 2);
    for (name, value) in raw_headers {
        validate_header_name(name)?;
        validate_header_value(value)?;
        headers.push((name.clone(), value.clone()));
    }
    if let Some(token) = bearer_token {
        validate_bearer_token(token)?;
        if !has_header(&headers, "authorization") {
            headers.push(("Authorization".to_owned(), format!("Bearer {token}")));
        }
    }
    if let Some(ua) = user_agent {
        validate_user_agent(ua)?;
        if !has_header(&headers, "user-agent") {
            headers.push(("User-Agent".to_owned(), ua.to_owned()));
        }
    }
    Ok(headers)
}

fn prepare_body(
    method: &str,
    body: &FetchBody,
    headers: &mut Vec<(String, String)>,
) -> Result<Option<Vec<u8>>, FetchError> {
    match body {
        FetchBody::None => Ok(None),
        FetchBody::Bytes(bytes) => {
            reject_body_on_get_or_head(method)?;
            Ok(Some(bytes.clone()))
        }
        FetchBody::Text(text) => {
            reject_body_on_get_or_head(method)?;
            Ok(Some(text.as_bytes().to_vec()))
        }
        FetchBody::Json(value) => {
            reject_body_on_get_or_head(method)?;
            let serialized = serde_json::to_vec(value).map_err(|err| FetchError::InvalidJson {
                reason: err.to_string(),
            })?;
            if !has_header(headers, "content-type") {
                headers.push((
                    "Content-Type".to_owned(),
                    "application/json; charset=utf-8".to_owned(),
                ));
            }
            Ok(Some(serialized))
        }
    }
}

fn reject_body_on_get_or_head(method: &str) -> Result<(), FetchError> {
    if method == "GET" || method == "HEAD" {
        return Err(FetchError::InvalidOption {
            reason: "GET and HEAD requests must not carry a request body",
        });
    }
    Ok(())
}

fn build_agent(allow_private_ips: bool, deadline: Instant, timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .https_only(false)
        .redirects(0)
        .try_proxy_from_env(false)
        .max_idle_connections(0)
        .max_idle_connections_per_host(0)
        .timeout(timeout)
        .resolver(SsrfGuardResolver::new(allow_private_ips, deadline))
        .user_agent(DEFAULT_USER_AGENT)
        .build()
}

#[allow(clippy::too_many_arguments)]
fn execute_with_redirects(
    agent: &ureq::Agent,
    mut current_url: Url,
    mut method: String,
    mut headers: Vec<(String, String)>,
    mut body: Option<Vec<u8>>,
    max_redirects: usize,
    allow_private_ips: bool,
    deadline: Instant,
) -> Result<(ureq::Response, Url, bool), FetchError> {
    let mut redirects_followed: usize = 0;

    let result = loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break Err(FetchError::Timeout {
                url: redact_url(&current_url),
            });
        };
        if remaining.is_zero() {
            break Err(FetchError::Timeout {
                url: redact_url(&current_url),
            });
        }

        let mut req = agent.request_url(&method, &current_url).timeout(remaining);
        for (name, value) in &headers {
            req = req.set(name, value);
        }

        let call_result = match body.as_deref() {
            Some(bytes) => req.send_bytes(bytes),
            None => req.call(),
        };

        let response = match call_result {
            Ok(resp) => resp,
            Err(ureq::Error::Status(_, resp)) => resp,
            Err(err) => break Err(classify_ureq_error(err, &current_url, deadline)),
        };

        let status = response.status();
        if matches!(status, 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.header("location")
        {
            if redirects_followed >= max_redirects {
                break Err(FetchError::TooManyRedirects {
                    url: redact_url(&current_url),
                    limit: max_redirects,
                });
            }

            let next_url = match validate_redirect_url(&current_url, location, allow_private_ips) {
                Ok(next) => next,
                Err(err) => break Err(err),
            };

            if next_url.origin() != current_url.origin() {
                headers.retain_mut(|(name, value)| {
                    if is_cross_origin_stripped_header(name) || name.eq_ignore_ascii_case("host") {
                        value.zeroize();
                        false
                    } else {
                        true
                    }
                });
            }

            match status {
                303 if method != "GET" && method != "HEAD" => {
                    method = "GET".to_owned();
                    body = None;
                    strip_body_headers(&mut headers);
                }
                301 | 302 if method == "POST" => {
                    method = "GET".to_owned();
                    body = None;
                    strip_body_headers(&mut headers);
                }
                _ => {}
            }

            current_url = next_url;
            redirects_followed += 1;
            continue;
        }

        break Ok((response, current_url, redirects_followed > 0));
    };

    for (name, value) in &mut headers {
        if is_sensitive_header_name(name) {
            value.zeroize();
        }
    }

    result
}

fn strip_body_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("content-length") && !name.eq_ignore_ascii_case("content-type")
    });
}

fn has_header(headers: &[(String, String)], target: &str) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(target))
}

fn find_header<'a>(headers: &'a [(String, String)], target: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(target))
        .map(|(_, value)| value.as_str())
}

fn collect_response_headers(response: &ureq::Response) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for name in response.headers_names() {
        let lower = name.to_ascii_lowercase();
        if !seen.insert(lower) {
            continue;
        }
        for value in response.all(&name) {
            headers.push((name.clone(), value.to_owned()));
        }
    }
    headers
}

fn parse_content_length(response: &ureq::Response) -> Option<u64> {
    response
        .header("content-length")
        .and_then(|v| v.trim().parse::<u64>().ok())
}

fn read_bounded_body(
    mut reader: Box<dyn Read + Send + Sync>,
    limit: usize,
    deadline: Instant,
    url: &Url,
) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 16 * 1024];

    loop {
        if Instant::now() >= deadline {
            return Err(FetchError::Timeout {
                url: redact_url(url),
            });
        }
        let n = match reader.read(&mut chunk) {
            Ok(0) => return Ok(body),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if is_timeout_io_error(&err) => {
                return Err(FetchError::Timeout {
                    url: redact_url(url),
                });
            }
            Err(_) => {
                return Err(FetchError::Network {
                    url: redact_url(url),
                    reason: "network I/O error while reading response body",
                });
            }
        };

        let next_len = body.len().checked_add(n).ok_or(FetchError::BodyTooLarge {
            url: redact_url(url),
            limit: limit as u64,
        })?;
        if next_len > limit {
            return Err(FetchError::BodyTooLarge {
                url: redact_url(url),
                limit: limit as u64,
            });
        }
        body.extend_from_slice(&chunk[..n]);
    }
}

fn classify_ureq_error(error: ureq::Error, url: &Url, deadline: Instant) -> FetchError {
    let redacted = redact_url(url);
    let Some(transport) = error.into_transport() else {
        return FetchError::Network {
            url: redacted,
            reason: "HTTP status transport error",
        };
    };

    let io_kind = std::error::Error::source(&transport)
        .and_then(|src| src.downcast_ref::<io::Error>())
        .map(io::Error::kind);

    if transport.kind() == ureq::ErrorKind::Dns {
        return match io_kind {
            Some(io::ErrorKind::PermissionDenied) => FetchError::SsrfBlocked { url: redacted },
            Some(io::ErrorKind::TimedOut) => FetchError::DnsTimeout { url: redacted },
            _ if Instant::now() >= deadline => FetchError::Timeout { url: redacted },
            _ => FetchError::Network {
                url: redacted,
                reason: "DNS resolution failed",
            },
        };
    }

    if io_kind.is_some_and(|k| k == io::ErrorKind::PermissionDenied) {
        return FetchError::SsrfBlocked { url: redacted };
    }
    if io_kind.is_some_and(|k| k == io::ErrorKind::TimedOut || k == io::ErrorKind::WouldBlock)
        || Instant::now() >= deadline
    {
        return FetchError::Timeout { url: redacted };
    }

    let reason = match transport.kind() {
        ureq::ErrorKind::InvalidUrl => "invalid URL",
        ureq::ErrorKind::UnknownScheme => "unsupported URL scheme",
        ureq::ErrorKind::Dns => "DNS resolution failed",
        ureq::ErrorKind::InsecureRequestHttpsOnly => "non-HTTPS request refused",
        ureq::ErrorKind::ConnectionFailed => "connection failed",
        ureq::ErrorKind::TooManyRedirects => "too many redirects",
        ureq::ErrorKind::BadStatus => "invalid HTTP status line",
        ureq::ErrorKind::BadHeader => "invalid HTTP header",
        ureq::ErrorKind::Io => "network I/O error",
        ureq::ErrorKind::InvalidProxyUrl => "invalid proxy URL",
        ureq::ErrorKind::ProxyConnect => "proxy connection failed",
        ureq::ErrorKind::ProxyUnauthorized => "proxy authentication failed",
        ureq::ErrorKind::HTTP => "HTTP transport error",
    };

    FetchError::Network {
        url: redacted,
        reason,
    }
}

fn is_timeout_io_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn validate_destination_upfront(dest_path: &Path, overwrite: bool) -> Result<&Path, FetchError> {
    let encoded = dest_path.as_os_str().as_encoded_bytes();
    if encoded.is_empty() || encoded.contains(&0) {
        return Err(FetchError::InvalidOption {
            reason: "destination path must not be empty or contain NUL bytes",
        });
    }
    if encoded
        .last()
        .copied()
        .is_some_and(|b| b == b'/' || b == b'\\')
        || encoded == b"."
        || dest_path.file_name().is_none()
    {
        return Err(FetchError::InvalidOption {
            reason: "destination path must name a file",
        });
    }

    let parent = match dest_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    let parent_meta = std::fs::symlink_metadata(parent).map_err(map_io_error)?;
    if is_symlink_or_reparse(&parent_meta) || !parent_meta.is_dir() {
        return Err(FetchError::InvalidOption {
            reason: "destination parent directory must be a non-symlink directory",
        });
    }

    match std::fs::symlink_metadata(dest_path) {
        Ok(meta) => {
            if !overwrite {
                return Err(FetchError::AlreadyExists);
            }
            if is_symlink_or_reparse(&meta) || meta.is_dir() {
                return Err(FetchError::InvalidOption {
                    reason: "destination path must not be a symlink, reparse point, or directory",
                });
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(map_io_error(err)),
    }

    Ok(parent)
}

fn commit_temp_file(
    guard: &mut TempFileGuard,
    dest_path: &Path,
    parent_dir: &Path,
    overwrite: bool,
) -> Result<(), FetchError> {
    if overwrite {
        match std::fs::symlink_metadata(dest_path) {
            Ok(meta) => {
                if is_symlink_or_reparse(&meta) || meta.is_dir() {
                    return Err(FetchError::InvalidOption {
                        reason: "destination path must not be a symlink, reparse point, or directory",
                    });
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(map_io_error(err)),
        }
        std::fs::rename(guard.path(), dest_path).map_err(map_io_error)?;
        guard.disarm();
    } else {
        match std::fs::hard_link(guard.path(), dest_path) {
            Ok(()) => {
                let _ = std::fs::remove_file(guard.path());
                guard.disarm();
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(FetchError::AlreadyExists);
            }
            Err(err) if err.kind() == io::ErrorKind::Unsupported => {
                match std::fs::symlink_metadata(dest_path) {
                    Ok(_) => return Err(FetchError::AlreadyExists),
                    Err(meta_err) if meta_err.kind() == io::ErrorKind::NotFound => {
                        std::fs::rename(guard.path(), dest_path).map_err(map_io_error)?;
                        guard.disarm();
                    }
                    Err(meta_err) => return Err(map_io_error(meta_err)),
                }
            }
            Err(err) => return Err(map_io_error(err)),
        }
    }

    #[cfg(unix)]
    if let Ok(dir) = File::open(parent_dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = parent_dir;

    Ok(())
}

/// RAII guard that removes the temporary download file if not disarmed.
struct TempFileGuard {
    path: PathBuf,
    active: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, active: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn create_unique_temp_file(parent_dir: &Path) -> Result<(TempFileGuard, File), FetchError> {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());

    for _ in 0..MAX_TEMP_CREATE_ATTEMPTS {
        let seq = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent_dir.join(format!(".vael-fetch-{pid}-{nanos:x}-{seq}.part"));
        match open_create_new_nofollow(&candidate) {
            Ok(file) => return Ok((TempFileGuard::new(candidate), file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(map_io_error(err)),
        }
    }

    Err(FetchError::Io {
        reason: "could not allocate unique temporary download file".to_owned(),
    })
}

fn open_create_new_nofollow(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flags = o_nofollow_flag();
        let mode_flags = 0o600;
        let _ = mode_flags;
        opts.mode(0o600);
        if flags != 0 {
            opts.custom_flags(flags);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    opts.open(path)
}

fn is_symlink_or_reparse(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

#[cfg(unix)]
const fn o_nofollow_flag() -> i32 {
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        0x0100
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "aarch64",
            target_arch = "arm",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "m68k"
        )
    ))]
    {
        0x8000
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "riscv64",
            target_arch = "s390x"
        )
    ))]
    {
        0x20000
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        all(
            any(target_os = "linux", target_os = "android"),
            any(
                target_arch = "aarch64",
                target_arch = "arm",
                target_arch = "powerpc",
                target_arch = "powerpc64",
                target_arch = "m68k",
                target_arch = "x86",
                target_arch = "x86_64",
                target_arch = "riscv64",
                target_arch = "s390x"
            )
        )
    )))]
    {
        0
    }
}

fn map_io_error(err: io::Error) -> FetchError {
    if err.kind() == io::ErrorKind::AlreadyExists {
        return FetchError::AlreadyExists;
    }
    FetchError::Io {
        reason: err.to_string(),
    }
}
