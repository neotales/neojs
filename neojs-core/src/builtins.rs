//! JavaScript evaluated in every runtime before the entry module, and the
//! synthetic ECMAScript modules that import specifiers reach.
//!
//! The scripts here install globals that neojs supplies: currently
//! [`neo_shim`], which defines the `Neo` object, its cached built-in modules,
//! and the introspection surface `Neo.engine` and
//! `Neo.getBuiltInModulesList`. `chars`, `console`, `os`, `slices`, `strings`,
//! and `test` are
//! plain ECMAScript (the `os` module contains two build-target constants
//! substituted by [`neo_shim`]); `assert` calls one private host op registered
//! by [`crate::assert`] to observe a genuine native Promise. `cron` calls six
//! private host ops registered by [`crate::cron`]: it parses a UTC five-field
//! expression, computes the next occurrence, sleeps on the runtime's Tokio
//! timer, and cancels a job. An undisposed job keeps the event loop alive; one
//! callback runs at a time, and overlap, missed ticks, and callback failures are
//! reported through an optional observer. It adds no filesystem, environment,
//! network, or process capability and has no local-time, DST, time-zone, or
//! fake-timer surface.
//!
//! `chars` is not written in this crate. Its implementation is the
//! engine-neutral expression in [`neojs_builtins::chars`], the same expression
//! the QuickJS runtime evaluates, and [`neo_shim`] splices it in when the local
//! `neo-chars` feature is enabled. The V8 adapter below still serves this branch
//! base's bare registry key and bare import spelling; the canonical-registry
//! change in flight will switch both to `neo:chars`. The expression is evaluated
//! once, before `slices`, so the V8 `chars` object, the `slices` capture of
//! `chars.toUpper`/`chars.toLower`, the registry property, and the module
//! namespace's named exports all refer to the same function objects. The ES
//! module namespace itself is a distinct object from the registry object.
//! `slices` is implemented separately in `builtins/slices.impl.js`; that file is
//! compiled in only with `neo-slices`, so a build without the feature does not
//! carry its implementation bytes. `neo-slices` implies `neo-chars`.
//! `crypto` calls private host ops registered by [`crate::crypto`],
//! [`crate::crypto_cipher`], [`crate::crypto_hash`], and
//! [`crate::crypto_shamir`]; it
//! adds the operating system CSPRNG, the NEO1 AES-256-GCM envelope, bounded
//! stateful AES-256-CBC/AES-256-GCM ciphers, SHA-2 hashing and HMAC, PBKDF2,
//! bcrypt, and GF(256) Shamir secret sharing, but no permission, filesystem
//! access, or sandbox. `crypto`'s `hashAsync`, `hmacAsync`, `pbkdf2Async`,
//! `bcryptHashAsync`, `bcryptVerifyAsync`, `shamirSplitAsync`, and
//! `shamirCombineAsync` offload their work to the runtime's bounded blocking
//! pool. Shamir shares are not authenticated and a wrong reconstruction is not
//! detected. `age` calls private host
//! ops registered by [`crate::age`]; it adds binary age v1 encryption for X25519
//! keys, but no permission, file access, or sandbox. `ssh` calls private host
//! ops registered by [`crate::ssh`]; it adds outbound TCP and SSH command
//! execution with a mandatory host-key pin or `known_hosts` document. An
//! explicit Unix-agent opt-in reads `SSH_AUTH_SOCK` after host verification;
//! other modes read no environment and no agent. Its socket connect may block
//! and its backend timeout is inactivity-based, not a hard wall clock. It adds
//! no filesystem or permission capability and never forwards an agent.
//! `uuid` calls two private host ops registered by [`crate::uuid`]: it
//! generates RFC 9562 version 4 and version 7 UUIDs from the operating system
//! CSPRNG and returns canonical lowercase text. It has no insecure fallback and
//! promises no monotonic order; the version 7 timestamp is the only ordering
//! component. The `uuid/v4` and `uuid/v7` subpath modules expose one generator
//! each as both the named and default export, backed by the same function
//! objects. Parsing, validation, and other versions are not implemented.
//! `strings` is plain ECMAScript: case-insensitive folded matching
//! (`equalFold`, `startsWithFold`, `endsWithFold`, `indexOfFold`,
//! `lastIndexOfFold`) and trimming of a caller-supplied code point set. The
//! search functions are linear in the text plus the needle and use O(needle)
//! extra memory; `lastIndexOfFold` also stops once no match can begin at or
//! before its limit. It returns UTF-16 code unit indices like
//! `String.prototype`, calls no host op,
//! and adds no capability. Its folding is ECMAScript simple case folding,
//! decided by the engine's own RegExp `/iu` canonicalization, so it is
//! locale-independent and follows the Unicode data in the V8 build.
//! `slices` is plain ECMAScript too: fixed-range, live, mutable and read-only
//! views over a JS Array or a genuine typed array, plus a `CharSlice` over a
//! caller-owned `Uint32Array` of Unicode code points. Construction, nested
//! `slice`, `at`, `set`, `indexOf`, `includes`, and iteration do not copy;
//! `copy`, `toArray`, and `toString` are the explicit snapshots. A view aliases
//! the target, so a Proxy's traps and an accessor element run, and it is not a
//! snapshot, a sandbox, or a cross-realm/cross-thread zero-copy mechanism. JS
//! strings are immutable: `CharSlice.fromString` allocates one code point
//! buffer and a view over it, `slice` allocates only a view/state object and no
//! character buffer, `toUpper`/`toLower` allocate no new character buffer but
//! may allocate a temporary string per mapped code point, and `toString`
//! allocates the result string and bounded chunk temporaries rather than a
//! string per code point.
//! `stdio` is plain
//! ECMAScript plus host ops registered by [`crate::stdio`]: bounded in-memory
//! byte streams and writes to the current process's standard output and standard
//! error. It adds no filesystem, process, environment, or stdin capability. The
//! `console` module is the realm's installed console object, not a separate
//! console or an isolation boundary. `path` is lexical POSIX and Windows path
//! manipulation adapted from NeoLiteJS and `@neotales/path`; its only host read
//! is the process working directory, through [`crate::path`], and only when a
//! resolve/relative operation lacks an explicit absolute base. It adds no
//! filesystem, environment, network, or process capability and is not a
//! containment check.
//!
//! [`AGE_MODULE`], [`ASSERT_MODULE`], `CHARS_MODULE`, [`CONSOLE_MODULE`],
//! [`CRON_MODULE`], [`CRYPTO_MODULE`], [`OS_MODULE`], [`PATH_MODULE`],
//! `SLICES_MODULE`, [`SSH_MODULE`], [`STDIO_MODULE`], [`STRINGS_MODULE`],
//! [`TEST_MODULE`], [`UUID_MODULE`], [`UUID_V4_MODULE`], and [`UUID_V7_MODULE`]
//! are not evaluated as scripts. They are the sources returned by the module
//! loader for the built-in specifiers [`AGE_SPECIFIER`], [`ASSERT_SPECIFIER`],
//! `CHARS_SPECIFIER`, [`CONSOLE_SPECIFIER`], [`CRON_SPECIFIER`],
//! [`CRYPTO_SPECIFIER`], [`OS_SPECIFIER`], [`PATH_SPECIFIER`],
//! `SLICES_SPECIFIER`, [`SSH_SPECIFIER`], [`STDIO_SPECIFIER`],
//! [`STRINGS_SPECIFIER`], [`TEST_SPECIFIER`], [`UUID_SPECIFIER`],
//! [`UUID_V4_SPECIFIER`], and [`UUID_V7_SPECIFIER`], and for their bare names.
//! `CHARS_MODULE` and `SLICES_MODULE` exist only with `neo-chars` and
//! `neo-slices`; with the features off their names, sources, registry entries,
//! and loader entries are absent. All read the public module objects installed by
//! [`neo_shim`]; each named export is the same function or class object as the
//! matching registry property, while the ES module namespace is a distinct
//! object. `assert` is the frozen callable
//! `assert` function itself, with its named assertions as properties; it also
//! has a default export in its module namespace. `age`, `chars`, `cron`,
//! `slices`, `strings`, `test`, and `uuid` are frozen module objects whose named
//! exports are their function or class objects, plus `test`'s constants.
//! `console` is the console
//! object the runtime installed: the
//! default export's value is that object, and the named exports are its method
//! values when the module is evaluated. `crypto`, `path`, `ssh`, and `stdio` are
//! frozen module objects whose named exports are their synchronous classes,
//! functions, constants, and shared stream objects; `os` is a frozen module
//! object whose named exports are two build-target strings; `uuid` is a frozen
//! module object whose named exports are the two generator functions, and
//! `uuid/v4`/`uuid/v7` are frozen module objects that expose one generator as
//! both the named and default export. In every
//! case the ES module namespace is a separate object from the registry object.

