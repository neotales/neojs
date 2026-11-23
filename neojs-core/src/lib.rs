//! Runs ECMAScript modules on V8 through Deno's Rust bindings.
//!
//! [`Runtime`] loads one entry module from the local filesystem, evaluates it,
//! and drains V8's event loop. A runtime is single-use; a second entry module
//! returns [`Error::RuntimeConsumed`].
//!
//! # Module selection
//!
//! Public Neo modules that can be built independently have an engine-local
//! Cargo feature named `neo-<module>`. `neo-strings` is the first; it is in the
//! crate's default features, so a default build keeps the existing V8 surface.
//! Disabling a module removes its public registry name, its implementation from
//! the realm shim, and its module adapter, and the loader reports the specifier
//! as unimplemented. Features are additive and unify across the dependency
//! graph, so a module is present whenever any dependent enables it; a build
//! cannot promise that a module compiled out here is absent from the whole
//! process.
//!
//! Module selection is compile-time surface selection, **not a sandbox**. It
//! does not remove the process authority of code that still runs, does not
//! create a permission boundary, and does not change the remote-loading opt-in.
//! Node modules have no feature: `node:` specifiers remain rejected.
//!
//! [`Runtime::new`] is local-only. [`Runtime::with_remote`] adds an explicit
//! opt-in for HTTPS modules: it accepts an HTTPS entry through
//! [`Runtime::run_url`] and lets any module import an absolute `https:` URL.
//! Remote modules keep the requested URL as their identity, resolve their
//! relative imports against it, and cannot import `file:`. Only `https:` is a
//! remote transport; redirects must stay credential-free HTTPS and within the
//! configured limit. The client sends no ambient credentials and reads no proxy
//! environment. It is not SSRF protection and not a sandbox: remote code runs
//! with the process's authority. Downloads are cached in one user-profile
//! package cache, `$XDG_CACHE_HOME/neojs/packages` or
//! `$HOME/.cache/neojs/packages`, keyed by a hash of the canonical URL
//! including its query; entries are written atomically, verify a body length
//! and SHA-256 digest, and revalidate with HTTP validators and upstream
//! `Age`/`Date`. There is no lockfile, signature, or content-addressed store.
//! [`RemoteOptions`] sets the per-module deadline, body, aggregate byte,
//! module-count, and redirect limits, and the cache directory.
//!
//! A default build provides the `age`, `assert`, `chars`, `console`, `crypto`,
//! `os`, `path`, `slices`, `ssh`, `stdio`, `strings`, `test`, and `uuid`
//! built-in modules through `Neo.getBuiltInModule(name)` and through the module
//! specifiers `age`, `neo:age`, `assert`, `neo:assert`, `chars`, `neo:chars`,
//! `console`,
//! `neo:console`, `crypto`, `neo:crypto`, `os`, `neo:os`, `path`, `neo:path`,
//! `slices`, `neo:slices`, `ssh`, `neo:ssh`, `stdio`, `neo:stdio`, `strings`,
//! `neo:strings`, `test`, `neo:test`, `uuid`, `neo:uuid`, `uuid/v4`,
//! `neo:uuid/v4`, `uuid/v7`, and `neo:uuid/v7`. The `uuid/v4` and `uuid/v7`
//! subpaths are thin modules whose named and default exports are the same
//! functions as `uuid.v4` and `uuid.v7`.
//!
//! `assert` is a synchronous assertion surface inspired by Node's
//! `assert/strict` and Deno's `@std/assert`. The registry object is the
//! callable `assert` function itself; `equals` is strict structural equality,
//! `strictEquals` is `Object.is` identity, and `notEquals`, `notStrictEquals`,
//! `ok`, `fail`, `throws`, `doesNotThrow`, `rejects`, and `doesNotReject` are
//! available both as named exports and as properties of the callable. Failures
//! throw `AssertionError`, an `Error` subclass with `name`, `code`
//! `"NEO_ASSERT"`, `operator`, and the `actual`/`expected` values when present.
//! Structural equality requires matching prototypes and compares own enumerable
//! string and symbol properties plus built-in state for Dates, RegExps, Maps,
//! Sets, ArrayBuffers, DataViews, typed arrays, boxed primitives, and Errors;
//! cycles are handled, Maps and Sets are order-insensitive, `NaN` equals `NaN`,
//! and `-0` differs from `0`. Functions and genuine native Promises, WeakMaps,
//! WeakSets, and WeakRefs compare by identity. There is no `assertEquals`,
//! `deepEqual`, or `strictEqual`, and this is not a Node or Deno compatibility
//! layer. `assert` calls one private host op to observe a genuine native
//! Promise's settlement; the op reads no file, environment, or network.
//! `throws`/`doesNotThrow` take synchronous callbacks; a genuine native Promise
//! return, checked by the engine's internal slot rather than its prototype, is
//! a `TypeError` that points at `rejects`/`doesNotReject`, which return Promises
//! and assimilate a thenable by reading `then` once. A `rejects` rejection reason
//! that is itself a thenable is subject to native Promise assimilation in the
//! returned Promise.
//!
//! `test` is a small registration-based runner inspired by Node's
//! `node:test`. `test(name, fn)` accepts a synchronous or async callback, and
//! `test(name)` registers a pending test that fails the run. The CLI runs the
//! queue after the entry module settles, so tests registered after a top-level
//! await are included, and it gives each file a fresh [`Runtime`]. `run()`
//! returns cumulative counts and throws `TestFailureError` (`code`
//! `"NEO_TEST_FAILURES"`) whenever that cumulative report has a failure, unless
//! called with `{ throwOnFailure: false }`, which is what the host does. The
//! host also passes `{ settle: true }` for a bounded event-loop settle check; a
//! user call inside top-level await must leave `settle` false because it would
//! see its own pending module evaluation. A
//! rejected Promise, a rejected thenable, a timeout, a generator object, an
//! unhandled rejection, and event-loop work that remains after a bounded settle
//! window are failures. The default per-test timeout is 30 seconds and can be
//! set with `{ timeout }`. A native Promise, including a subclass with throwing
//! `then`, `constructor`, or `Symbol.species` getters, is observed through V8's
//! internal Promise state rather than those properties. A non-native thenable
//! is read once, called once, and its first settlement wins; nested promises and
//! thenables are assimilated recursively, with cycle and chain-length guards.
//! Identity is checked before a later `then` read, so a stateful getter is read
//! once and cannot turn its own cycle into a plain value.
//! The module is available as `test` and `neo:test`;
//! `node:` modules, including `node:test`, remain rejected while no explicit
//! Node-module opt-in exists. It is not Node's `node:test`: there are no
//! subtests, hooks, mocks, snapshots, tags, filters, retries, or parallel
//! tests, and `test()` returns `undefined`. Tests in one file share one realm
//! and its mutable state, so this is not isolation or a sandbox.
//!
//! `age` is synchronous binary age v1 encryption for X25519 recipients. It
//! exposes `generateIdentity`, `identityToRecipient`, `encrypt`, and `decrypt`.
//! It uses the MIT-licensed `neotales-age` crate, returns `Uint8Array` bytes,
//! takes identities and recipients as their standard Bech32 text, and reports
//! failures with stable `NEO_AGE_*` codes. ASCII armor, passphrases, SSH keys,
//! plugins, and post-quantum recipients are not exposed. It is not a sandbox,
//! does not manage keys for the caller, and does not promise zeroization. See
//! `neojs-core/src/age.rs` for the backend decision, limits, and error contract.
//!
//! `crypto` adds AES-256-GCM authenticated encryption with a fresh per-message
//! nonce, an authenticated envelope, and documented size bounds; bounded
//! stateful AES-256-CBC and AES-256-GCM ciphers; SHA-2 hashing, HMAC, PBKDF2,
//! and bcrypt; and GF(256) Shamir secret sharing. It exposes
//! `generateEnvelopeKey`, `randomBytes`, `encrypt`, `decrypt`,
//! `createEncryptor`, `createDecryptor`, incremental `createHash` and
//! `createHmac` (with explicit `close()`/`dispose()` and a per-runtime
//! open-object cap), one-shot `hashAsync` and `hmacAsync`, synchronous
//! `pbkdf2`, `bcryptHash`, and `bcryptVerify`, the asynchronous `pbkdf2Async`,
//! `bcryptHashAsync`, and `bcryptVerifyAsync`, and `shamirSplit`,
//! `shamirCombine`, `shamirSplitAsync`, and `shamirCombineAsync`. The envelope
//! and cipher functions are synchronous and return `Uint8Array` values; cipher
//! `update` also accepts a UTF-8 string.
//! `createEncryptor`/`createDecryptor` accept `aes-256-cbc` or `aes-256-gcm`,
//! buffer bounded input across `update`, and produce output at `final`; GCM
//! decryptors require `setAuthTag` before `final`, and GCM plaintext is never
//! returned from `update`. CBC is explicitly unauthenticated and uses strict
//! PKCS#7 padding unless `setAutoPadding(false)` is used. This is
//! Node-`createCipheriv`-inspired, not Node-compatible, and the cipher API has
//! no Promise form. `createHash`/`createHmac` are synchronous incremental
//! objects over `sha256`, `sha384`, and `sha512`; `pbkdf2` accepts the same
//! three names. `hashAsync`/`hmacAsync`/`pbkdf2Async`/`bcryptHashAsync`/
//! `bcryptVerifyAsync` and `shamirSplitAsync`/`shamirCombineAsync` return a
//! `Promise` resolved by the runtime's bounded blocking pool, not a
//! `Promise.resolve` wrapper around blocking work.
//! Bcrypt emits `$2b$`, verifies `$2a$`, `$2b$`, and `$2y$`, rejects passwords
//! over 72 bytes, defaults to cost 10, and never verifies a hash whose encoded
//! cost is above 16. Random values come from the operating system CSPRNG. The
//! name `generateKey` is deliberately reserved for a future Node-compatible
//! surface and is not exported. It is not a sandbox, it does not manage keys for
//! the caller, and it does not promise zeroization. The envelope layout, error
//! codes, and limits are documented in `neojs-core/src/crypto.rs`; the stateful
//! CBC/GCM lifecycle, padding, tag/AAD handling, buffering, and resource caps
//! are documented in `neojs-core/src/crypto_cipher.rs`; the hash algorithms,
//! encodings, lifecycle, asynchronous executor condition, and bounds are
//! documented in `neojs-core/src/crypto_hash.rs`; PBKDF2 and bcrypt are
//! documented in `neojs-core/src/crypto_hash/password.rs`; Shamir's GF(256)
//! encoding, one-based indexes, threshold and authenticity limitations,
//! bounds, memory policy, and interoperability are documented in
//! `neojs-core/src/crypto_shamir.rs`. Shamir shares are bytewise compatible
//! with the Neotales PLAB and rust-libraries encodings, are not authenticated,
//! and cannot detect a wrong reconstruction. `SharedArrayBuffer`-backed inputs
//! are rejected because a copy is not one coherent snapshot. Returned share and
//! secret bytes use V8 backing stores that are cleared when V8 releases them;
//! JavaScript-made copies remain outside that guarantee.
//!
//! `stdio` is a synchronous byte-stream foundation over bounded memory:
//! `Readable` reads from an owned snapshot of a `Uint8Array`, `Writable`
//! appends to an owned bounded buffer, and the shared `stdout` and `stderr`
//! writers write to the current process's standard output and standard error.
//! It exposes `Readable`, `Writable`, `stdout`, `stderr`, and `MAX_BYTES`.
//! There is no stdin, file, process, or environment access, and writing to the
//! two standard descriptors adds no capability beyond the console. It is not
//! Node's `stream` or `process` module and not the WHATWG Streams API, and it
//! promises no such compatibility. There is no asynchronous or callback form:
//! `writeSync` on a host stream blocks the isolate until the operating system
//! accepts the bytes or reports an error. See `neojs-core/src/stdio.rs` for the
//! ownership, partial-read, end-of-file, close, error, and size contracts.
//!
//! `os` is two frozen strings describing the target the runtime was compiled
//! for: `platform` is `std::env::consts::OS` and `arch` is
//! `std::env::consts::ARCH`. They use the compiler's vocabulary, not Node's
//! (`macos`, not `darwin`; `x86_64` or `aarch64`, not `x64` or `arm64`), and
//! describe the build target rather than probing the running host. The module
//! calls no host op and reads no environment variable, OS-release file,
//! registry, sysctl value, file, or process table. neojs installs no `os`
//! global. `version`, `codename`, `variant`, user, keyring, and scheduler
//! surfaces are not implemented.
//!
//! `uuid` generates RFC 9562 version 4 and version 7 UUIDs as canonical
//! lowercase text. `v4` is 122 random bits with no timestamp or order. `v7`
//! embeds the current Unix time in milliseconds in its first 48 bits, followed
//! by a random payload; values from different milliseconds sort by that
//! timestamp when the clock moves forward, but values within one millisecond
//! are randomly ordered and a backward clock step can break the order. The
//! module promises no monotonicity. Both functions are synchronous, take no
//! arguments, and draw entirely from the operating system CSPRNG through
//! fallible `getrandom`, with no insecure fallback. A time before the Unix epoch
//! or beyond the 48-bit field fails with `NEO_UUID_CLOCK`; an entropy failure
//! fails with `NEO_UUID_RANDOM`. The layout and formatting use the `uuid` crate's
//! `Builder`; its panic-on-entropy-failure convenience constructors are not
//! used. Parsing, validation, timestamp extraction, and versions other than 4
//! and 7 are not implemented. The `uuid/v4` and `uuid/v7` subpath modules
//! export the same functions as both the named and default export. See
//! `neojs-core/src/uuid.rs` for the bit layout, ordering limits, and error
//! contract.
//!
//! `strings` is plain ECMAScript: folded matching (`equalFold`,
//! `startsWithFold`, `endsWithFold`, `indexOfFold`, `lastIndexOfFold`) and
//! trimming of a caller-supplied code point set (`trim`, `trimStart`,
//! `trimEnd`). Folding is ECMAScript's simple case folding, decided per code
//! point by V8's own RegExp `/iu` canonicalization, so it ships no Unicode
//! table, is locale-independent, and follows the Unicode data in the V8 build.
//! It is not `toLowerCase()` comparison: `"K"` equals U+212A KELVIN SIGN,
//! `"s"` equals U+017F LONG S, `"I"` does not equal U+0131 DOTLESS I, and
//! `"ss"` does not equal `"ß"`. Indices are UTF-16 code unit offsets like
//! `String.prototype.indexOf`, and overlapping occurrences are found. Searching
//! is linear in the text plus the search string (KMP over simple-fold
//! equivalence) and uses O(search) extra memory, so a repeated prefix cannot
//! make it quadratic. `lastIndexOfFold` stops as soon as no match can begin at
//! or before its limit, so a small limit does not scan a long tail. A lone
//! surrogate is compared as itself and never matches half of a pair. With no
//! `chars`, trimming is exactly `String.prototype.trim`, so it uses
//! JavaScript's whitespace set rather than `neo:chars`' Go-derived
//! `isSpace`. The module does not coerce arguments and calls no host op; it
//! adds no capability beyond the strings it is given.
//!
//! `ssh` is a synchronous command-execution slice over the MIT-licensed
//! `neotales-ssh` crate: `connect` dials one host, completes the SSH handshake
//! and exactly one of password, in-memory-private-key, or explicit Unix-agent
//! authentication, and `exec` runs one command and captures bounded stdout and
//! its exit status until the channel closes. Trust is mandatory and fail-closed:
//! exactly one of an explicit Ed25519 host-key `Uint8Array` or an explicit
//! `known_hosts` text is required, and there is no unknown-host mode. In
//! password and private-key modes the module reads no file, environment
//! variable, or agent. `agent: true` reads `SSH_AUTH_SOCK` once, only after the
//! host key is trusted, tries at most 16 identities, and never forwards the
//! agent; Windows agent pipes are rejected. The Unix socket connect may block,
//! and the backend's 30-second agent read/write timeout is inactivity-based,
//! not a hard wall clock. Sessions are closed before `exec`
//! returns, and a fatal `exec` failure closes the connection. It is not a
//! shell, PTY, SCP, SFTP, port forwarder, or agent client, and it is not a
//! permission system. Both `connect` and `exec` block the isolate and have
//! documented deadlines; there is no asynchronous or callback form. See
//! `neojs-core/src/ssh.rs` for the trust, credential, deadline, bound, and
//! error contracts.
//!
//! `path` is lexical POSIX and Windows path manipulation adapted from NeoLiteJS
//! `neo:path` and `@neotales/path`. It exposes frozen `posix` and `win32`
//! namespaces plus host-selected `join`, `normalize`, `basename`, `dirname`,
//! `extname`, `isAbsolute`, `parse`, `format`, `common`, `resolve`, `relative`,
//! `toNamespacedPath`, `SEPARATOR`, `DELIMITER`, and `SEPARATOR_PATTERN`. The
//! only host read is the process working directory, and only when `resolve`,
//! `relative`, or Windows `toNamespacedPath` lacks a complete explicit base. It
//! does not read the filesystem, environment, network, or process table, and it
//! is not a containment check. Glob and file URL helpers are deliberately
//! absent. See `neojs-core/src/path.rs`, the path tests, and
//! `neojs-core/THIRD_PARTY_NOTICES.md` for the cwd, error, security, and
//! provenance contracts.
//!
//! An ES module namespace object is not the registry object. For `age`,
//! `assert`, `chars`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`, and
//! `strings`, both paths expose the same classes, functions, and shared stream
//! objects;
//! the
//! namespace is a separate container over them. For `assert`, the registry
//! object is the callable function, and the module also has a default export
//! whose value is that function. For `console`, the registry object is the
//! console object the runtime installed, and the default export's value is that
//! object.
//! The named exports are that
//! object's method values when the module is
//! evaluated, so a method replaced before then is what they capture; later
//! replacements are not followed. Neither path follows a replacement of the
//! whole `globalThis.console` property. It does not provide other `neo:` or
//! `node:` import specifiers; `node:` modules, including `node:test`, remain
//! rejected while no explicit Node-module opt-in exists. CommonJS (`.cjs` and
//! `.cts` files), remote imports unless [`Runtime::with_remote`] enabled them,
//! module type attributes, and a permission system are not implemented. See the
//! repository README for the full list of limits.
//!
//! A [`Runtime`] owns two Tokio runtimes: a multi-thread driver with one worker
//! thread for V8's delayed foreground tasks and `crypto`'s bounded blocking
//! pool, and a current-thread runtime that drives the isolate for `run_file`
//! and hosts deno_core's async-op driver. The isolate is created inside the
//! multi-thread driver's context and both runtimes live until the isolate is
//! gone, so `run_file` and `run_file_async` do not depend on an ambient Tokio
//! runtime. `run_file_async` can be polled on any executor. It does not offload
//! work: entry and imported files are read and type-stripped synchronously on
//! the polling thread, and module compilation and evaluation run there too.
//! Asynchronous `crypto` calls made through `run_file_async` need a Tokio
//! current-thread runtime to host deno_core's async-op driver; without one they
//! fail with `NEO_CRYPTO_NO_EXECUTOR`. Cancelling the future after preflight
//! leaves the runtime consumed.
//!
//! Entry and imported `.ts` and `.mts` files run by erasing TypeScript syntax.
//! Only the erasable subset is supported: constructs that need a transform, such
//! as `enum`, `namespace`, and parameter properties, are rejected with a reason.
//! There is no type checking, so a type error is not reported and the program
//! runs with its runtime values. `.tsx`, `.jsx`, `.cts`, and declaration files
//! are rejected by extension. Erased text is replaced by spaces, so V8 stack
//! traces point at the original TypeScript line and column.
//!
//! Module loading uses the process's filesystem authority and, when remote
//! loading is enabled, its network authority. neojs is not a sandbox and offers
//! no host allowlist or private-address protection.
//!
//! # Example
//!
//! ```no_run
//! use neojs_core::Runtime;
//!
//! # fn main() -> Result<(), neojs_core::Error> {
//! let mut runtime = Runtime::new()?;
//! runtime.run_file("hello.js")?;
//! # Ok(())
//! # }
//! ```

