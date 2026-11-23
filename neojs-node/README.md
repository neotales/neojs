# neojs-node

Opt-in Node compatibility adapter for the V8 `neojs-core` runtime. Enable the
`node-compat` feature on `neojs-core` and construct the runtime with
`Runtime::with_node_compat`.

The only module is `node:child_process`; only `spawnSync` and `execFileSync` are
implemented, with `encoding: "utf8"`. Every asynchronous form throws
`ERR_NODE_UNSUPPORTED`. The module is registered as a native V8 synthetic module
before user code runs, so static and dynamic imports share one namespace and the
same native function objects. There is no JavaScript source, JSON bridge, host
op, or global installation hook. The module is not in the `Neo` registry and has
no bare or `neo:` alias.

Option and environment enumeration uses a narrow bounded V8 engine API added by
`eng/v8-source/patches/0001-bounded-own-enumerable-string-keys.patch`. The
`node-compat` build therefore requires an archive built from the pinned V8
source and `eng/v8-source/build-bounded.sh`, supplied through
`RUSTY_V8_ARCHIVE`. A stock V8 archive lacks the symbol and fails to link; it is
not silently replaced by the unbounded public API. See
`eng/v8-source/PATCH.md` and `docs/node-compat.md`.

A default `Runtime::new` rejects every `node:` import. In an opted-in runtime, a
bare specifier that is a familiar Node module name (including `crypto`, `fs`,
`os`, and `path`) is rejected rather than resolved to a Neo module; use `neo:`
for Neo modules. Process execution also requires an explicit
`ChildProcessPolicy` allowlist; an empty policy imports the module but denies
every execution. There is no shell, `PATH` lookup, inherited environment, or
`stdio: inherit`, and all resources are bounded. The allowlist matches canonical
paths, not inodes: protect the executable and every ancestor directory from
untrusted writes. A timeout, overflow, or inherited pipe left after the direct
child exits triggers a bounded drain and a process-group kill; an inherited pipe
is reported as an explicit incomplete error, and no reader thread is left
waiting. An allowed executable still runs with the full authority of the
process; this is not a sandbox.

See `neojs-node-builtins` for the shared support matrix and limits.