use deno_core::url::Url;

/// Installs the global `Neo` object and the built-in module registry.
///
/// Evaluation happens once per realm. `Neo.engine` is `"neo"` for this V8
/// runtime. `Neo.getBuiltInModulesList` returns one frozen array, built from the
/// registry in insertion order, with each built-in's bare name followed by its
/// canonical `neo:` specifier; repeated calls return that same array. The `age`,
/// `assert`, `chars`, `cron`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`,
/// `strings`, `test`, `uuid`,
/// `uuid/v4`, and `uuid/v7` module objects are frozen and
/// cached, so repeated `Neo.getBuiltInModule` calls return the same object for
/// each. The `console` module object is the console object the runtime
/// installed, captured after the console shim runs. At installation time the
/// registry, the module bindings, and the global console share the same
/// function objects; a method replaced before the console module is evaluated
/// is what its named exports capture. Names that are not implemented return
/// `undefined`.
const NEO_SHIM_SOURCE: &str = include_str!("builtins/neo.js");

/// The marker replaced with the shared `chars` declaration.
///
/// The implementation itself lives in the engine-neutral `neojs-builtins`
/// crate. The marker is the comment token on its own line, so removing it leaves
/// no half-statement behind.
const CHARS_DECLARATION_TOKEN: &str = "// __NEO_CHARS_DECLARATION__";

