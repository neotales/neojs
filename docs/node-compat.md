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
`Neo.getBuiltInModule("node:child_process")` returns `undefined`. A bare
specifier such as `child_process` is rejected with a request to use the
`node:` prefix; it is not an alias.

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

The `node:child_process` method implementations and policy are in Rust
(`neojs-node-builtins`). The QuickJS adapter now exposes a native C module
(`Module::declare_def`/`ModuleDef`) whose exports are Rust functions; it has no
JavaScript module source and no JSON bridge. The V8 adapter still delivers its
exports through `child_process.js`, which marshals arguments over the JSON
bridge. That V8 JS facade does **not** satisfy the no-JavaScript-built-in-modules
requirement; it is a migration item, not a completed native module.

The V8 adapter is blocked by the public `deno_core` 0.413.0 module seam:

* `ModuleMap` is `pub(crate)`, and `JsRuntime::module_map()` is `pub(crate)` and
  test-only. The public `ModuleMap::new_synthetic_module_from_exports_object`
  therefore cannot be reached by this crate.
* `ModuleLoader::load` can return only a `ModuleSource`/`ModuleSourceCode`, which
  is source text; it cannot return a Rust-created module object.
* `Extension::synthetic_esm_modules` maps a specifier to a **backing script
  specifier**; building it calls `load_ext_script` and requires that script to
  return an object. The only public path is another JS source file, i.e. the
  facade this requirement excludes.

The QuickJS binding exposes a native C-module seam (`Module::declare_def`,
`ModuleDef`, and `Module::exports`), and `neojs-lite-node` now uses it. The
module has no JavaScript source or bridge; its named/default exports are Rust
functions. The V8 side has no safe public named synthetic-module registration,
so its JS facade remains until that seam is available or a deliberate
deno_core fork/extension design is approved.

This change also does not migrate the existing NeoJS built-ins that are still
implemented in JavaScript (`age`, `assert`, `chars`, `console`, `cron`, `crypto`,
`os`, `path`, `slices`, `ssh`, `stdio`, `strings`, `test`, and `uuid`). A
repo-wide Rust-native built-ins migration is separate work and is not claimed
here.

### Native property enumeration limitation

`rquickjs::Object::keys` calls QuickJS `JS_GetOwnPropertyNames`, which
materializes the complete `JSPropertyEnum` list before this module can cap the
count. There is no public QuickJS/rquickjs API for a count-only or bounded
property iterator. The native adapter rejects Proxy `options` and `options.env`
before enumeration, so an adversarial `ownKeys` trap is not entered; Proxy
argument arrays remain supported with a one-time length snapshot. For accepted
plain objects, the adapter enumerates atoms rather than converting each key to a
Rust `String`, caps the number it converts, preflights each environment key's
UTF-8 length, and does not interpolate unsupported option keys into errors. The
property-enum array itself is still materialized before that cap, so this is
**not** a strict pre-materialization key-count bound. A rquickjs/QuickJS patch or
a bounded enumeration API is required for that claim.

The tests cover a 100,000-byte option key, a 100,000-byte environment key, 129
environment keys, a UTF-8 value that expands beyond its UTF-16 length, and
Proxy `ownKeys` traps for both options and environment. They verify the
adapter's caps and Proxy rejection, not a bound on QuickJS's property-enum
allocation for plain objects.

## Implemented surface

Only `node:child_process` is implemented. The module exports:

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

