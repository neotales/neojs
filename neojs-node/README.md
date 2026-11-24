# neojs-node

Opt-in Node compatibility adapter for the V8 `neojs-core` runtime. Enable the
`node-compat` feature on `neojs-core` and construct the runtime with
`Runtime::with_node_compat`.

The modules are `node:child_process` and `node:path`. `node:child_process`
implements `spawnSync` and `execFileSync` with `encoding: "utf8"`; every
asynchronous form throws `ERR_NODE_UNSUPPORTED`. `node:path` implements the
documented POSIX/Win32 subset, with `matchesGlob` explicitly unsupported. Both
modules are registered as native V8 synthetic modules before user code runs, so
static and dynamic imports share one namespace and the same native function
objects. There is no JavaScript source, JSON bridge, host op, or global
installation hook. Neither module is in the `Neo` registry and neither has a
bare or `neo:` alias.

Option and environment enumeration uses a narrow bounded V8 engine API added by
`eng/v8-source/patches/0001-bounded-own-enumerable-string-keys.patch`. The
`node-compat` build therefore requires an archive built from the pinned V8
source and `eng/v8-source/build-bounded.sh`, supplied through
`RUSTY_V8_ARCHIVE`. The C ABI copies keys to a Rust sink; no V8 handle crosses
the boundary. A stock V8 archive lacks the symbol and fails to link; it is not
silently replaced by the unbounded public API. See `eng/v8-source/PATCH.md` and
`docs/node-compat.md`.

A default `Runtime::new` rejects every `node:` import. In an opted-in runtime, a
bare specifier that is a familiar Node module name (including `crypto`, `fs`,
`os`, and `path`) is rejected rather than resolved to a Neo module; use `neo:`
for Neo modules. Process execution also requires an explicit
`ChildProcessPolicy` allowlist; an empty policy imports the module but denies
every execution. There is no shell, `PATH` lookup, inherited environment, or
`stdio: inherit`. Input, output, argument, environment, and key sizes are
bounded; the timeout is a polling/kill deadline, not a hard wall-clock cap. The
allowlist matches canonical paths, not inodes: protect the executable and every
ancestor directory from untrusted writes. A timeout or overflow triggers a
process-group kill and a bounded drain; a natural exit with an inherited pipe
drains first, then kills. An inherited pipe is reported as an explicit
incomplete error, and no reader thread is left waiting. The final direct-child reap uses a
blocking `wait()` and can outlive the deadline if the operating system does not
complete the kill or reap. An allowed executable still runs with the full
authority of the process; this is not a sandbox.

See `neojs-node-builtins` for the shared support matrix and limits.
