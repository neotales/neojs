# neojs-node-builtins

Engine-neutral policy and synchronous process core for opt-in Node compatibility
in NeoJS. It is shared by the V8 adapter (`neojs-node`) and the QuickJS adapter
(`neojs-lite-node`) and has no dependency on either engine.

## What is implemented

| Node module           | Implemented surface                              | Explicitly absent |
| --------------------- | ------------------------------------------------ | ----------------- |
| `node:child_process`  | `spawnSync`, `execFileSync` with `encoding: "utf8"` | `spawn`, `exec`, `execFile`, `execSync`, `fork`, `ChildProcess`, callbacks, streams, promises |

The module has no bare alias and is never substituted with a `neo:` module.
Every other `node:` specifier is rejected with a reason.

## Security contract

Process execution is disabled by default: a `ChildProcessPolicy` starts with no
allowed programs. An embedding application adds exact canonical executable
paths. There is no `PATH` lookup, no shell, no inherited environment, and no
`stdio: inherit`. Time, input, output, argument count/bytes, and environment
size are bounded. The serialized V8 bridge request is capped before parsing.

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

The V8 adapter still uses a JavaScript export facade because deno_core 0.413.0
has no public named synthetic-module seam; that is an explicit migration item,
not a completed native module.

On Unix, pipes are non-blocking and driven by `poll` on the calling thread; no
reader or writer threads are created. A timeout, output overflow, or an
inherited pipe left after the direct child exits triggers a bounded drain and a
process-group SIGKILL, so a descendant cannot keep a join blocked. That case is
reported as an explicit incomplete error, never a successful status with
possibly truncated output. An unexpected pipe read error is an explicit I/O
error, never end-of-file. A descendant that creates a new process group and
escapes is not tracked or killed. Other platforms return
`ERR_NODE_CHILD_PROCESS_UNSUPPORTED` before spawning. The guest serializes
null-prototype records and arrays with intrinsics captured at bridge
installation, so inherited `toJSON` hooks or a replaced `JSON.stringify` are not
used.

This is not a sandbox. An allowed executable runs with the authority of the
NeoJS process. The allowlist matches canonical paths, not inodes or contents:
protect the executable and every ancestor directory from untrusted writes. It
is a permission gate and a resource bound, not containment.

The engine adapters are not part of the default V8 or QuickJS graph. The
workspace's `default-members` excludes this crate and both adapters.
