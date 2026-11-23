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
- Every `JS_BOUNDED_KEYS_MEMORY` return has a pending QuickJS out-of-memory
  exception: the table allocation goes through `js_malloc`, and the size-overflow
  branch throws explicitly. The Rust adapter rethrows that pending exception
  instead of constructing a new error, so an allocation failure cannot be
  reported as a Node input error. No atom table exists on that path; the normal
  RAII guard still runs.
- Two test-only entry points force allocation failure without a guest surface:
  `JS_GetOwnEnumerableStringKeysBoundedAllocFailureForTest` fails the table
  allocation, and `JS_AtomToCStringLenBoundedAllocFailureForTest` fails an atom
  conversion after the table and its atoms already exist. Both produce a normal
  `out of memory` exception through the same bounds and cleanup paths. They are
  not part of the adapter API and run no JavaScript.
- The generated bindings in `src/bindings/` carry the same declarations for
  every supported target so the ABI is identical on all bundled platforms. The
  C side is target-independent, but only `x86_64-unknown-linux-gnu` has been
  compiled and tested in this change; no other target was cross-compiled here.
- Build gating is unchanged from upstream: the same target table, feature
  switches and WASI handling decide which sources and bindings are used.

This is a source patch, not a fork of `rquickjs` or `rquickjs-core`. The root
workspace uses `[patch.crates-io]` only for this crate.
