# vaeljs-http-core

Engine-neutral core HTTP routing, safe static file resolution, cookie parsing
and formatting, and bounded HTTP policy primitives for `vael:http`, shared by
both the V8 runtime (`vaeljs-core`) and the QuickJS runtime (`vaeljs-lite-core`).

This crate uses `#![forbid(unsafe_code)]` and has zero JavaScript engine
dependencies (`v8`, `deno_core`, `rquickjs`), zero `tokio`, and zero `hyper`
dependencies. Its only runtime dependency is `thiserror`.

```sh
cargo test --offline -p vaeljs-http-core
cargo clippy --offline -p vaeljs-http-core --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --offline --no-deps -p vaeljs-http-core
cargo tree --offline -p vaeljs-http-core
```

## Primitives

* **`Router`**: Go 1.22+ `ServeMux`-style method and path-parameter router with
  deterministic specificity precedence (`literal > {param} > {wildcard...}` and
  method-specific > any-method), automatic `GET` -> `HEAD` fallback, sorted
  `Allow` lists for `405 Method Not Allowed`, and safe percent-decoding.
* **`resolve_static_file` / `mime_type_for_path`**: Path-traversal-hardened
  static file resolver rejecting NUL bytes, backslashes, `..` segments, and
  symlink escapes outside `root_dir`, with optional `index_file` resolution and
  web MIME type mapping.
* **`parse_cookies` / `SetCookie` / `format_set_cookie`**: Bounded `Cookie`
  header parser and injection-safe `Set-Cookie` formatter supporting `Path`,
  `Domain`, `Max-Age`, `HttpOnly`, `Secure`, and `SameSite`.
* **HTTP policy constants & validation**: `DEFAULT_MAX_BODY_BYTES` (10 MiB),
  `DEFAULT_MAX_HEADER_BYTES` (64 KiB), `validate_header_name`, and
  `validate_header_value`.
