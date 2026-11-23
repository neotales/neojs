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
size are bounded.

This is not a sandbox. An allowed executable runs with the authority of the
NeoJS process. The timeout kills the direct child only; descendants are not
tracked or killed. It is a permission gate and a resource bound, not
containment.

The engine adapters are not part of the default V8 or QuickJS graph. The
workspace's `default-members` excludes this crate and both adapters.