/// The marker replaced with the `chars` registry entry.
const CHARS_REGISTRY_TOKEN: &str = "// __NEO_CHARS_REGISTRY_ENTRY__";

/// The marker replaced with the separate `slices` implementation source.
///
/// The implementation is not part of [`NEO_SHIM_SOURCE`]. It is compiled in
/// only when `neo-slices` is enabled, so a disabled build does not carry its
/// bytes.
const SLICES_IMPL_TOKEN: &str = "// __NEO_SLICES_IMPL__";

/// The `slices` implementation, compiled only when `neo-slices` is enabled.
#[cfg(feature = "neo-slices")]
const SLICES_IMPL_SOURCE: &str = include_str!("builtins/slices.impl.js");

/// The marker replaced with the `slices` registry entry.
const SLICES_REGISTRY_TOKEN: &str = "// __NEO_SLICES_REGISTRY_ENTRY__";

/// The placeholder replaced with the build target's `std::env::consts::OS`.
const OS_PLATFORM_TOKEN: &str = "__NEO_OS_PLATFORM__";

/// The placeholder replaced with the build target's `std::env::consts::ARCH`.
const OS_ARCH_TOKEN: &str = "__NEO_OS_ARCH__";

/// Returns the shim with the build target's `os` constants substituted and the
/// selected built-in source and registration spliced in.
///
/// [`NEO_SHIM_SOURCE`] holds two placeholders, one per `neo:os` value. They are
/// replaced with ECMAScript string literals for this build's
/// `std::env::consts::OS` and `std::env::consts::ARCH`, so the module reports the
/// target the runtime was compiled for and reads no host state at runtime.
///
/// `neo-chars` replaces the declaration marker with
/// [`neojs_builtins::chars::IMPLEMENTATION`], the same source the QuickJS shim
/// evaluates, and adds the registry entry. `neo-slices` splices the separate
/// `slices.impl.js` source and registry entry; it implies `neo-chars` because
/// `CharSlice` captures `chars.toUpper` and `chars.toLower`. A disabled module's
/// source, registry entry, loader entry, and name are all absent. The `slices`
/// source is not compiled into a disabled build at all. The default V8 features
/// keep both present, so the default surface is unchanged.
pub(crate) fn neo_shim() -> String {
    let shim = NEO_SHIM_SOURCE
        .replace(OS_PLATFORM_TOKEN, &js_string(std::env::consts::OS))
        .replace(OS_ARCH_TOKEN, &js_string(std::env::consts::ARCH));

    #[cfg(feature = "neo-chars")]
    let shim = shim
        .replace(
            CHARS_DECLARATION_TOKEN,
            &format!("const chars = {};", neojs_builtins::chars::IMPLEMENTATION),
        )
        .replace(CHARS_REGISTRY_TOKEN, "[\"chars\", chars],");
    #[cfg(not(feature = "neo-chars"))]
    let shim = shim
        .replace(CHARS_DECLARATION_TOKEN, "")
        .replace(CHARS_REGISTRY_TOKEN, "");

    #[cfg(feature = "neo-slices")]
    let shim = shim
        .replace(SLICES_IMPL_TOKEN, SLICES_IMPL_SOURCE)
        .replace(SLICES_REGISTRY_TOKEN, "[\"slices\", slices],");
    #[cfg(not(feature = "neo-slices"))]
    let shim = shim
        .replace(SLICES_IMPL_TOKEN, "")
        .replace(SLICES_REGISTRY_TOKEN, "");

    debug_assert!(!shim.contains(CHARS_DECLARATION_TOKEN));
    debug_assert!(!shim.contains(CHARS_REGISTRY_TOKEN));
    debug_assert!(!shim.contains(SLICES_IMPL_TOKEN));
    debug_assert!(!shim.contains(SLICES_REGISTRY_TOKEN));
    shim
}

