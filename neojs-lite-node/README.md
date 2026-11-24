# neojs-lite-node

Opt-in Node compatibility adapter for the QuickJS `neojs-lite-core` runtime.
Enable the `node-compat` feature on `neojs-lite-core` and construct the runtime
with `Runtime::with_node_compat`.

This adapter is synchronous because NeoJS Lite has no host-asynchrony loop. It
serves only `node:child_process` with `spawnSync` and `execFileSync`, and
`encoding: "utf8"` is required. The module is a native QuickJS C module built
with `Module::declare_def`/`ModuleDef`; it has no JavaScript module source and
no JSON bridge, and its named/default exports are Rust functions. `spawn`,
`exec`, `execFile`, `execSync`, `fork`, and `ChildProcess` throw
`ERR_NODE_UNSUPPORTED`; none is faked as a blocking call or a resolved Promise.
The native `ChildProcess` export is a throwing function, not a constructible
class; `new ChildProcess()` is a `TypeError` rather than the coded unsupported
error. String fields are preflighted through QuickJS's string length property
before conversion; the actual UTF-8 length is checked after conversion, so the
host allocation is bounded by four times the configured cap.

Option and environment keys go through the workspace's vendored `rquickjs-sys`
patch, not `Object::keys`. `JS_GetOwnEnumerableStringKeysBounded` rejects Proxy
and exotic objects before any trap, counts every shape slot against a scan bound,
counts enumerable string keys against a key bound, allocates only after both
pass, and preserves JavaScript key order. Limits are 6 keys and 64 scanned slots
for `options`, 128 keys and 4096 scanned slots for `options.env`, and 4096 UTF-16
units per key. Symbol keys are ignored but their slots still count. See
`vendor/rquickjs-sys/VENDOR.md` and `docs/node-compat.md`.

A default `Runtime::new` rejects every `node:` import. In an opted-in runtime, a
bare specifier that is a familiar Node module name is rejected rather than
resolved as a Neo module; use the `neo:` prefix for any future Neo module.
Process execution requires an explicit `ChildProcessPolicy` allowlist; an empty
policy denies every execution. There is no shell, `PATH` lookup, inherited
environment, or `stdio: inherit`. Input, output, argument, environment, and key
sizes are bounded; the timeout is a polling/kill deadline, not a hard
wall-clock cap. The allowlist matches canonical paths, not inodes: protect the
executable and every ancestor directory from untrusted writes. A timeout,
overflow, or inherited pipe left after the direct child exits triggers a bounded
drain and a process-group kill; an inherited pipe is reported as an explicit
incomplete error, and no reader thread is left waiting. The final direct-child
reap uses a blocking `wait()` and can outlive the deadline if the operating
system does not complete the kill or reap. An allowed executable still runs
with the full authority of the process; this is not a sandbox.

See `neojs-node-builtins` for the shared support matrix and limits.
