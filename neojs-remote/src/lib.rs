//! Engine-independent HTTPS remote transport, user-profile package cache,
//! GitHub specifier normalization, and JSR resolver for neojs.
//!
//! This crate has no JavaScript engine dependency (`deno_core`, `deno_error`,
//! `v8`, and `rquickjs` are absent). With the default `network` feature enabled,
//! `RemoteClient::new` uses `ureq` over `rustls` with bundled `webpki-roots`.
//! With `--no-default-features`, HTTP/TLS transport dependencies are omitted
//! while offline cache reading ([`Cache::load_offline`]), URL and GitHub
//! normalization ([`github_url`]), [`RemoteClient::with_transport`], and
//! offline JSR resolution ([`jsr::CacheRegistry`]) remain available.
//!
//! # Example
//!
//! ```
//! use neojs_remote::{Cache, RemoteClient, RemoteError, RemoteOptions, github_url};
//! use url::Url;
//!
//! let dir = tempfile::tempdir().unwrap();
//! let cache = Cache::new(dir.path());
//!
//! // Normalize a `gh:` specifier and check an empty offline cache (miss -> `Ok(None)`).
//! let url = github_url("gh:owner/repo@v1.0.0/mod.ts").unwrap().unwrap();
//! assert_eq!(cache.load_offline(&url).unwrap(), None);
//!
//! // Non-HTTPS URLs are rejected on offline reads just as on live fetches.
//! let insecure = Url::parse("http://example.test/mod.ts").unwrap();
//! assert!(matches!(
//!     cache.load_offline(&insecure),
//!     Err(RemoteError::UnsupportedScheme { .. })
//! ));
//! # let _ = (RemoteOptions::new(), std::any::type_name::<RemoteClient>());
//! ```

pub mod github;
pub mod jsr;
pub mod remote;

pub use github::github_url;
#[cfg(any(test, feature = "test-support"))]
pub use remote::TestRemoteResponse;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use remote::test_transport;
pub use remote::{
    Cache, DEFAULT_MAX_REDIRECTS, FetchedModule, OfflineModule, Redactions, RemoteClient,
    RemoteError, RemoteOptions, RemoteRequest, RemoteResponse, RemoteTransport, diagnostic_url,
    display_url, validate_url,
};
