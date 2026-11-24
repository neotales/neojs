# neojs-node-builtins

Engine-neutral policy and synchronous process core for opt-in Node compatibility
in NeoJS. It is shared by the V8 adapter (`neojs-node`) and the QuickJS adapter
(`neojs-lite-node`) and has no dependency on either engine.

## What is implemented

| Node module           | Implemented surface                              | Explicitly absent |
| --------------------- | ------------------------------------------------ | ----------------- |
| `node:child_process`  | `spawnSync`, `execFileSync` with `encoding: "utf8"` | `spawn`, `exec`, `execFile`, `execSync`, `fork`, `ChildProcess`, callbacks, streams, promises |
| `node:path`           | `basename`, `dirname`, `extname`, `format`, `isAbsolute`, `join`, `normalize`, `parse`, `relative`, `resolve`, `toNamespacedPath`, `sep`, `delimiter`, `posix`, `win32`, `_makeLong` | `matchesGlob` |

The modules have no bare alias and are never substituted with a `neo:` module.
Every other `node:` specifier is rejected with a reason.

## Security contract

Process execution is disabled by default: a `ChildProcessPolicy` starts with no
allowed programs. An embedding application adds exact canonical executable
paths. There is no `PATH` lookup, no shell, no inherited environment, and no
`stdio: inherit`. Time, input, output, argument count/bytes, and environment
size are bounded. Both adapters validate and copy values directly; there is no
serialized request.

The QuickJS adapter exposes this core through native `ModuleDef` exports with no
JavaScript source or bridge. It preflights string fields through QuickJS's string
length property before conversion and checks actual UTF-8 length after; the host
allocation is bounded by the configured cap. Option and environment keys use
the workspace's vendored `rquickjs-sys` bounded enumeration helper: it rejects
Proxy and exotic objects before any trap, scans at most a documented number of
shape slots, caps the key count, allocates only after both bounds pass, and
preserves JavaScript key order. Keys are limited to 4096 UTF-16 units; `options`
to 6 keys and 64 scanned slots; `options.env` to 128 keys and 4096 scanned slots.
See `vendor/rquickjs-sys/VENDOR.md` and `docs/node-compat.md`.

The V8 adapter registers a native synthetic module through the workspace's
vendored `deno_core` `register_native_module` seam. Its exports are native V8
functions; it has no JavaScript source, bridge, or host op.

On Unix, pipes are non-blocking and driven by `poll` on the calling thread; no
reader or writer threads are created. A timeout or output overflow triggers a
process-group SIGKILL and then a bounded drain; a natural direct-child exit with
an inherited pipe drains first and then triggers the group SIGKILL. A
descendant holding a pipe cannot keep pipe draining blocked. The direct child is
then reaped with a blocking `wait()`; that final
reap is not bounded by the request deadline, so the call can outlive it if the
operating system does not complete the kill or reap. The inherited-pipe case is
reported as an explicit incomplete error, never a successful status with
possibly truncated output. An unexpected pipe read error is an explicit I/O
error, never end-of-file. A descendant that creates a new process group and
escapes is not tracked or killed. Other platforms return
`ERR_NODE_CHILD_PROCESS_UNSUPPORTED` before spawning. Both adapters validate
natively, so `toJSON`, `JSON.stringify`, `Number.isInteger`, and a replaced
global are not consulted.

This is not a sandbox. An allowed executable runs with the authority of the
NeoJS process. The allowlist matches canonical paths, not inodes or contents:
protect the executable and every ancestor directory from untrusted writes. It
is a permission gate and a resource bound, not containment.

The engine adapters are not part of the default V8 or QuickJS graph. The
workspace's `default-members` excludes this crate and both adapters.
