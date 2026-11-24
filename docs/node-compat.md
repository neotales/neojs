# Opt-in Node compatibility

NeoJS does not ship a Node compatibility layer by default. The `node:` resolver,
the shared process core, and both engine adapters are compiled only with the
off-by-default `node-compat` feature, and installed at runtime only when a
runtime is explicitly constructed for Node compatibility. The default V8 and
QuickJS graphs do not include `neojs-node`, `neojs-lite-node`, or
`neojs-node-builtins`.

## Build and enable

`neojs-core` and `neojs-lite-core` each have an off-by-default `node-compat`
feature:

```sh
cargo build -p neojs-core --features node-compat
cargo build -p neojs-lite-core --features node-compat
```

At runtime, opt in with the corresponding constructor:

```rust
use neojs_core::{ChildProcessPolicy, NodeOptions, Runtime};

let policy = ChildProcessPolicy::new()
    .allow_program("/bin/echo")?;
let mut runtime = Runtime::with_node_compat(NodeOptions::new(policy))?;
```

```rust
use neojs_lite_core::{ChildProcessPolicy, NodeOptions, Runtime};

let policy = ChildProcessPolicy::new()
    .allow_program("/bin/echo")?;
let mut runtime = Runtime::with_node_compat(NodeOptions::new(policy))?;
```

`Runtime::new()` and every other existing constructor reject `node:`
specifiers. A Node module is never added to `Neo.getBuiltInModulesList`, and
`Neo.getBuiltInModule("node:child_process")` and
`Neo.getBuiltInModule("node:path")` return `undefined`. A bare specifier such as
`child_process` or `path` is rejected with a request to use the `node:` prefix;
it is not an alias.

While Node compatibility is enabled, a bare specifier that is a familiar Node
module name (including `assert`, `console`, `crypto`, `fs`, `os`, and `path`) is
rejected instead of resolving to a NeoJS module of that name. Neo built-ins are
canonical-only in every runtime, so use `neo:` for a Neo module and `node:` for a
Node module. A Node import is therefore never silently executed as a Neo API,
and a bare Node name is never a Node alias.

This slice has no command-line flag. The opt-in is the embedding constructor
above; the `neojs` and `neojs-lite` commands do not expose a Node switch and
continue to reject `node:` imports.

## Rust-native module status

The child-process implementations, policy, and synchronous runner with bounded
buffers and a polling/kill deadline are
Rust in `neojs-node-builtins`. Both adapters expose that core as native modules:

* The QuickJS adapter uses `Module::declare_def`/`ModuleDef`; its exports are
  native QuickJS functions.
* The V8 adapter calls `JsRuntime::register_native_module` from the workspace's
  vendored `deno_core` 0.413.0 patch. Its exports are native V8 function objects
  registered in the normal module map before user code runs.

Neither adapter has a JavaScript module source, a JSON bridge, a host op, or a
global installation hook. Static imports, dynamic imports, and repeated imports
see one module instance and the same function objects. The vendored patch adds
only that public registration seam; provenance and the diff are in
`vendor/deno_core/PATCH.md`. It is a separate, pinned patch from
`vendor/rquickjs-sys`.

