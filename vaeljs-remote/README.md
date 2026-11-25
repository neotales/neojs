# vaeljs-remote

Engine-independent HTTPS remote transport, user-profile package cache, GitHub
specifier normalization, and JSR resolver for vaeljs.

This crate has no JavaScript engine dependency (`deno_core`, `deno_error`,
`v8`, and `rquickjs` are absent). `vaeljs-core` wraps it in thin V8
`ModuleLoader` adapters (`loader.rs` and `jsr_loader.rs`), and offline tooling
can depend on it with `--no-default-features` to read cached modules and
resolve cached JSR packages without pulling in HTTP or TLS dependencies.

```sh
cargo test --locked -p vaeljs-remote
cargo test --locked -p vaeljs-remote --no-default-features
cargo tree -p vaeljs-remote --no-default-features -e normal
```

## Feature selection

* `network` (enabled by default): enables `ureq` over `rustls` with bundled
  `webpki-roots` and exposes `RemoteClient::new`.
* `--no-default-features`: omits `ureq` and all HTTP/TLS dependencies while
  retaining `Cache::load_offline`, `Cache::load_offline_with_limit`,
  `RemoteClient::with_transport`, `github_url`, and `jsr::CacheRegistry`.

## Modules

* `remote`: `RemoteOptions`, `RemoteClient`, `RemoteTransport`, `Cache` (with
  atomic writes, per-key lock files capped at 250 ms, monotonic `.rev` revision
  ordering, `.tomb` `no-store` tombstones, `.redir` bounded redirect pointers,
  and lock-free `Cache::load_offline`), `validate_url`, `display_url`, and
  `diagnostic_url`.
* `github`: `github_url` normalization for `gh:` and `github:` specifiers
  through `vaeljs-manifest`.
* `jsr`: `Specifier`, `PackageMetadata`, `VersionMetadata`, `resolve_module`,
  canonical `https://jsr.io` URL helpers, `Registry` trait, `RemoteRegistry`,
  and offline `CacheRegistry`.

## Cache layout and threat model

* Entries are stored in `$XDG_CACHE_HOME/vaeljs/packages` (when absolute) or
  `$HOME/.cache/vaeljs/packages` (`%USERPROFILE%\.cache\vaeljs\packages` on
  Windows), keyed by the SHA-256 of the canonical credential-free URL without
  its fragment (preserving query strings).
* `Cache::load_offline` takes no write locks, ignores `max-age` expiry,
  verifies exact length and SHA-256 checksums, fails closed on corrupt `.tomb`
  tombstones or corrupt/oversized entries, and follows `.redir` pointers up to
  `DEFAULT_MAX_REDIRECTS` (5) hops while rejecting redirect loops.
* `Cache-Control: no-store` on a `200`, `304`, or `3xx` hop suppresses caching
  and tombstones stale records so offline reads do not resurrect `no-store`
  content.
* This is not SSRF protection, not a sandbox, and not a cryptographic
  authentication boundary for the local cache: an attacker who can rewrite both
  cached metadata and body files in the user's cache directory can keep
  checksums consistent.
