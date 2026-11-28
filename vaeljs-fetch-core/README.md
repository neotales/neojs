# `vaeljs-fetch-core`

Engine-neutral synchronous HTTP client and streaming atomic file downloader powering `vael:fetch` for both `vjs` (V8) and `vjs-lite` (QuickJS).

## Guarantees

* `#![forbid(unsafe_code)]` and zero `tokio`, `hyper`, `v8`, or `rquickjs` dependencies.
* Uses `ureq` over `rustls` with bundled `webpki-roots` (no ambient proxy or cookie jar).
* Fail-closed SSRF guard (`allow_private_ips: false` by default) blocking loopback, private RFC 1918, link-local / cloud metadata (`169.254.169.254`), CGNAT, ULA, unspecified, multicast, broadcast, NAT64, IPv4-compatible, and IPv4-mapped IPv6 addresses, with bounded DNS resolution (`min(timeout, 10s)`).
* Explicit hop-by-hop redirect loop (`301`, `302`, `303`, `307`, `308`) with origin checks that strip `Authorization`, `Proxy-Authorization`, and `Cookie` on cross-origin hops, and standard Fetch method/body rewrites.
* Bounded streaming reads for both `fetch_sync` (`max_response_bytes`) and `download_sync` (`max_bytes`) with 64 KiB chunked streaming to a sibling `create_new(true)` temporary file, incremental SHA-256 verification via constant-time byte comparison, symlink/reparse guards, and RAII cleanup on error.
* Redacted `Debug` and `Display` output: query strings, userinfo, bearer tokens, and `Authorization`/`Cookie` values never appear in errors or debug representations.
