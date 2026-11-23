# Vendored rquickjs-sys

This directory is `rquickjs-sys` 0.14.0 from crates.io, pinned exactly.

- Upstream: <https://crates.io/crates/rquickjs-sys/0.14.0>
- Repository: <https://github.com/DelSkayn/rquickjs>
- VCS revision from the crate metadata: `d7ef5eeae702fea24c03643064de454f1c1dd4b0`
- Crate licence: MIT (the crate metadata; the QuickJS-NG source is also MIT,
  see `quickjs/LICENSE`)
- Local patch: bounded own-enumerable string-key enumeration plus bounded
  atom-to-C-string conversion. See the declarations in `quickjs/quickjs.h` and
  the implementations in `quickjs/quickjs.c`.
- The patch adds no callbacks, does not run Proxy or other exotic traps, rejects
  every exotic object before inspection, counts all shape slots against a scan
  bound, and allocates only after both key and scan bounds pass.
- The generated bindings in `src/bindings/` carry the same two declarations for
  every supported target so the ABI is identical on all bundled platforms. The
  C side is target-independent, but only `x86_64-unknown-linux-gnu` has been
  compiled and tested in this change; no other target was cross-compiled here.
- Build gating is unchanged from upstream: the same target table, feature
  switches and WASI handling decide which sources and bindings are used.

This is a source patch, not a fork of `rquickjs` or `rquickjs-core`. The root
workspace uses `[patch.crates-io]` only for this crate.