/// Encodes `value` as a double-quoted ECMAScript string literal.
///
/// The build-target constants are plain ASCII identifiers, but the helper
/// escapes everything that would end or alter a literal so the substitution
/// cannot produce invalid or misleading source.
fn js_string(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            character if character.is_control() => {
                literal.push_str(&format!("\\u{{{:x}}}", character as u32));
            }
            character => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

/// The canonical module specifier for the `age` built-in.
///
/// Both `"age"` and `"neo:age"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never read
/// from disk.
pub(crate) const AGE_SPECIFIER: &str = "neo:age";

/// The ECMAScript module source served for [`AGE_SPECIFIER`].
///
/// It re-exports the function objects from the public `age` module object, so
/// the module and `Neo.getBuiltInModule("age")` share function identity; the ES module namespace is a distinct object. The functions
/// call host ops registered by [`crate::age`].
const AGE_MODULE: &str = include_str!("builtins/age.esm.js");

/// The canonical module specifier for the `assert` built-in.
///
/// Both `"assert"` and `"neo:assert"` resolve to this URL, so the engine caches
/// one module instance for both spellings. It is not a `file:` URL and is never
/// read from disk.
pub(crate) const ASSERT_SPECIFIER: &str = "neo:assert";

/// The ECMAScript module source served for [`ASSERT_SPECIFIER`].
///
/// It re-exports the callable `assert` function and its named assertions from
/// the public `assert` module object, and also exposes that function as the
/// default export. The implementations are in [`neo_shim`]; the only host op
/// called observes a genuine native Promise and is registered by
/// [`crate::assert`].
const ASSERT_MODULE: &str = include_str!("builtins/assert.esm.js");

/// The canonical module specifier for the `chars` built-in.
///
/// Both `"chars"` and `"neo:chars"` resolve to this URL when `neo-chars` is
/// enabled, so the engine caches one module instance for both spellings. It is
/// not a `file:` URL and is never read from disk. With the feature off, the
/// specifier is not in the name or source table and the loader rejects it.
#[cfg(feature = "neo-chars")]
pub(crate) const CHARS_SPECIFIER: &str = "neo:chars";

/// The ECMAScript module source served for [`CHARS_SPECIFIER`].
///
/// It re-exports the function objects from the public `chars` module object, so
/// each named export is the same function object as the matching registry
/// property. The ES module namespace is a separate object from the registry
/// object. The implementation is the shared `neojs-builtins` expression.
#[cfg(feature = "neo-chars")]
const CHARS_MODULE: &str = include_str!("builtins/chars.esm.js");

/// The canonical module specifier for the `console` built-in.
///
/// Both `"console"` and `"neo:console"` resolve to this URL, so the engine
/// caches one module instance for both spellings. It is not a `file:` URL and
/// is never read from disk.
pub(crate) const CONSOLE_SPECIFIER: &str = "neo:console";

/// The ECMAScript module source served for [`CONSOLE_SPECIFIER`].
///
/// It exports the console object the runtime installed as the default export
/// and its five neojs-defined method values as named exports. The method values
/// are read when the module is evaluated, so a replacement made before then is
/// captured. The ES module namespace is a separate object from the registry.
const CONSOLE_MODULE: &str = include_str!("builtins/console.esm.js");

/// The canonical module specifier for the `cron` built-in.
///
/// Both `"cron"` and `"neo:cron"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never read
/// from disk.
pub(crate) const CRON_SPECIFIER: &str = "neo:cron";

/// The ECMAScript module source served for [`CRON_SPECIFIER`].
///
/// It re-exports `cron`, `next`, and `CronError` from the public `cron` module
/// object, so the module and `Neo.getBuiltInModule("cron")` share function identity; the ES module namespace is a distinct object. The
/// functions call host ops registered by [`crate::cron`]; scheduling uses the
/// runtime's own Tokio timer and a pending op keeps the event loop alive until
/// the job is disposed.
const CRON_MODULE: &str = include_str!("builtins/cron.esm.js");

/// The canonical module specifier for the `crypto` built-in.
///
/// Both `"crypto"` and `"neo:crypto"` resolve to this URL, so the engine caches
/// one module instance for both spellings. It is not a `file:` URL and is never
/// read from disk.
pub(crate) const CRYPTO_SPECIFIER: &str = "neo:crypto";

/// The ECMAScript module source served for [`CRYPTO_SPECIFIER`].
///
/// It re-exports the function objects from the public `crypto` module object,
/// so the module and `Neo.getBuiltInModule("crypto")` share function identity; the ES module namespace is a distinct object. The
/// functions call host ops registered by [`crate::crypto`],
/// [`crate::crypto_cipher`], [`crate::crypto_hash`], and
/// [`crate::crypto_shamir`].
const CRYPTO_MODULE: &str = include_str!("builtins/crypto.esm.js");

/// The canonical module specifier for the `os` built-in.
///
/// Both `"os"` and `"neo:os"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never
/// read from disk.
pub(crate) const OS_SPECIFIER: &str = "neo:os";

/// The ECMAScript module source served for [`OS_SPECIFIER`].
///
/// It re-exports the two build-target constants from the public `os` module
/// object, so the module and `Neo.getBuiltInModule("os")` share function identity; the ES module namespace is a distinct object. The
/// values are substituted into [`neo_shim`]; the module calls no host op and
/// reads no environment variable or operating-system file.
const OS_MODULE: &str = include_str!("builtins/os.esm.js");

/// The canonical module specifier for the `ssh` built-in.
///
/// Both `"ssh"` and `"neo:ssh"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never
/// read from disk.
pub(crate) const SSH_SPECIFIER: &str = "neo:ssh";

/// The ECMAScript module source served for [`SSH_SPECIFIER`].
///
/// It re-exports the synchronous `connect` function and the documented limits
/// from the public `ssh` module object, so the module and
/// `Neo.getBuiltInModule("ssh")` share function identity; the ES module namespace is a distinct object. `connect` calls host ops
/// registered by [`crate::ssh`]; it requires an explicit host-key pin or
/// `known_hosts` document and never accepts an unknown host key.
const SSH_MODULE: &str = include_str!("builtins/ssh.esm.js");

/// The canonical module specifier for the `stdio` built-in.
///
/// Both `"stdio"` and `"neo:stdio"` resolve to this URL, so the engine caches
/// one module instance for both spellings. It is not a `file:` URL and is never
/// read from disk.
pub(crate) const STDIO_SPECIFIER: &str = "neo:stdio";

/// The ECMAScript module source served for [`STDIO_SPECIFIER`].
///
/// It re-exports the classes, constant, and shared stream objects from the
/// public `stdio` module object, so the module and
/// `Neo.getBuiltInModule("stdio")` share function identity; the ES module namespace is a distinct object. The stream methods call host
/// ops registered by [`crate::stdio`].
const STDIO_MODULE: &str = include_str!("builtins/stdio.esm.js");

/// The canonical module specifier for the `path` built-in.
///
/// Both `"path"` and `"neo:path"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never read
/// from disk.
pub(crate) const PATH_SPECIFIER: &str = "neo:path";

/// The ECMAScript module source served for [`PATH_SPECIFIER`].
///
/// It re-exports the frozen POSIX and Windows namespaces, their functions, and
/// the host-selected aliases from the public `path` module object, so the module
/// and `Neo.getBuiltInModule("path")` share function identity; the ES module namespace is a distinct object. The functions are lexical;
/// their host ops report the build dialect and the process working directory,
/// the latter only when an operation lacks an explicit absolute base.
const PATH_MODULE: &str = include_str!("builtins/path.esm.js");

/// The canonical module specifier for the `slices` built-in.
///
/// Both `"slices"` and `"neo:slices"` resolve to this URL when `neo-slices` is
/// enabled. It requires `neo-chars` because `CharSlice` captures
/// `chars.toUpper` and `chars.toLower`. It is not a `file:` URL and is never
/// read from disk.
#[cfg(feature = "neo-slices")]
pub(crate) const SLICES_SPECIFIER: &str = "neo:slices";

/// The ECMAScript module source served for [`SLICES_SPECIFIER`].
///
/// It re-exports the view classes and factories from the public `slices` module
/// object, so each named export is the same function or class object as the
/// matching registry property. The ES module namespace is a separate object from
/// the registry object. The implementations are plain ECMAScript in
/// [`neo_shim`]; no host op is called. Views alias the caller's Array or typed
/// array; they are not copies, a sandbox, or a cross-realm/cross-thread
/// zero-copy mechanism.
#[cfg(feature = "neo-slices")]
const SLICES_MODULE: &str = include_str!("builtins/slices.esm.js");

/// The canonical module specifier for the `strings` built-in.
///
/// Both `"strings"` and `"neo:strings"` resolve to this URL, so the engine
/// caches one module instance for both spellings. It is not a `file:` URL and
/// is never read from disk.
pub(crate) const STRINGS_SPECIFIER: &str = "neo:strings";

/// The ECMAScript module source served for [`STRINGS_SPECIFIER`].
///
/// It re-exports the function objects from the public `strings` module object,
/// so the module and `Neo.getBuiltInModule("strings")` share function identity; the ES module namespace is a distinct object. The
/// implementations are plain ECMAScript in [`neo_shim`]; no host op is called.
const STRINGS_MODULE: &str = include_str!("builtins/strings.esm.js");

/// The canonical module specifier for the `test` built-in.
///
/// Both `"test"` and `"neo:test"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never
/// read from disk. `node:test` is deliberately not an alias while neojs has no
/// explicit Node-module opt-in.
pub(crate) const TEST_SPECIFIER: &str = "neo:test";

/// The ECMAScript module source served for [`TEST_SPECIFIER`].
///
/// It re-exports the callable `test` function and the runner helpers from the
/// public `test` module object, so the module and
/// `Neo.getBuiltInModule("test")` share function identity; the ES module namespace is a distinct object. The implementation is in
/// [`neo_shim`]; it calls no host op.
const TEST_MODULE: &str = include_str!("builtins/test.esm.js");

/// The canonical module specifier for the `uuid` built-in.
///
/// Both `"uuid"` and `"neo:uuid"` resolve to this URL, so the engine caches one
/// module instance for both spellings. It is not a `file:` URL and is never read
/// from disk.
pub(crate) const UUID_SPECIFIER: &str = "neo:uuid";

/// The ECMAScript module source served for [`UUID_SPECIFIER`].
///
/// It re-exports the version 4 and version 7 generators from the public `uuid`
/// module object, so the module and `Neo.getBuiltInModule("uuid")` share
/// identity. The functions call host ops registered by [`crate::uuid`]; they
/// draw from the operating system CSPRNG and return canonical lowercase text.
const UUID_MODULE: &str = include_str!("builtins/uuid.esm.js");

/// The canonical module specifier for the version 4 UUID built-in.
///
/// Both `"uuid/v4"` and `"neo:uuid/v4"` resolve to this URL. It is a thin
/// module over the same function as `uuid.v4`, with that function as both the
/// named `v4` export and the default export. It is not read from disk.
pub(crate) const UUID_V4_SPECIFIER: &str = "neo:uuid/v4";

/// The ECMAScript module source served for [`UUID_V4_SPECIFIER`].
///
/// It re-exports `uuid.v4` as the named `v4` export and as the default export,
/// so all spellings share one function object.
const UUID_V4_MODULE: &str = include_str!("builtins/uuid-v4.esm.js");

/// The canonical module specifier for the version 7 UUID built-in.
///
/// Both `"uuid/v7"` and `"neo:uuid/v7"` resolve to this URL. It is a thin
/// module over the same function as `uuid.v7`, with that function as both the
/// named `v7` export and the default export. It is not read from disk.
pub(crate) const UUID_V7_SPECIFIER: &str = "neo:uuid/v7";

/// The ECMAScript module source served for [`UUID_V7_SPECIFIER`].
///
/// It re-exports `uuid.v7` as the named `v7` export and as the default export,
/// so all spellings share one function object.
const UUID_V7_MODULE: &str = include_str!("builtins/uuid-v7.esm.js");

/// Registry name and canonical specifier for each implemented built-in module.
///
/// The loader serves one module instance per canonical specifier, and the error
/// messages list the same table, so adding a module in one place is enough.
const BUILTINS: &[(&str, &str)] = &[
    ("age", AGE_SPECIFIER),
    ("assert", ASSERT_SPECIFIER),
    #[cfg(feature = "neo-chars")]
    ("chars", CHARS_SPECIFIER),
    ("console", CONSOLE_SPECIFIER),
    ("cron", CRON_SPECIFIER),
    ("crypto", CRYPTO_SPECIFIER),
    ("os", OS_SPECIFIER),
    ("path", PATH_SPECIFIER),
    #[cfg(feature = "neo-slices")]
    ("slices", SLICES_SPECIFIER),
    ("ssh", SSH_SPECIFIER),
    ("stdio", STDIO_SPECIFIER),
    ("strings", STRINGS_SPECIFIER),
    ("test", TEST_SPECIFIER),
    ("uuid", UUID_SPECIFIER),
    ("uuid/v4", UUID_V4_SPECIFIER),
    ("uuid/v7", UUID_V7_SPECIFIER),
];

/// Returns the canonical specifier for a built-in module name, or `None`.
///
/// This is the name `Neo.getBuiltInModule` takes: a bare module name without
/// the `neo:` prefix.
pub(crate) fn resolve_name(name: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|(registry, _)| *registry == name)
        .map(|(_, canonical)| *canonical)
}

