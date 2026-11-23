# neojs-lite-core

The primary NeoJS Lite interface: an embeddable QuickJS runtime for ECMAScript
module files. `neojs-lite` is a separate, secondary command-line wrapper around
this crate.

`neojs-lite-core` is built with `rquickjs`/QuickJS-NG. It is a separate crate
from the V8-based `neojs-core`; the workspace's default build does not compile
it and does not put QuickJS or `rquickjs` in the V8 graph.

## Embed a module file

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

A complete example is in `examples/embed.rs`:

```sh
cargo run -p neojs-lite-core --example embed -- examples/hello.js
```

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader. `console.log`
writes to standard output and `console.error` to standard error.

## Run an in-memory module

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_source("/app/main.js", "console.log('embedded');")
}
```

`run_source(name, source)` uses `name` as the module identity and the base for
relative imports. Pass an absolute path when the source imports a file; a
relative name resolves imports against the process working directory. The
in-memory entry is not read from disk and its own extension is not checked;
imported files still go through the loader and shared extension policy.

## Lifecycle

A runtime evaluates one entry. A preflight failure (unsupported format,
unreadable file) leaves it unused, so another entry can be tried. Once
evaluation starts, the runtime is consumed even if evaluation, the job drain, or
an unhandled rejection fails. Create a new runtime for subsequent work.

After the entry promise settles, the runtime drains the remaining QuickJS job
queue, so a trailing `.then` or an unawaited dynamic import still runs. The
drain stops after 100,000 jobs and fails closed on a nonconverging chain. A
queued job that throws is an error. A promise still rejected without a handler
when the queue empties is an error; a rejection handled later in the queue is
not. Diagnostics do not execute script code: primitive values and an Error's own
`message`/`stack` data are used, never getters, proxies, `toString`, or `String`.

## Optional `neo:crypto` built-in

The `crypto` Cargo feature is a positive opt-in and is **off by default**. With
it enabled, the runtime registers one native QuickJS module under the canonical
specifier `neo:crypto`. Neo built-ins always require the prefix:
`import ... from "neo:crypto"` (the `neo:` scheme is matched case-insensitively).
A bare `crypto` import is **not** an alias; it remains a normal bare specifier,
reserved for Node compatibility and packages.

The module exports exactly four synchronous functions:

| Export | Contract |
|---|---|
| `generateEnvelopeKey()` | 32 CSPRNG bytes as a `Uint8Array`. |
| `randomBytes(length)` | `length` CSPRNG bytes, at most 8 MiB. |
| `encrypt(key, plaintext[, associatedData])` | A NEO1 AES-256-GCM envelope. |
| `decrypt(key, envelope[, associatedData])` | The plaintext, or a stable error. |

There is no `generateKey`, `createHash`, `hashAsync`, `subtle`, bcrypt, cipher,
or Shamir export. Importing one is a link error, not a rejected Promise. Lite
has no host asynchronous work, so this slice advertises no async form.

### Enable it (workspace / local checkout)

The workspace itself uses a sibling path dependency. This is a local-checkout
example, not a claim that `neojs-lite-core` is published at a registry version.
Adjust the path for your checkout.

```toml
[dependencies]
neojs-lite-core = { path = "../neojs-lite-core", features = ["crypto"] }
```

Cargo unifies features across a build graph: if any dependency enables
`crypto`, every user of this crate in that build gets it too.
`neojs_lite_core::enabled_builtin_modules()` reports `["neo:crypto"]` when the
feature is on and an empty slice when it is off.

### Complete example

`main.js`:

```js
import { decrypt, encrypt, generateEnvelopeKey, randomBytes } from "neo:crypto";

const key = generateEnvelopeKey();
const plaintext = Uint8Array.from([0x6e, 0x65, 0x6f]); // "neo"
const envelope = encrypt(key, plaintext);
const opened = decrypt(key, envelope);
if (opened.length !== plaintext.length) throw new Error("roundtrip length");
for (let i = 0; i < plaintext.length; i++) {
    if (opened[i] !== plaintext[i]) throw new Error("roundtrip byte");
}
console.log(envelope.length, randomBytes(8).length);
```

Run it from the command, which enables the feature explicitly:

```sh
cargo run -p neojs-lite -- main.js
```

An embedder built with the feature gets the same module from `Runtime::new()`
without any further registration.

### Limits and security

* Keys are exactly 32 bytes. Plaintext and associated data are at most 8 MiB;
  an envelope is at most 8 MiB + 34 bytes; `randomBytes` accepts at most 8 MiB.
* The envelope is `NEO1` magic, version 1, AES-256-GCM, a fresh 12-byte random
  nonce, and a 16-byte tag. The header, nonce, and caller associated data are
  authenticated. A wrong key, changed envelope, or changed associated data is
  `NEO_CRYPTO_AUTH` or `NEO_CRYPTO_FORMAT`; no plaintext is returned.
* Randomness is the operating-system CSPRNG through `getrandom`. A failure is
  `NEO_CRYPTO_RANDOM`; there is no weaker fallback.
* Stable failure codes are `NEO_CRYPTO_KEY`, `NEO_CRYPTO_FORMAT`,
  `NEO_CRYPTO_AUTH`, `NEO_CRYPTO_RANDOM`, `NEO_CRYPTO_LIMIT`, and
  `NEO_CRYPTO_DETACHED`. Argument errors are `TypeError` or `RangeError` with no
  code.
* The module stores no keys, promises no zeroization, and is not a sandbox.
  Callers keep the key secret and retire it well before about 2^32 messages
  under one key.
* A `Uint8Array` is recognized by its internal slot, not by a prototype or hook.
  Its range is copied once. Detached views and resizable views whose current
  range cannot be proven are rejected rather than truncated, including
  length-tracking views that grew or shrank.

## Limitations

This crate does **not** claim parity with the V8-based `neojs`:

* **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason.
* **No host asynchrony.** There are no timers or host Promise integration. A
  top-level `await` works only when QuickJS's own job queue can settle it.
* **Only the optional `neo:crypto` built-in.** No other `neo:` module and no
  `Neo.getBuiltInModule` registry exists in Lite.
* **No permissions or sandbox.** None are implemented.

Both runtimes share `neojs-module-policy` for executable-extension decisions, so
`.js`/`.mjs`/extensionless names and the rejection reasons for `.cjs`, JSX/TSX,
declaration files, JSON, WebAssembly, and native addons are the same.

## Build isolation

The workspace's default members are the V8 crates. `cargo build` does not compile
QuickJS or add `rquickjs` to the default graph. Build this runtime explicitly:

```sh
cargo build -p neojs-lite-core
cargo test -p neojs-lite-core -p neojs-lite
cargo test -p neojs-lite-core --features crypto
```

The last line exercises the enabled `neo:crypto` module; `neojs-lite` enables
it for its own tests and its command.

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `THIRD_PARTY_NOTICES.md`.