The existing NeoJS built-ins still implemented in JavaScript are `age`,
`assert`, `console`, `cron`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`,
`strings`, `test`, and `uuid`. `chars` is native in both runtimes. A repo-wide
Rust-native migration of the rest is separate work and is not claimed here.

### Native property enumeration bound

The native QuickJS adapter does not call `rquickjs::Object::keys`. That API calls
QuickJS `JS_GetOwnPropertyNames`, which scans the whole shape and materialises the
complete `JSPropertyEnum` list before any count cap can be applied. Instead, this
workspace vendors `rquickjs-sys` 0.14.0 with one source patch (provenance and
licence in `vendor/rquickjs-sys/VENDOR.md`). The patch adds:

- `JS_GetOwnEnumerableStringKeysBounded`: rejects Proxy and every exotic object
  before any callback or trap can run; counts every shape slot (deleted,
  non-enumerable, symbol and private slots included) against `max_scan`; counts
  enumerable string keys against `max_keys` with checked arithmetic; allocates
  only after both bounds pass; preserves JavaScript key order (integer keys
  ascending, then other string keys in insertion order); and returns owned atoms
  that the caller frees through RAII.
- `JS_AtomToCStringLenBounded`: checks a key's UTF-16 length before converting it
  to a UTF-8 C string.

Limits applied by the adapter:

| Object | Max enumerable string keys | Max shape slots scanned | Max key length |
| --- | --- | --- | --- |
| `options` | 6 | 64 | 4096 UTF-16 units |
| `options.env` | 128 | 4096 | 4096 UTF-16 units |

Both bounds fail closed. Options report `ERR_NODE_UNSUPPORTED`; environment
objects report `ERR_NODE_REQUEST_TOO_LARGE`. Allocation failure is different: the
pending QuickJS out-of-memory exception is rethrown unchanged rather than being
translated into a Node input error, and the Rust guard releases the atom table on
every other failure path. Symbol keys are not returned, but
symbol slots still count toward the scan bound. Proxy and exotic objects
(including `Map`, `Set`, typed arrays and module namespaces) are rejected before
a trap runs or a property list is built. This is a deliberate compatibility
narrowing: option bags with more than 64 shape slots, or environment objects with
more than 4096 shape slots, are rejected instead of enumerated. Proxy argument
arrays remain supported with a one-time `length` snapshot.

Tests cover 0, 128 and 129 keys, the inclusive 64-slot scan boundary,
100,000-byte option and environment keys, deleted/non-enumerable/symbol slots,
integer-key ordering, lone-surrogate keys, array and typed-array rejection,
getters not being read during enumeration, a throwing getter followed by a
successful call, Proxy `ownKeys` traps that must not run, and repeated failure
cleanup. Test-only entry points force the table allocation and an atom conversion
after the table exists to fail. The tests check that both option and environment
mappings surface the QuickJS `out of memory` exception, not
`ERR_NODE_UNSUPPORTED` or `ERR_NODE_REQUEST_TOO_LARGE`, that the context remains
usable, and that repeated failures leave the runtime heap unchanged, which
exercises the atom-table RAII cleanup.

### V8 bounded own-key build

The stock V8 public API exposes no bounded own-key enumeration:
`Object::get_own_property_names` materializes the complete key array. The V8
adapter therefore requires a V8 archive built from the pinned
`v8` 150.4.0 / V8 15.0.245.2 source with
`eng/v8-source/patches/0001-bounded-own-enumerable-string-keys.patch` applied.
The patch adds a narrow C ABI used only by `neojs-node`; it is separate from the
`deno_core` and `rquickjs-sys` patches. `eng/v8-source/build-bounded.sh` verifies
the prepared inputs, recreates a fresh isolated source tree, checks the exact
patch and post-patch file hashes, rejects overlapping build roots, and verifies
the procedure helper code it executes. That check rejects ignored, untracked,
staged, modified or mode-changed entries under `eng/v8-source`, and requires
every recorded helper to be a regular non-symlink file whose bytes and Git
executable bit match the recorded commit. It saves the pre-build revision,
helper-tree object and file count, re-verifies them after the build, and fails
if a clean checkout switched commits mid-build; the post-check does not
overwrite the identity written to provenance. A switch that is reverted before
the post-check is not distinguishable, so the procedure worktree must not be
changed concurrently while a build runs. It locks the output root, uses the
committed `builder.lock` with `--locked` (pinning `bindgen 0.72.1`), builds with
`V8_FROM_SOURCE`, and prints the archive path and hash. Every run gets a unique
`mktemp -d` run root, so target and artifact directories are never reused. The
provenance records the helper tree object and file count. The caller supplies
and must trust the prepared cache/procedure roots; `--verify-only` is drift
detection, not attacker authentication. Neither the V8 tree nor the archive is
committed.

The engine API supports `JS_OBJECT_TYPE` ordinary objects with fast or
dictionary properties and fast non-double elements. It rejects Proxy, functions,
interceptors, access checks, custom-element maps and other exotic objects before
any callback. Dictionary elements, double elements, dictionary capacity beyond
the scan cap, and property/dictionary slots beyond the scan cap fail closed.
It counts all descriptor and element capacity slots against the scan bound
before output allocation, allocates only after the key and scan bounds pass,
and preserves index-first/insertion order. Keys are copied to a Rust sink while
the V8 handle scope is alive; no V8 local handle crosses the C ABI, so there is
no escaped-handle lifetime to manage. Empty keys and keys with unpaired
surrogates are rejected before UTF-8 conversion. Conversion writes UTF-8
directly without the replacement flag; a length/write mismatch is reported as a
distinct conversion failure and becomes a coded `ERR_INVALID_ARG_VALUE`, never
an `undefined` return. A GC/handle-churn unit test exercises the sink path.
Test-only element-scan counters are `thread_local` and prove the slot loop is
not entered before the budget check, including across parallel isolates. A
thread-local test hook fault-injects the conversion-failure path; it is inert
unless called from native test code and is not exposed to JavaScript.

The adapter applies the same caps as the QuickJS adapter: options 6 keys / 64
scanned slots, environment 128 keys / 4096 scanned slots, and 4096 UTF-16 units
per key. Tests cover huge dictionary shapes, hidden/symbol slots, fast index
elements, Proxy rejection, and the ordinary execution surface.

Using a stock V8 archive with the `node-compat` feature fails to link because
the bounded symbol is absent. That is deliberate: there is no silent fallback to
the unbounded public API. Build provenance, pins, observed archive hash, and
the no-bit-reproducibility note are in `eng/v8-source/PATCH.md`; the earlier
handoff is retained in [`v8-bounded-enumeration-handoff.md`](v8-bounded-enumeration-handoff.md).

## Implemented surface

Two Node modules are implemented: `node:child_process` and `node:path`.

### node:child_process

The module exports:

| Export | Status | Notes |
| ------ | ------ | ----- |
| `spawnSync(file, args?, options?)` | implemented | Returns `pid`, `output`, `stdout`, `stderr`, `status`, `signal`; adds `error` for a timeout, output overflow, or `ENOENT`/`EACCES`. |
| `execFileSync(file, args?, options?)` | implemented | Returns stdout as a string; throws on a non-zero exit, signal, timeout, overflow, spawn failure, or policy denial. |
| `spawn`, `exec`, `execFile`, `execSync`, `fork` | not implemented | Throw `ERR_NODE_UNSUPPORTED`. None is wrapped in a blocking call or a resolved Promise. |
| `ChildProcess` | not implemented | The constructor throws `ERR_NODE_UNSUPPORTED`. |

Both implemented functions require `encoding: "utf8"` (or `"utf-8"`). Omitting
it throws `ERR_NODE_UNSUPPORTED_ENCODING`; this slice does not return a Node
`Buffer` or substitute another representation silently.

Supported options are exactly `cwd`, `env`, `input` (a string), `timeout`,
`maxBuffer`, and `encoding`. Every other option, including `shell` and `stdio`,
throws `ERR_NODE_UNSUPPORTED`.

Both adapters validate values directly; no request is serialized. Each caps
input at 1,048,576 code units, arguments at 256 entries and 64 KiB total,
environment at 128 entries and 64 KiB total, and each path at 4,096 code units.
The V8 adapter reads a V8 string's UTF-16 length before copying it to Rust. The
QuickJS adapter reads the string length through QuickJS's public string property
API before converting it, so an oversized file, argument, environment value, or
input is rejected before a large Rust String is allocated. UTF-8 conversion
allocation is bounded by four times the configured cap, and the actual UTF-8
length is checked after conversion. Public rquickjs does not expose an exact
UTF-8 length before conversion; the QuickJS preflight is a single audited C-API
function used only for this bound.

### node:path

`node:path` is a native module with no JavaScript source or wrapper. It
provides:

* `basename(path, suffix?)`, `dirname`, `extname`, `format`, `isAbsolute`,
  `join`, `normalize`, `parse`, `relative`, `resolve`, and
  `toNamespacedPath`;
* `sep` and `delimiter`;
* `posix` and `win32` views with the same functions and constants;
* `_makeLong` as the deprecated alias for `toNamespacedPath`;
* `matchesGlob`, which throws `ERR_NODE_UNSUPPORTED` rather than pretending to
  implement glob matching.

The host default is POSIX on Unix and Win32 on Windows, and the default object
is the matching `posix` or `win32` view, as in Node. The other view is explicit
and independent of the host. `path.resolve` and
`path.toNamespacedPath` use the host working directory, as Node uses
`process.cwd()`.

Inputs are capped at 1,048,576 UTF-8 bytes per string. A larger string throws
`ERR_NODE_REQUEST_TOO_LARGE`; a non-string argument throws
`ERR_INVALID_ARG_TYPE`. `format` requires string fields or `undefined`; unlike
Node's coercing formatter it does not call `ToString` on arbitrary field
values. The algorithms follow Node's common POSIX and Win32 rules for drive
letters, UNC and device roots, reserved device names, `.`/`..`, and trailing
separators. This is a documented subset, not a claim of full Node path parity.

Other `node:` modules are rejected with `the Node module ... is not
implemented`. Resolution never falls through to a `neo:` module.

### Error codes

| Code | Meaning |
| ---- | ------- |
| `ERR_NODE_UNSUPPORTED` | An unimplemented function, option, or operation was requested. |
| `ERR_NODE_UNSUPPORTED_ENCODING` | `encoding: "utf8"` (or `"utf-8"`) was not supplied. |
| `ERR_NODE_CHILD_PROCESS_DENIED` | The canonical executable is not in the policy. |
| `ERR_NODE_COMMAND_NOT_ABSOLUTE` | The program name was relative; there is no `PATH` lookup. |
| `ERR_NODE_LIMIT` | A requested timeout, `maxBuffer`, or other size exceeds the policy. |
| `ERR_NODE_REQUEST_TOO_LARGE` | A native input/argument/environment/path field, or a QuickJS key table, exceeds its hard cap. |
| `ERR_INVALID_ARG_VALUE` | A value or property name is invalid, including an empty or unpaired-surrogate environment/option key. |
| `ERR_MEMORY_ALLOCATION_FAILED` | A bounded property-name buffer could not be allocated; the adapter throws a coded error instead of returning `undefined`. |
| `ERR_NODE_CHILD_PROCESS_UNSUPPORTED` | The Unix child-process runner is not available on this platform. |
| `ETIMEDOUT` | The child was killed at the timeout. |
| `ERR_CHILD_PROCESS_STDIO_MAXBUFFER` | An output stream exceeded `maxBuffer`. |
| `ERR_NODE_CHILD_PROCESS_INCOMPLETE` | The direct child exited while a descendant still held a piped stream; the result is an explicit error, not a success with possibly truncated output. |
| `ERR_NODE_CHILD_PROCESS_IO` | A pipe read failed unexpectedly; output may be incomplete. |
| `ERR_NODE_CHILD_PROCESS_EXIT` | `execFileSync`'s child exited non-zero; `status`, `stdout`, and `stderr` are attached. |
| `ENOENT`, `EACCES` | The program could not be started. `spawnSync` returns these in `error`; `execFileSync` throws. |

## Process authority

`ChildProcessPolicy::new()` allows no programs. It is a permission gate and a
resource bound, not a sandbox.

* The program must be an absolute path whose canonical form is in the policy.
  There is no `PATH` lookup.
* The environment is cleared unless `options.env` supplies an explicit map.
  The ambient environment is never inherited.
* `cwd` is applied to the child; a relative `cwd` resolves against the NeoJS
  process's working directory. It does not change the executable lookup, which
  is already an exact absolute path.
* There is no shell, `stdio: inherit`, or detached process. Arguments are passed
  directly to the executable.
* Default limits are a 10-second polling/kill deadline, 1 MiB per output
  stream, 1 MiB of input, 256 arguments and 64 KiB of argument bytes, and 128
  environment entries and 64 KiB of environment bytes. The policy caps every
  request; a larger `timeout` or `maxBuffer` is rejected rather than clamped.
  The timeout is not a hard wall-clock bound on the whole call.
* A timeout or output overflow kills the child's process group, then drains for
  a short bounded grace, returns any captured prefix, and reports `timedOut` or
  `limitExceeded`. The runner then reaps the direct child with a blocking
  `wait()`, which can outlive the deadline if the operating system does not
  complete the kill or reap.
* If the direct child exits while a descendant still holds an inherited pipe,
  the runner drains for a short grace and then kills the child's process group.
  Because output may be incomplete, the result is an explicit
  `ERR_NODE_CHILD_PROCESS_INCOMPLETE` error, not a success with a status. A
  descendant that creates a new process group and escapes is not tracked or
  killed.
* An unexpected pipe read error is `ERR_NODE_CHILD_PROCESS_IO`; it is never
  treated as end-of-file or a successful exit.
* Both adapters are native and validate each field directly. They never invoke
  `JSON.stringify`, `toJSON`, `Number.isInteger`, or a global bridge. Option
  getters and a Proxy argument array's `length` are read once. Proxy options and
  environments are rejected before `ownKeys` can run. The QuickJS adapter also
  applies the bounded key/scan limits and atom cleanup described above.
* A signal-terminated `execFileSync` child has `status: None`; it is not treated
  as status 0. `spawnSync` keeps `status: null` and reports the signal without an
  `.error` unless a timeout, overflow, incomplete read, or I/O failure occurred.
* `spawnSync(file, undefined, options)` and the same form of `execFileSync`
  retain the third argument as options.
* Unix pipe polling and process-group kill use non-blocking pipes and `poll`;
  no reader or writer threads are created. The final direct-child reap uses a
  blocking `wait()` and is not bounded by the deadline. Another platform returns
  `ERR_NODE_CHILD_PROCESS_UNSUPPORTED` before spawning a child.
* The allowlist matches canonical paths, not inodes or contents. Protect the
  executable **and every ancestor directory** from untrusted writes; otherwise
  a replacement under the same canonical path can run. This is a path gate, not
  object pinning.
* An allowed executable keeps the full authority of the NeoJS process:
  filesystem, network, and further process creation permitted by the operating
  system.

Both adapters call the shared synchronous runner on the calling thread. V8
blocks the isolate thread, as Node's sync APIs do. QuickJS has no host-asynchrony
loop in this runtime, so the lite adapter offers only the synchronous surface.

## Tests

The shared core tests use real absolute system programs and fail if one is
missing; they do not skip. Both adapters run the same real-operation, denial,
registry-separation, unsupported-surface, timeout, overflow, inherited-pipe,
hostile-serialization, path-vector, and huge-request cases through their public
`Runtime` constructors. V8 covers native module identity across static and dynamic
imports, the absence of a bridge global, Proxy option/environment rejection,
and large-input caps; QuickJS covers its bounded enumeration and atom cleanup.
The shared suite also verifies that
a descendant left holding the pipes is killed after the direct child exits, that
an unsupported-platform seam rejects before spawning, that an injected
post-spawn setup failure kills and reaps the child group, and that an injected
pipe read error is not treated as success. Adapter tests cover changing option
getters, a changing Proxy array length, primitive numeric options with replaced
intrinsics, oversized native fields, third-argument options after `undefined`
args, and signal-terminated `execFileSync`.