/// Returns the canonical specifier for an import specifier, or `None`.
///
/// Both the bare name and the `neo:`-prefixed name are accepted. A specifier
/// that is not listed is not treated as a module name, and a relative or
/// absolute path is never intercepted.
pub(crate) fn resolve(specifier: &str) -> Option<&'static str> {
    resolve_name(specifier.strip_prefix("neo:").unwrap_or(specifier))
}

/// Returns the module source for a resolved built-in specifier, or `None`.
pub(crate) fn source(specifier: &Url) -> Option<&'static str> {
    match specifier.as_str() {
        AGE_SPECIFIER => Some(AGE_MODULE),
        ASSERT_SPECIFIER => Some(ASSERT_MODULE),
        #[cfg(feature = "neo-chars")]
        CHARS_SPECIFIER => Some(CHARS_MODULE),
        CONSOLE_SPECIFIER => Some(CONSOLE_MODULE),
        CRON_SPECIFIER => Some(CRON_MODULE),
        CRYPTO_SPECIFIER => Some(CRYPTO_MODULE),
        OS_SPECIFIER => Some(OS_MODULE),
        PATH_SPECIFIER => Some(PATH_MODULE),
        #[cfg(feature = "neo-slices")]
        SLICES_SPECIFIER => Some(SLICES_MODULE),
        SSH_SPECIFIER => Some(SSH_MODULE),
        STDIO_SPECIFIER => Some(STDIO_MODULE),
        STRINGS_SPECIFIER => Some(STRINGS_MODULE),
        TEST_SPECIFIER => Some(TEST_MODULE),
        UUID_SPECIFIER => Some(UUID_MODULE),
        UUID_V4_SPECIFIER => Some(UUID_V4_MODULE),
        UUID_V7_SPECIFIER => Some(UUID_V7_MODULE),
        _ => None,
    }
}

