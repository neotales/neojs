//! Engine-neutral core HTTP primitives for the `vael:http` module.
//!
//! This crate provides synchronous, deterministic, fail-closed HTTP building
//! blocks shared by both the V8 runtime (`vaeljs-core`) and the QuickJS runtime
//! (`vaeljs-lite-core`). It uses `#![forbid(unsafe_code)]` and carries zero
//! JavaScript engine (`v8`, `deno_core`, `rquickjs`), `tokio`, or `hyper`
//! dependencies.
//!
//! # Modules and Surface
//!
//! * [`Router`] and [`RouteMatch`]: Go 1.22+ `ServeMux`-style method and
//!   path-parameter routing with deterministic precedence
//!   (`literal > {param} > {wildcard...}` and method-specific > any-method),
//!   automatic `GET` -> `HEAD` fallback, sorted `Allow` lists on `405 Method
//!   Not Allowed`, and safe percent-decoding.
//! * [`resolve_static_file`], [`StaticFile`], and [`mime_type_for_path`]:
//!   traversal-resistant static file resolution with canonical symlink
//!   containment checks and web MIME mapping.
//! * [`parse_cookies`], [`parse_cookies_with_limit`], [`SetCookie`],
//!   [`SameSite`], [`format_set_cookie`], and [`DEFAULT_MAX_COOKIES`]: bounded
//!   `Cookie` header parsing and injection-safe `Set-Cookie` formatting.
//! * [`DEFAULT_MAX_BODY_BYTES`], [`DEFAULT_MAX_HEADER_BYTES`],
//!   [`validate_header_name`], and [`validate_header_value`]: bounded HTTP
//!   policy defaults and RFC 9110 header validation.
//!
//! # Example
//!
//! ```
//! use vaeljs_http_core::{RouteMatch, Router, SameSite, SetCookie, format_set_cookie};
//!
//! let mut router = Router::new();
//! router.add("GET /hello/{name}", 1).unwrap();
//!
//! assert_eq!(
//!     router.match_route("GET", "/hello/world"),
//!     RouteMatch::Matched {
//!         handler_id: 1,
//!         params: vec![("name".to_owned(), "world".to_owned())],
//!     }
//! );
//!
//! let cookie = format_set_cookie(
//!     &SetCookie::new("session", "tok123")
//!         .path("/")
//!         .http_only(true)
//!         .same_site(SameSite::Strict),
//! )
//! .unwrap();
//! assert_eq!(cookie, "session=tok123; Path=/; HttpOnly; SameSite=Strict");
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cookies;
mod error;
mod policy;
mod router;
mod static_files;

pub use cookies::{
    DEFAULT_MAX_COOKIES, SameSite, SetCookie, format_set_cookie, parse_cookies,
    parse_cookies_with_limit,
};
pub use error::{HttpCoreError, HttpCoreErrorCode};
pub use policy::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_HEADER_BYTES, validate_header_name, validate_header_value,
};
pub use router::{
    DEFAULT_MAX_PATH_BYTES, DEFAULT_MAX_PATH_SEGMENTS, DEFAULT_MAX_ROUTES, RouteMatch, Router,
};
pub use static_files::{
    StaticFile, mime_type_for_path, resolve_static_file, resolve_static_relative_path,
};