mod age;
mod assert;
mod builtins;
mod crypto;
mod crypto_cipher;
mod crypto_hash;
mod crypto_shamir;
mod error;
// Private groundwork for native `jsr:` imports. `Runtime::new` does not install
// a JSR-capable loader, so every public `jsr:` import is still rejected; see
// `src/jsr.rs` for the implemented core and the work that remains.
#[allow(dead_code)]
mod jsr;
// Test-only loader wiring for the private JSR resolver. It is not compiled
// into a normal build, and `Runtime::new` never installs it, so public `jsr:`
// imports remain rejected until an HTTPS transport and cache policy land.
#[cfg(test)]
mod jsr_loader;
mod loader;
mod path;
mod remote;
mod runtime;
mod ssh;
mod stdio;
mod typescript;
mod uuid;

pub use error::Error;
pub use remote::RemoteOptions;
pub use runtime::Runtime;
pub use runtime::TestReport;

/// Test-only hooks. Compiled only with the off-by-default `test-support`
/// feature, which a normal build never enables.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support {
    /// A scripted response for the test-only remote transport hook.
    ///
    /// Pass this type to [`crate::Runtime::with_remote_transport`] to drive
    /// remote loading without a network. It is not part of the stable API.
    pub use crate::remote::TestRemoteResponse;

    /// Make the next required CLOSE reply fail as a transport error.
    ///
    /// This exercises the caller-facing path from `Connection::exec` through
    /// the host op and the JavaScript `closed` state without relying on a
    /// racy TCP reset. It has no effect unless the feature is enabled.
    pub fn fail_next_peer_close_reply() {
        crate::ssh::FAIL_NEXT_PEER_CLOSE_REPLY.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Install a deterministic cwd sequence for `f` and return its call count.
    ///
    /// Used by the path tests to prove that `relative` takes one lazy cwd
    /// snapshot per call. Compiled only with `test-support`.
    pub use crate::path::with_cwd_sequence;

    /// Test-only gate that holds a real blocking hash worker after it starts.
    pub use crate::crypto_hash::CancellationGate;

    /// Install a cancellation gate for a runtime created inside `f`.
    pub use crate::crypto_hash::with_cancellation_gate;

    /// Make the next bcrypt salt draw fail with `NEO_CRYPTO_RANDOM`.
    ///
    /// The injection is consumed exactly once and is compiled only with the
    /// off-by-default `test-support` feature. It exists so the synchronous and
    /// asynchronous error paths can be tested deterministically without a
    /// production hook.
    pub fn fail_next_bcrypt_random() {
        crate::crypto_hash::fail_next_bcrypt_random();
    }

    /// Make the next Shamir split's noise draw fail with `NEO_CRYPTO_RANDOM`.
    ///
    /// The injection is consumed exactly once and is compiled only with the
    /// off-by-default `test-support` feature. It exists so the synchronous and
    /// asynchronous error paths can be tested deterministically without a
    /// production hook.
    pub fn fail_next_shamir_random() {
        crate::crypto_shamir::fail_next_shamir_random();
    }

    /// Make the next UUID entropy draw fail with `NEO_UUID_RANDOM`.
    ///
    /// The injection is consumed exactly once and is compiled only with the
    /// off-by-default `test-support` feature. It exists so the JavaScript error
    /// path can be tested deterministically without a production hook.
    pub fn fail_next_uuid_random() {
        crate::uuid::fail_next_uuid_random();
    }

    /// Count Shamir V8 backing stores cleared by the zeroizing deleter.
    ///
    /// Used to prove that dropped share arrays and secrets are cleared when
    /// V8 releases them. Compiled only with `test-support`.
    pub fn zeroized_shamir_buffers() -> usize {
        crate::crypto_shamir::zeroized_v8_buffers()
    }

    /// Count sensitive Shamir worker buffers cleared at the wipe boundary.
    ///
    /// The counter increments only after the full-capacity zeroization, so it
    /// proves an abandoned worker result reached the actual clearing code when
    /// the runtime was torn down.
    pub fn zeroized_shamir_worker_results() -> usize {
        crate::crypto_shamir::zeroized_worker_results()
    }
}