/// Human-readable list of implemented built-in specifiers, for error messages.
pub(crate) fn implemented_specifiers() -> String {
    let mut specifiers = Vec::with_capacity(BUILTINS.len() * 2);
    for (registry, canonical) in BUILTINS {
        specifiers.push(*registry);
        specifiers.push(*canonical);
    }
    specifiers.join(", ")
}

#[cfg(test)]
mod tests {
    use super::{
        CHARS_DECLARATION_TOKEN, CHARS_REGISTRY_TOKEN, OS_ARCH_TOKEN, OS_PLATFORM_TOKEN,
        SLICES_IMPL_TOKEN, SLICES_REGISTRY_TOKEN, neo_shim,
    };
    use deno_core::url::Url;

    /// The shim is not valid JavaScript until the placeholders are replaced, so
    /// a missing substitution must fail here rather than at the first
    /// `Runtime::new`. The feature markers are checked the same way: a marker
    /// left in the evaluated shim is a comment, which would silently drop a
    /// module, and a disabled module must leave no source bytes behind.
    #[test]
    fn shim_substitutes_the_build_target_constants_and_selected_modules() {
        let shim = neo_shim();
        assert!(!shim.contains(OS_PLATFORM_TOKEN));
        assert!(!shim.contains(OS_ARCH_TOKEN));
        assert!(!shim.contains(CHARS_DECLARATION_TOKEN));
        assert!(!shim.contains(CHARS_REGISTRY_TOKEN));
        assert!(!shim.contains(SLICES_IMPL_TOKEN));
        assert!(!shim.contains(SLICES_REGISTRY_TOKEN));
        assert!(
            shim.contains(&format!("platform: \"{}\"", std::env::consts::OS)),
            "platform literal is missing from the shim"
        );
        assert!(
            shim.contains(&format!("arch: \"{}\"", std::env::consts::ARCH)),
            "arch literal is missing from the shim"
        );

        #[cfg(feature = "neo-chars")]
        {
            assert!(
                shim.contains("const chars = "),
                "the shared chars implementation was not spliced in"
            );
            assert!(
                shim.contains("(() => {") && shim.contains("return freeze({"),
                "the shared chars implementation is not an install expression"
            );
            assert!(
                shim.contains("const isInteger = Number.isInteger;"),
                "the shared chars implementation did not capture its intrinsics"
            );
            assert!(
                shim.contains("[\"chars\", chars],"),
                "the chars registry entry is missing"
            );
        }
        #[cfg(not(feature = "neo-chars"))]
        {
            assert!(
                !shim.contains("const chars = "),
                "the disabled chars declaration is present"
            );
            assert!(
                !shim.contains("function isChar("),
                "the disabled chars implementation is present"
            );
            assert!(
                !shim.contains("[\"chars\", chars],"),
                "the disabled chars registry entry is present"
            );
        }

        #[cfg(feature = "neo-slices")]
        {
            assert!(
                shim.contains("const slices = (() => {"),
                "the slices implementation is missing"
            );
            assert!(
                shim.contains("[\"slices\", slices],"),
                "the slices registry entry is missing"
            );
        }
        #[cfg(not(feature = "neo-slices"))]
        {
            assert!(
                !shim.contains("const slices = (() => {"),
                "the disabled slices implementation is present"
            );
            assert!(
                !shim.contains("class CharSlice"),
                "the disabled slices source is present"
            );
            assert!(
                !shim.contains("[\"slices\", slices],"),
                "the disabled slices registry entry is present"
            );
        }
    }