The V8 JSON bridge request has a hard 4 MiB cap before parsing. The V8 op
checks the V8 string length before copying it to Rust, and the guest wrapper
caps input at 1,048,576 code units, arguments at 256 and 64 KiB total,
environment at 128 entries and 64 KiB total, and each path at 4,096 code units.
The QuickJS module is native: it validates values directly and enforces the same
input, argument, environment, and path bounds without serialization. It reads a
string's UTF-16 length through QuickJS's public string property API before
converting it, so an oversized file, argument, environment value, or input is
rejected before a large Rust String is allocated. UTF-8 conversion allocation is
therefore bounded by four times the configured cap, and the actual UTF-8 length
is checked after conversion. Public rquickjs does not expose an exact UTF-8
length before conversion; the QuickJS length preflight is a single audited
C-API function used only for this bound.

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
| `ERR_NODE_REQUEST_TOO_LARGE` | A V8 bridge request or a native input/argument/environment/path field exceeds its hard cap. |
| `ERR_NODE_CHILD_PROCESS_UNSUPPORTED` | Bounded child-process completion is not implemented on this platform. |
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
* Default limits are a 10-second timeout, 1 MiB per output stream, 1 MiB of
  input, 256 arguments and 64 KiB of argument bytes, and 128 environment entries
  and 64 KiB of environment bytes. The policy caps every request; a larger
  `timeout` or `maxBuffer` is rejected rather than clamped.
* A timeout or output overflow kills the child's process group after a short
  bounded drain, returns any captured prefix, and reports `timedOut` or
  `limitExceeded`.
* If the direct child exits while a descendant still holds an inherited pipe,
  the runner drains for a short grace and then kills the child's process group.
  Because output may be incomplete, the result is an explicit
  `ERR_NODE_CHILD_PROCESS_INCOMPLETE` error, not a success with a status. A
  descendant that creates a new process group and escapes is not tracked or
  killed.
* An unexpected pipe read error is `ERR_NODE_CHILD_PROCESS_IO`; it is never
  treated as end-of-file or a successful exit.
* The V8 guest uses intrinsics captured at bridge installation for
  `JSON.stringify`, record/array creation, `Array.isArray`, and own-property
  checks. Serialized request records and arrays have null prototypes, so
  inherited `toJSON` hooks, getters, or a replaced global `JSON.stringify` are
  not used, and the serialized request is capped before JSON parsing. The
  QuickJS module has no JSON bridge: its native functions validate each field
  and cap input, argument, environment, and path copies directly.
* A signal-terminated `execFileSync` child has `status: None`; it is not treated
  as status 0. `spawnSync` keeps `status: null` and reports the signal without an
  `.error` unless a timeout, overflow, incomplete read, or I/O failure occurred.
* `spawnSync(file, undefined, options)` and the same form of `execFileSync`
  retain the third argument as options.
* Bounded pipe completion is implemented on Unix with non-blocking pipes and
  `poll`; no reader or writer threads are created. Another platform returns
  `ERR_NODE_CHILD_PROCESS_UNSUPPORTED` before spawning a child.
* The allowlist matches canonical paths, not inodes or contents. Protect the
  executable **and every ancestor directory** from untrusted writes; otherwise
  a replacement under the same canonical path can run. This is a path gate, not
  object pinning.
* An allowed executable keeps the full authority of the NeoJS process:
  filesystem, network, and further process creation permitted by the operating
  system.

The V8 op is synchronous and blocks the isolate thread, as Node's sync APIs do.
QuickJS has no host-asynchrony loop in this runtime, so the lite adapter offers
only the synchronous surface.

## Tests

The shared core tests use real absolute system programs and fail if one is
missing; they do not skip. Both adapters run the same real-operation, denial,
registry-separation, unsupported-surface, timeout, overflow, inherited-pipe,
hostile-serialization, and huge-request cases through their public `Runtime`
constructors. V8 also covers pre-import bridge replacement; QuickJS covers the
native module namespace with no bridge. The shared suite also verifies that
a descendant left holding the pipes is killed after the direct child exits, that
an unsupported-platform seam rejects before spawning, that an injected
post-spawn setup failure kills and reaps the child group, and that an injected
pipe read error is not treated as success. Adapter tests cover changing option
getters, a changing Proxy array length, primitive numeric options with replaced
intrinsics, oversized native fields, third-argument options after `undefined`
args, and signal-terminated `execFileSync`.
