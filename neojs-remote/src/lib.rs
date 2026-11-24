//! Engine-independent HTTPS remote transport, user-profile package cache,
//! GitHub specifier normalization, and JSR resolver for neojs.
//!
//! This crate has no JavaScript engine dependency (`deno_core`, `deno_error`,
//! `v8`, and `rquickjs` are absent). With the default `network` feature enabled,
//! [`RemoteClient::new`] uses `ureq` over `rustls` with bundled `webpki-roots`.
//! With `--no-default-features`, HTTP/TLS transport dependencies are omitted
//! while offline cache reading ([`Cache::load_offline`]), URL and GitHub
//! normalization ([`github_url`]), and offline JSR resolution
//! ([`jsr::CacheRegistry`]) remain available.

pub mod github;
pub mod jsr;
pub mod remote;

pub use github::github_url;
pub use remote::{
    Cache, DEFAULT_MAX_REDIRECTS, FetchedModule, OfflineModule, Redactions, RemoteClient,
    RemoteError, RemoteOptions, RemoteRequest, RemoteResponse, RemoteTransport, diagnostic_url,
    display_url, validate_url,
};
#[cfg(any(test, feature = "test-support"))]
pub use remote::{TestRemoteResponse, test_transport};