    /// A disabled module must be absent from both the name table the loader
    /// consults and the source table it serves. The enabled module is present.
    #[test]
    fn module_tables_follow_the_chars_and_slices_features() {
        let chars = Url::parse("neo:chars").expect("valid chars specifier");
        let slices = Url::parse("neo:slices").expect("valid slices specifier");

        #[cfg(feature = "neo-chars")]
        {
            assert_eq!(super::resolve("neo:chars"), Some(super::CHARS_SPECIFIER));
            assert!(super::source(&chars).is_some());
            assert!(super::implemented_specifiers().contains("neo:chars"));
        }
        #[cfg(not(feature = "neo-chars"))]
        {
            assert_eq!(super::resolve("neo:chars"), None);
            assert!(super::source(&chars).is_none());
            assert!(!super::implemented_specifiers().contains("neo:chars"));
        }

        #[cfg(feature = "neo-slices")]
        {
            assert_eq!(super::resolve("neo:slices"), Some(super::SLICES_SPECIFIER));
            assert!(super::source(&slices).is_some());
            assert!(super::implemented_specifiers().contains("neo:slices"));
        }
        #[cfg(not(feature = "neo-slices"))]
        {
            assert_eq!(super::resolve("neo:slices"), None);
            assert!(super::source(&slices).is_none());
            assert!(!super::implemented_specifiers().contains("neo:slices"));
        }
    }
}
