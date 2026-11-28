//! Engine-neutral synchronous HTTP fetch and streaming atomic download core
//! for `vael:fetch`.
//!
//! This crate powers `vael:fetch` for both the V8 runtime (`vjs`) and the
//! QuickJS runtime (`vjs-lite`). It enforces `#![forbid(unsafe_code)]` and has
//! zero `tokio`, `hyper`, `v8`, or `rquickjs` dependencies.
//!
//! # Security Guarantees
//!
//! * **SSRF Protection:** Only `http:` and `https:` URLs without embedded
//!   userinfo are accepted. A custom `ureq::Resolver` with a bounded lookup
//!   timeout (`min(timeout, 10s)`) blocks private, loopback, link-local, cloud
//!   metadata, CGNAT, ULA, unspecified, multicast, broadcast, NAT64, and
//!   IPv4-mapped/compatible IPv6 addresses unless `allow_private_ips` is
//!   explicitly enabled.
//! * **Redirect Policy:** Automatic redirects are disabled and an explicit
//!   hop-by-hop loop handles `301`, `302`, `303`, `307`, and `308`,
//!   re-validating every hop, stripping `Authorization`, `Proxy-Authorization`,
//!   and `Cookie` on origin changes, and applying standard Fetch method/body
//!   rewrites.
//! * **Bounded Memory & Atomic Streaming:** [`fetch_sync`] enforces
//!   `max_response_bytes` while reading. [`download_sync`] streams in 64 KiB
//!   chunks directly to a `create_new(true)` sibling temporary file with
//!   incremental SHA-256 verification and RAII cleanup on error.
//! * **Redacted Diagnostics:** Query strings, userinfo, bearer tokens, and
//!   sensitive headers never appear in [`FetchError`] or option `Debug` output.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod error;
mod security;
mod types;

pub use client::{download_sync, download_sync_cancellable, fetch_request_sync, fetch_sync};
pub use error::{FetchError, FetchErrorCode};
pub use types::{
    DEFAULT_DOWNLOAD_MAX_BYTES, DEFAULT_MAX_REDIRECTS, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT,
    DEFAULT_USER_AGENT, DOWNLOAD_CHUNK_BYTES, DownloadOptions, DownloadProgress, DownloadResult,
    FetchBody, FetchOptions, FetchRequest, FetchResponse, MAX_ALLOWED_DOWNLOAD_BYTES,
    MAX_ALLOWED_REDIRECTS, MAX_ALLOWED_RESPONSE_BYTES, MAX_DNS_TIMEOUT, MAX_TIMEOUT,
};
pub use zeroize::Zeroizing;

/// The canonical `vael:fetch` import specifier and module registry key.
pub const MODULE_SPECIFIER: &str = "vael:fetch";
