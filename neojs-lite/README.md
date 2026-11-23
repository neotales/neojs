# neojs-lite

A secondary command-line wrapper around `neojs-lite-core`, the embeddable
QuickJS runtime. The primary NeoJS Lite interface is the library; use the
command only when you want to run a module from a shell.

See [`neojs-lite-core/README.md`](../neojs-lite-core/README.md) for embedding,
the runtime lifecycle, and the complete limitations.

## Run a module

```sh
cargo run -p neojs-lite -- hello.js
```

```js
const name = "neojs-lite";
console.log(`hello from ${name}`);
```

Relative imports, absolute paths, and `file:` URLs resolve from the importing
module. Static imports and dynamic `import()` use the same loader. `console.log`
writes to standard output and `console.error` to standard error.

The command maps module and job failures to exit 1 and preflight failures to
exit 2, matching the V8 `neojs` command's classes.

## `neo:crypto`

The command enables the `crypto` feature explicitly. The library default is off
for embedders; enable it on `neojs-lite-core` if you embed the runtime.

The only built-in module is `neo:crypto`. Neo built-ins always require the
prefix, so `import ... from "neo:crypto"` (case-insensitive `neo:` scheme) is
the only spelling. A bare `crypto` import is **not** an alias and fails as a
normal bare specifier.

The module exports exactly four synchronous functions:

* `generateEnvelopeKey()` — 32 CSPRNG bytes.
* `randomBytes(length)` — CSPRNG bytes, at most 8 MiB.
* `encrypt(key, plaintext[, associatedData])` — a NEO1 AES-256-GCM envelope.
* `decrypt(key, envelope[, associatedData])` — the plaintext or a stable error.

There is no `generateKey`, hash, HMAC, PBKDF2, bcrypt, cipher, Shamir, `subtle`,
or async export. Importing one is a link error; the command never fakes async
work or returns an already-resolved Promise.

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

```sh
cargo run -p neojs-lite -- main.js
```

Keys are exactly 32 bytes; plaintext and associated data are at most 8 MiB; an
envelope is at most 8 MiB + 34 bytes. Randomness is the operating-system CSPRNG
through `getrandom`, with no fallback. Failures use the stable
`NEO_CRYPTO_*` codes; argument errors are `TypeError` or `RangeError`. The
module stores no keys, promises no zeroization, and is not a sandbox. Detached
views and resizable views whose current range cannot be proven are rejected
rather than truncated. See
[`neojs-lite-core/README.md`](../neojs-lite-core/README.md) for the full
contract.

## Limitations

This command does **not** claim parity with `neojs` (V8):

* **No TypeScript.** `.ts` and `.mts` are rejected with a clear reason. Use
  `.js` or `.mjs`.
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
QuickJS or add `rquickjs` to the default graph. Build this command explicitly:

```sh
cargo build -p neojs-lite
cargo test -p neojs-lite-core -p neojs-lite
```

## Licence

MIT, matching the workspace. QuickJS and `rquickjs` are MIT-licensed dependencies;
see `neojs-lite-core/THIRD_PARTY_NOTICES.md`.
