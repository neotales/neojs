//! JavaScript evaluated in every runtime before the entry module, and the
//! synthetic ECMAScript modules that import specifiers reach.
//!
//! The scripts here install globals that vaeljs supplies: currently
//! [`vael_shim`], which defines the `Vael` object, its cached built-in modules,
//! and the introspection surface `Vael.engine` and
//! `Vael.getBuiltInModulesList`. `console`, `os`, `slices`, and `test` are plain
//! ECMAScript (the `os` module contains two build-target constants substituted
//! by [`vael_shim`]); `chars` and `strings` are native Rust bindings installed
//! before the shim runs; `assert` calls one private host op registered
//! by [`crate::assert`] to observe a genuine native Promise. `cron` calls six
//! private host ops registered by [`crate::cron`]: it parses a UTC five-field
//! expression, computes the next occurrence, sleeps on the runtime's Tokio
//! timer, and cancels a job. An undisposed job keeps the event loop alive; one
//! callback runs at a time, and overlap, missed ticks, and callback failures are
//! reported through an optional observer. It adds no filesystem, environment,
//! network, or process capability and has no local-time, DST, time-zone, or
//! fake-timer surface.
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
//! `strings` is a native Rust binding shared with VaelJS Lite through
//! `vaeljs-builtins`: case-insensitive folded matching (`equalFold`,
//! `startsWithFold`, `endsWithFold`, `indexOfFold`, `lastIndexOfFold`) and
//! trimming of a caller-supplied code point set. It operates on UTF-16 code
//! units and preserves lone surrogates. The search scan examines the text plus
//! the needle linearly and uses O(needle) extra memory plus one fixed window;
//! `lastIndexOfFold` also stops once no match can begin at or before its limit,
//! so a small limit does not copy the whole text. The binding bounds copied
//! units and retained memory, not V8's internal rope traversal time. It calls no host op
//! and adds no capability. Folding is Unicode 16.0 simple case folding from a
//! fixed table in the shared crate, so it is locale-independent and both
//! engines agree instead of following their own Unicode release.
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
//! console or an isolation boundary. `fs` calls fourteen private host ops
//! registered by [`crate::fs`] and implemented in `vaeljs-fs-core` for
//! synchronous filesystem operations. When `vael-http` is enabled, `http`
//! (`vael:http`) exposes a Go-style `ServeMux` router, traversal/symlink-safe
//! static file serving, RFC 6265 cookies, composable middleware, and a live
//! HTTP/1.1 server backed by `vaeljs-http-core`, `hyper`, and `tokio`. `path` is lexical POSIX and Windows path
//! manipulation adapted from NeoLiteJS and `@neotales/path`; its only host read
//! is the process working directory, through [`crate::path`], and only when a
//! resolve/relative operation lacks an explicit absolute base. It adds no
//! filesystem, environment, network, or process capability and is not a
//! containment check.
//!
//! [`AGE_MODULE`], [`ASSERT_MODULE`], [`CONSOLE_MODULE`],
//! [`CRON_MODULE`], [`CRYPTO_MODULE`], [`FS_MODULE`], [`OS_MODULE`],
//! [`PATH_MODULE`], [`SLICES_MODULE`], [`SSH_MODULE`], [`STDIO_MODULE`],
//! [`TEST_MODULE`], [`UUID_MODULE`], [`UUID_V4_MODULE`], and [`UUID_V7_MODULE`]
//! are not evaluated as scripts. They are the sources returned by the module
//! loader for the canonical built-in specifiers [`AGE_SPECIFIER`],
//! [`ASSERT_SPECIFIER`], [`CHARS_SPECIFIER`], [`CONSOLE_SPECIFIER`],
//! [`CRON_SPECIFIER`], [`CRYPTO_SPECIFIER`], [`FS_SPECIFIER`], [`OS_SPECIFIER`],
//! [`PATH_SPECIFIER`], [`SLICES_SPECIFIER`], [`SSH_SPECIFIER`],
//! [`STDIO_SPECIFIER`], [`STRINGS_SPECIFIER`], [`TEST_SPECIFIER`],
//! [`UUID_SPECIFIER`], [`UUID_V4_SPECIFIER`], and [`UUID_V7_SPECIFIER`].
//! Only those exact `vael:` specifiers are accepted: a bare name is not a
//! built-in module specifier and the loader rejects it. `vael:chars` and
//! `vael:strings` are native V8 modules registered by `crate::chars::install`
//! and `crate::strings::install`; the loader source table returns only an empty
//! deduplication sentinel for them. `SLICES_MODULE`
//! exists only with `vael-slices`; with the feature off its name, source,
//! registry entry, and loader entry are absent. All except the native `chars`
//! and `strings` modules read the public module objects installed by
//! [`vael_shim`], so the
//! registry and the module bindings share identity. `assert` is the frozen callable
//! `assert` function itself, with its named assertions as properties; it also
//! has a default export in its module namespace. `age`, `chars`, `cron`, `fs`,
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

/// Installs the global `Vael` object and the built-in module registry.
///
/// Evaluation happens once per realm. `Vael.engine` is `"vael"` for this V8
/// runtime. `Vael.getBuiltInModulesList` returns one frozen array of the
/// canonical `vael:` specifiers, built from the registry in insertion order;
/// repeated calls return that same array. The `age`,
/// `assert`, `chars`, `cron`, `crypto`, `fs`, `http` (when `vael-http` is
/// enabled), `os`, `path`, `slices`, `ssh`, `stdio`, `strings`, `test`,
/// `uuid`, `uuid/v4`, and `uuid/v7` module objects are frozen and cached, so
/// repeated `Vael.getBuiltInModule` calls with the canonical specifier return
/// the same object for each. The `console` module object is
/// the console object the runtime installed, captured after the console shim
/// runs. At installation time the
/// registry, the module bindings, and the global console share the same
/// function objects; a method replaced before the console module is evaluated
/// is what its named exports capture. Names that are not implemented return
/// `undefined`.
const VAEL_SHIM_SOURCE: &str = include_str!("builtins/vael.js");

/// The placeholder replaced with the build target's Deno-compatible OS name.
const OS_PLATFORM_TOKEN: &str = "__VAEL_OS_PLATFORM__";

/// The placeholder replaced with the build target's Deno-compatible arch name.
const OS_ARCH_TOKEN: &str = "__VAEL_OS_ARCH__";

/// The marker replaced with the native `vael:chars` registry entry.
///
/// The V8 runtime installs the native `vael:chars` object before this shim is
/// evaluated. The marker is replaced with a spread of that entry array, so the
/// `vael:chars` implementation never appears in JavaScript.
const NATIVE_ENTRIES_TOKEN: &str = "// __VAEL_NATIVE_ENTRIES__";

/// The marker replaced with the native `vael:strings` registry entry.
///
/// This marker is at the `strings` position in the registry, so enabling the
/// native binding does not reorder the introspection list. The implementation
/// never appears in JavaScript.
const NATIVE_STRINGS_TOKEN: &str = "// __VAEL_NATIVE_STRINGS_ENTRY__";

/// The marker replaced with the shared `vael:assert` factory source.
const ASSERT_IMPL_TOKEN: &str = "// __VAEL_ASSERT_IMPL__";

/// The marker replaced with the shared `vael:path` factory source.
const PATH_IMPL_TOKEN: &str = "// __VAEL_PATH_IMPL__";

/// The marker replaced with the shared `vael:test` factory source.
const TEST_IMPL_TOKEN: &str = "// __VAEL_TEST_IMPL__";

/// The marker replaced with the separate `slices` implementation source.
///
/// The implementation is not part of [`VAEL_SHIM_SOURCE`]. It is compiled in
/// only when `vael-slices` is enabled, so a disabled build does not carry its
/// bytes.
const SLICES_IMPL_TOKEN: &str = "// __VAEL_SLICES_IMPL__";

/// The marker replaced with the `slices` registry entry.
const SLICES_REGISTRY_TOKEN: &str = "// __VAEL_SLICES_REGISTRY_ENTRY__";

/// The marker replaced with the `db` implementation source.
///
/// The implementation is not part of [`VAEL_SHIM_SOURCE`]. It is compiled in
/// only when `vael-db` is enabled, so a disabled build does not carry its bytes.
const DB_IMPL_TOKEN: &str = "// __VAEL_DB_IMPL__";

/// The marker replaced with the `db` registry entry.
const DB_REGISTRY_TOKEN: &str = "// __VAEL_DB_REGISTRY_ENTRY__";

/// Source for the `vael:db` registry object, compiled only when `vael-db` is enabled.
#[cfg(feature = "vael-db")]
const DB_IMPL_SOURCE: &str = include_str!("builtins/db.impl.js");

/// The marker replaced with the `http` implementation source.
const HTTP_IMPL_TOKEN: &str = "// __VAEL_HTTP_IMPL__";

/// The marker replaced with the `http` registry entry.
const HTTP_REGISTRY_TOKEN: &str = "// __VAEL_HTTP_REGISTRY_ENTRY__";

/// The `vael:http` factory source, compiled in only when `vael-http` is enabled.
#[cfg(feature = "vael-http")]
const HTTP_IMPL_SOURCE: &str = include_str!("builtins/http.js");

/// The marker replaced with the `template` implementation source.
const TEMPLATE_IMPL_TOKEN: &str = "// __VAEL_TEMPLATE_IMPL__";

/// The marker replaced with the `template` registry entry.
const TEMPLATE_REGISTRY_TOKEN: &str = "// __VAEL_TEMPLATE_REGISTRY_ENTRY__";

/// Returns the shim with the build target's `os` constants substituted and the
/// selected built-in source and registration spliced in.
///
/// [`VAEL_SHIM_SOURCE`] holds two placeholders, one per `vael:os` value. They are
/// replaced with ECMAScript string literals for this build's
/// `vaeljs_builtins::os::platform()` (Deno-compatible OS target) and `arch()`
/// (normalized short architecture name), so the module reports the target the
/// runtime was compiled for and reads no host state at runtime.
///
/// `vael-chars` spreads the native entry installed by `crate::chars::install`
/// into the registry and the introspection list, and `vael-strings` splices the
/// entry installed by `crate::strings::install` at the `strings` position. The
/// objects and every function are native V8 values; no ECMAScript
/// implementation or facade is spliced in.
/// `vael-slices` splices the shared `vaeljs_builtins::slices::FACTORY_SOURCE`
/// and registry entry; it implies `vael-chars` because `CharSlice` captures
/// `chars.toUpper` and `chars.toLower`, and reads the native object through the
/// same entries global.
/// A disabled module's registry entry, loader entry, and name are all absent.
pub(crate) fn vael_shim() -> String {
    let shim = VAEL_SHIM_SOURCE
        .replace(
            OS_PLATFORM_TOKEN,
            &js_string(vaeljs_builtins::os::platform()),
        )
        .replace(OS_ARCH_TOKEN, &js_string(vaeljs_builtins::os::arch()))
        .replace(ASSERT_IMPL_TOKEN, vaeljs_builtins::assert::FACTORY_SOURCE)
        .replace(PATH_IMPL_TOKEN, vaeljs_builtins::path::FACTORY_SOURCE)
        .replace(TEST_IMPL_TOKEN, vaeljs_builtins::test::FACTORY_SOURCE);

    #[cfg(feature = "vael-chars")]
    let shim = shim.replace(
        NATIVE_ENTRIES_TOKEN,
        &format!("...globalThis.{},", crate::chars::NATIVE_ENTRIES_GLOBAL),
    );
    #[cfg(not(feature = "vael-chars"))]
    let shim = shim.replace(NATIVE_ENTRIES_TOKEN, "");

    #[cfg(feature = "vael-strings")]
    let shim = shim.replace(
        NATIVE_STRINGS_TOKEN,
        &format!("...globalThis.{},", crate::strings::NATIVE_ENTRIES_GLOBAL),
    );
    #[cfg(not(feature = "vael-strings"))]
    let shim = shim.replace(NATIVE_STRINGS_TOKEN, "");

    #[cfg(feature = "vael-slices")]
    let shim = shim
        .replace(
            SLICES_IMPL_TOKEN,
            &format!(
                "const slices = (\n{}\n)(globalThis.{}.find((entry) => entry[0] === \"vael:chars\")[1]);",
                vaeljs_builtins::slices::FACTORY_SOURCE,
                crate::chars::NATIVE_ENTRIES_GLOBAL,
            ),
        )
        .replace(SLICES_REGISTRY_TOKEN, "[\"vael:slices\", slices],");
    #[cfg(not(feature = "vael-slices"))]
    let shim = shim
        .replace(SLICES_IMPL_TOKEN, "")
        .replace(SLICES_REGISTRY_TOKEN, "");

    #[cfg(feature = "vael-db")]
    let shim = shim
        .replace(DB_IMPL_TOKEN, &format!("const db = {DB_IMPL_SOURCE};"))
        .replace(DB_REGISTRY_TOKEN, "[\"vael:db\", db],");
    #[cfg(not(feature = "vael-db"))]
    let shim = shim
        .replace(DB_IMPL_TOKEN, "")
        .replace(DB_REGISTRY_TOKEN, "");

    #[cfg(feature = "vael-http")]
    let shim = shim
        .replace(
            HTTP_IMPL_TOKEN,
            &format!("const http = (\n{HTTP_IMPL_SOURCE}\n)({{ ops: Deno.core.ops }});"),
        )
        .replace(HTTP_REGISTRY_TOKEN, "[\"vael:http\", http],");
    #[cfg(not(feature = "vael-http"))]
    let shim = shim
        .replace(HTTP_IMPL_TOKEN, "")
        .replace(HTTP_REGISTRY_TOKEN, "");

    #[cfg(feature = "vael-template")]
    let shim = shim
        .replace(TEMPLATE_IMPL_TOKEN, include_str!("builtins/template.js"))
        .replace(TEMPLATE_REGISTRY_TOKEN, "[\"vael:template\", template],");
    #[cfg(not(feature = "vael-template"))]
    let shim = shim
        .replace(TEMPLATE_IMPL_TOKEN, "")
        .replace(TEMPLATE_REGISTRY_TOKEN, "");

    debug_assert!(!shim.contains(ASSERT_IMPL_TOKEN));
    debug_assert!(!shim.contains(PATH_IMPL_TOKEN));
    debug_assert!(!shim.contains(TEST_IMPL_TOKEN));
    debug_assert!(!shim.contains(NATIVE_ENTRIES_TOKEN));
    debug_assert!(!shim.contains(NATIVE_STRINGS_TOKEN));
    debug_assert!(!shim.contains(SLICES_IMPL_TOKEN));
    debug_assert!(!shim.contains(SLICES_REGISTRY_TOKEN));
    debug_assert!(!shim.contains(DB_IMPL_TOKEN));
    debug_assert!(!shim.contains(DB_REGISTRY_TOKEN));
    debug_assert!(!shim.contains(HTTP_IMPL_TOKEN));
    debug_assert!(!shim.contains(HTTP_REGISTRY_TOKEN));
    debug_assert!(!shim.contains(TEMPLATE_IMPL_TOKEN));
    debug_assert!(!shim.contains(TEMPLATE_REGISTRY_TOKEN));
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
/// Only `vael:age` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const AGE_SPECIFIER: &str = "vael:age";

/// The ECMAScript module source served for [`AGE_SPECIFIER`].
///
/// It re-exports the function objects from the public `age` module object, so
/// the module and `Vael.getBuiltInModule("vael:age")` share identity. The
/// functions call host ops registered by [`crate::age`].
const AGE_MODULE: &str = include_str!("builtins/age.esm.js");

/// The canonical module specifier for the `assert` built-in.
///
/// Only `vael:assert` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const ASSERT_SPECIFIER: &str = "vael:assert";

/// The ECMAScript module source served for [`ASSERT_SPECIFIER`].
///
/// It re-exports the callable `assert` function and its named assertions from
/// the public `assert` module object, and also exposes that function as the
/// default export. The factory implementation and this ESM wrapper live in
/// `vaeljs_builtins::assert`; [`vael_shim`] supplies the V8 Promise hooks, and the
/// only host op called observes a genuine native Promise via [`crate::assert`].
const ASSERT_MODULE: &str = vaeljs_builtins::assert::ESM_SOURCE;

/// The canonical module specifier for the `chars` built-in.
///
/// Only `vael:chars` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one native module instance
/// for every importer. It is not a `file:` URL and is never read from disk. The
/// module is registered natively by `crate::chars::install`; the loader's
/// source table returns only an empty deduplication sentinel. With `vael-chars`
/// off, the specifier is absent from the name table and the loader rejects it.
#[cfg(feature = "vael-chars")]
pub(crate) const CHARS_SPECIFIER: &str = "vael:chars";

/// The canonical module specifier for the `console` built-in.
///
/// Only `vael:console` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const CONSOLE_SPECIFIER: &str = vaeljs_builtins::console::SPECIFIER;

/// The ECMAScript module source served for [`CONSOLE_SPECIFIER`].
///
/// It exports the console object the runtime installed as the default export
/// and its five vaeljs-defined method values as named exports. The method values
/// are read when the module is evaluated, so a replacement made before then is
/// captured. The ES module namespace is a separate object from the registry.
const CONSOLE_MODULE: &str = vaeljs_builtins::console::ESM_SOURCE;

/// The canonical module specifier for the `cron` built-in.
///
/// Only `vael:cron` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const CRON_SPECIFIER: &str = "vael:cron";

/// The ECMAScript module source served for [`CRON_SPECIFIER`].
///
/// It re-exports `cron`, `next`, and `CronError` from the public `cron` module
/// object, so the module and `Vael.getBuiltInModule("vael:cron")` share
/// identity. The functions call host ops registered by [`crate::cron`];
/// scheduling uses the runtime's own Tokio timer and a pending op keeps the
/// event loop alive until the job is disposed.
const CRON_MODULE: &str = include_str!("builtins/cron.esm.js");

/// The canonical module specifier for the `crypto` built-in.
///
/// Only `vael:crypto` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const CRYPTO_SPECIFIER: &str = "vael:crypto";

/// The ECMAScript module source served for [`CRYPTO_SPECIFIER`].
///
/// It re-exports the function objects from the public `crypto` module object,
/// so the module and `Vael.getBuiltInModule("vael:crypto")` share identity. The
/// functions call host ops registered by [`crate::crypto`],
/// [`crate::crypto_cipher`], [`crate::crypto_hash`], and
/// [`crate::crypto_shamir`].
const CRYPTO_MODULE: &str = include_str!("builtins/crypto.esm.js");

/// The canonical module specifier for the `db` built-in.
///
/// Only `vael:db` resolves to this URL. With `vael-db` off, the specifier is
/// absent from the name and source tables and the loader rejects it.
#[cfg(feature = "vael-db")]
pub(crate) const DB_SPECIFIER: &str = "vael:db";

/// The ECMAScript module source served for [`DB_SPECIFIER`].
#[cfg(feature = "vael-db")]
const DB_MODULE: &str = include_str!("builtins/db.esm.js");

/// The canonical module specifier for the `fs` built-in.
///
/// Only `vael:fs` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const FS_SPECIFIER: &str = vaeljs_fs_core::SPECIFIER;

/// The ECMAScript module source served for [`FS_SPECIFIER`].
///
/// It re-exports the function objects from the public `fs` module object, so
/// the module and `Vael.getBuiltInModule("vael:fs")` share identity. The functions
/// call host ops registered by [`crate::fs`].
const FS_MODULE: &str = include_str!("builtins/fs.esm.js");

/// The canonical module specifier for the `http` built-in.
///
/// Only `vael:http` resolves to this URL. With `vael-http` off, the specifier is
/// absent from the name and source tables and the loader rejects it.
#[cfg(feature = "vael-http")]
pub(crate) const HTTP_SPECIFIER: &str = "vael:http";

/// The ECMAScript module source served for [`HTTP_SPECIFIER`].
#[cfg(feature = "vael-http")]
const HTTP_MODULE: &str = include_str!("builtins/http.esm.js");

/// The canonical module specifier for the `os` built-in.
///
/// Only `vael:os` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const OS_SPECIFIER: &str = vaeljs_builtins::os::SPECIFIER;

/// The ECMAScript module source served for [`OS_SPECIFIER`].
///
/// It re-exports the two build-target constants from the public `os` module
/// object, so the module and `Vael.getBuiltInModule("vael:os")` share identity.
/// The values are substituted into [`vael_shim`]; the module calls no host op and
/// reads no environment variable or operating-system file.
const OS_MODULE: &str = include_str!("builtins/os.esm.js");

/// The canonical module specifier for the `ssh` built-in.
///
/// Only `vael:ssh` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const SSH_SPECIFIER: &str = "vael:ssh";

/// The ECMAScript module source served for [`SSH_SPECIFIER`].
///
/// It re-exports the synchronous `connect` function and the documented limits
/// from the public `ssh` module object, so the module and
/// `Vael.getBuiltInModule("vael:ssh")` share identity. `connect` calls host ops
/// registered by [`crate::ssh`]; it requires an explicit host-key pin or
/// `known_hosts` document and never accepts an unknown host key.
const SSH_MODULE: &str = include_str!("builtins/ssh.esm.js");

/// The canonical module specifier for the `stdio` built-in.
///
/// Only `vael:stdio` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const STDIO_SPECIFIER: &str = "vael:stdio";

/// The ECMAScript module source served for [`STDIO_SPECIFIER`].
///
/// It re-exports the classes, constant, and shared stream objects from the
/// public `stdio` module object, so the module and
/// `Vael.getBuiltInModule("vael:stdio")` share identity. The stream methods call
/// host ops registered by [`crate::stdio`].
const STDIO_MODULE: &str = include_str!("builtins/stdio.esm.js");

/// The canonical module specifier for the `path` built-in.
///
/// Only `vael:path` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const PATH_SPECIFIER: &str = vaeljs_builtins::path::SPECIFIER;

/// The ECMAScript module source served for [`PATH_SPECIFIER`].
///
/// It re-exports the frozen POSIX and Windows namespaces, their functions, and
/// the host-selected aliases from the public `path` module object, so the module
/// and `Vael.getBuiltInModule("vael:path")` share identity. The functions are
/// lexical; their host ops report the build dialect and the process working
/// directory, the latter only when an operation lacks an explicit absolute base.
const PATH_MODULE: &str = vaeljs_builtins::path::ESM_SOURCE;

/// The canonical module specifier for the `slices` built-in.
///
/// Only `vael:slices` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It requires `vael-chars` because `CharSlice` captures
/// `chars.toUpper` and `chars.toLower`. It is not a `file:` URL and is never
/// read from disk. With `vael-slices` off, the specifier is absent from the name
/// and source tables and the loader rejects it.
#[cfg(feature = "vael-slices")]
pub(crate) const SLICES_SPECIFIER: &str = vaeljs_builtins::slices::SPECIFIER;

/// The ECMAScript module source served for [`SLICES_SPECIFIER`].
///
/// It re-exports the view classes and factories from the public `slices` module
/// object, so each named export is the same function or class object as the
/// matching registry property. The ES module namespace is a separate object from
/// the registry object. The implementations are plain ECMAScript in
/// [`vael_shim`]; no host op is called. Views alias the caller's Array or typed
/// array; they are not copies, a sandbox, or a cross-realm/cross-thread
/// zero-copy mechanism.
#[cfg(feature = "vael-slices")]
const SLICES_MODULE: &str = vaeljs_builtins::slices::ESM_SOURCE;

/// The canonical module specifier for the `strings` built-in.
///
/// Only `vael:strings` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one native module instance
/// for every importer. It is not a `file:` URL and is never read from disk. The
/// module is registered natively by `crate::strings::install`; the loader's
/// source table returns only an empty deduplication sentinel. With
/// `vael-strings` off, the specifier is absent from the name table and the
/// loader rejects it.
#[cfg(feature = "vael-strings")]
pub(crate) const STRINGS_SPECIFIER: &str = "vael:strings";

/// The canonical module specifier for the `template` built-in.
///
/// Only `vael:template` resolves to this URL. It is the canonical and only
/// module specifier for this built-in, so the engine caches one module instance
/// for every importer. It is not a `file:` URL and is never read from disk.
/// With `vael-template` off, the specifier is absent from the name and source
/// tables and the loader rejects it.
#[cfg(feature = "vael-template")]
pub(crate) const TEMPLATE_SPECIFIER: &str = "vael:template";

/// The ECMAScript module source served for [`TEMPLATE_SPECIFIER`].
#[cfg(feature = "vael-template")]
const TEMPLATE_MODULE: &str = include_str!("builtins/template.esm.js");

/// The canonical module specifier for the `test` built-in.
///
/// Only `vael:test` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const TEST_SPECIFIER: &str = "vael:test";

/// The ECMAScript module source served for [`TEST_SPECIFIER`].
///
/// It re-exports the callable `test` function and the runner helpers from the
/// public `test` module object, so the module and
/// `Vael.getBuiltInModule("vael:test")` share identity. The factory
/// implementation and this ESM wrapper live in `vaeljs_builtins::test`;
/// [`vael_shim`] supplies V8's print, timer, rejection-handler, and event-loop
/// hooks.
const TEST_MODULE: &str = vaeljs_builtins::test::ESM_SOURCE;

/// The canonical module specifier for the `uuid` built-in.
///
/// Only `vael:uuid` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const UUID_SPECIFIER: &str = vaeljs_builtins::uuid::SPECIFIER;

/// The ECMAScript module source served for [`UUID_SPECIFIER`].
///
/// It re-exports the version 4 and version 7 generators from the public `uuid`
/// module object, so the module and `Vael.getBuiltInModule("vael:uuid")` share
/// identity. The functions call host ops registered by [`crate::uuid`]; they
/// draw from the operating system CSPRNG and return canonical lowercase text.
const UUID_MODULE: &str = include_str!("builtins/uuid.esm.js");

/// The canonical module specifier for the version 4 UUID built-in.
///
/// Only `vael:uuid/v4` resolves to this URL. It is a thin module over the
/// same function as `uuid.v4`, with that function as both the named
/// `v4` export and the default export. It is not read from disk.
pub(crate) const UUID_V4_SPECIFIER: &str = vaeljs_builtins::uuid::V4_SPECIFIER;

/// The ECMAScript module source served for [`UUID_V4_SPECIFIER`].
///
/// It re-exports `uuid.v4` as the named `v4` export and as the default export,
/// so both exports share one function object.
const UUID_V4_MODULE: &str = include_str!("builtins/uuid-v4.esm.js");

/// The canonical module specifier for the version 7 UUID built-in.
///
/// Only `vael:uuid/v7` resolves to this URL. It is a thin module over the
/// same function as `uuid.v7`, with that function as both the named
/// `v7` export and the default export. It is not read from disk.
pub(crate) const UUID_V7_SPECIFIER: &str = vaeljs_builtins::uuid::V7_SPECIFIER;

/// The ECMAScript module source served for [`UUID_V7_SPECIFIER`].
///
/// It re-exports `uuid.v7` as the named `v7` export and as the default export,
/// so both exports share one function object.
const UUID_V7_MODULE: &str = include_str!("builtins/uuid-v7.esm.js");

/// Registry name and canonical specifier for each implemented built-in module.
///
/// The loader serves one module instance per canonical specifier, and the error
/// messages list the same table, so adding a module in one place is enough. A
/// registry name is the bare key used internally; it is not an import
/// specifier and `Vael.getBuiltInModule` requires the canonical form.
const BUILTINS: &[(&str, &str)] = &[
    ("age", AGE_SPECIFIER),
    ("assert", ASSERT_SPECIFIER),
    #[cfg(feature = "vael-chars")]
    ("chars", CHARS_SPECIFIER),
    ("console", CONSOLE_SPECIFIER),
    ("cron", CRON_SPECIFIER),
    ("crypto", CRYPTO_SPECIFIER),
    #[cfg(feature = "vael-db")]
    ("db", DB_SPECIFIER),
    ("fs", FS_SPECIFIER),
    #[cfg(feature = "vael-http")]
    ("http", HTTP_SPECIFIER),
    ("os", OS_SPECIFIER),
    ("path", PATH_SPECIFIER),
    #[cfg(feature = "vael-slices")]
    ("slices", SLICES_SPECIFIER),
    ("ssh", SSH_SPECIFIER),
    ("stdio", STDIO_SPECIFIER),
    #[cfg(feature = "vael-strings")]
    ("strings", STRINGS_SPECIFIER),
    #[cfg(feature = "vael-template")]
    ("template", TEMPLATE_SPECIFIER),
    ("test", TEST_SPECIFIER),
    ("uuid", UUID_SPECIFIER),
    ("uuid/v4", UUID_V4_SPECIFIER),
    ("uuid/v7", UUID_V7_SPECIFIER),
];

/// Returns the canonical specifier for a built-in registry name, or `None`.
///
/// This accepts the bare registry name. It is used to match the path of a
/// `vael:` URL, not to accept a bare import specifier. `Vael.getBuiltInModule`
/// takes the canonical specifier instead.
pub(crate) fn resolve_name(name: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|(registry, _)| *registry == name)
        .map(|(_, canonical)| *canonical)
}

/// Returns the canonical specifier for an import specifier, or `None`.
///
/// Only the exact canonical `vael:` specifier is accepted. A bare name is not a
/// built-in module specifier, so a relative, absolute, package, or `node:`
/// specifier is never intercepted. Scheme case folding is handled by the loader
/// through the URL parser.
pub(crate) fn resolve(specifier: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .map(|(_, canonical)| *canonical)
        .find(|canonical| *canonical == specifier)
}

/// Returns whether `name` is a bare registry name for an implemented module.
///
/// The loader uses this only to explain a rejected bare specifier. It never
/// turns the name into a built-in module.
pub(crate) fn is_registry_name(name: &str) -> bool {
    BUILTINS.iter().any(|(registry, _)| *registry == name)
}

/// Returns the module source for a resolved built-in specifier, or `None`.
pub(crate) fn source(specifier: &Url) -> Option<&'static str> {
    match specifier.as_str() {
        AGE_SPECIFIER => Some(AGE_MODULE),
        ASSERT_SPECIFIER => Some(ASSERT_MODULE),
        // `vael:chars` is registered natively before the shim runs. The loader
        // still has to answer a load request for it, so it returns an empty
        // sentinel; the module map deduplicates onto the already-registered
        // native module and never compiles this source.
        #[cfg(feature = "vael-chars")]
        CHARS_SPECIFIER => Some(""),
        CONSOLE_SPECIFIER => Some(CONSOLE_MODULE),
        CRON_SPECIFIER => Some(CRON_MODULE),
        CRYPTO_SPECIFIER => Some(CRYPTO_MODULE),
        #[cfg(feature = "vael-db")]
        DB_SPECIFIER => Some(DB_MODULE),
        FS_SPECIFIER => Some(FS_MODULE),
        #[cfg(feature = "vael-http")]
        HTTP_SPECIFIER => Some(HTTP_MODULE),
        OS_SPECIFIER => Some(OS_MODULE),
        PATH_SPECIFIER => Some(PATH_MODULE),
        #[cfg(feature = "vael-slices")]
        SLICES_SPECIFIER => Some(SLICES_MODULE),
        SSH_SPECIFIER => Some(SSH_MODULE),
        STDIO_SPECIFIER => Some(STDIO_MODULE),
        // `vael:strings` is registered natively before the shim runs. The loader
        // still has to answer a load request for it, so it returns an empty
        // sentinel; the module map deduplicates onto the already-registered
        // native module and never compiles this source.
        #[cfg(feature = "vael-strings")]
        STRINGS_SPECIFIER => Some(""),
        #[cfg(feature = "vael-template")]
        TEMPLATE_SPECIFIER => Some(TEMPLATE_MODULE),
        TEST_SPECIFIER => Some(TEST_MODULE),
        UUID_SPECIFIER => Some(UUID_MODULE),
        UUID_V4_SPECIFIER => Some(UUID_V4_MODULE),
        UUID_V7_SPECIFIER => Some(UUID_V7_MODULE),
        _ => None,
    }
}

/// Human-readable list of implemented built-in specifiers, for error messages.
pub(crate) fn implemented_specifiers() -> String {
    BUILTINS
        .iter()
        .map(|(_, canonical)| *canonical)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::OS_ARCH_TOKEN;
    use super::OS_PLATFORM_TOKEN;
    use super::implemented_specifiers;
    use super::is_registry_name;
    use super::resolve;
    use super::resolve_name;
    use super::vael_shim;
    use super::{
        ASSERT_IMPL_TOKEN, DB_IMPL_TOKEN, DB_REGISTRY_TOKEN, NATIVE_ENTRIES_TOKEN,
        NATIVE_STRINGS_TOKEN, PATH_IMPL_TOKEN, SLICES_IMPL_TOKEN, SLICES_REGISTRY_TOKEN,
        TEMPLATE_IMPL_TOKEN, TEMPLATE_REGISTRY_TOKEN, TEST_IMPL_TOKEN,
    };
    use deno_core::url::Url;

    /// The shim is not valid JavaScript until the two `os` placeholders are
    /// replaced, so a missing substitution must fail here rather than at the
    /// first `Runtime::new`.
    /// Only the canonical `vael:` specifier is an import specifier. A bare
    /// registry name is not, and case folding belongs to the URL parser in the
    /// loader rather than to this exact lookup.
    #[test]
    fn only_canonical_specifiers_resolve() {
        assert_eq!(resolve("vael:age"), Some("vael:age"));
        assert_eq!(resolve("vael:uuid/v4"), Some("vael:uuid/v4"));
        assert_eq!(resolve("age"), None);
        assert_eq!(resolve("uuid/v4"), None);
        assert_eq!(resolve("VAEL:age"), None);
        assert_eq!(resolve("vael:nope"), None);
        assert_eq!(resolve("./age.js"), None);
    }

    /// The registry keeps bare names as internal keys only. Error reporting can
    /// suggest the canonical spelling without ever treating the bare name as a
    /// module.
    #[test]
    fn bare_registry_names_are_internal_keys_only() {
        assert_eq!(resolve_name("age"), Some("vael:age"));
        assert_eq!(resolve_name("uuid/v7"), Some("vael:uuid/v7"));
        assert_eq!(resolve_name("vael:age"), None);
        assert!(is_registry_name("age"));
        assert!(!is_registry_name("vael:age"));
        assert!(!is_registry_name("lodash"));
    }

    /// Error messages list canonical specifiers only, never the internal bare
    /// registry names that a detached package resolver could later own.
    #[test]
    fn implemented_specifiers_lists_canonical_names_only() {
        let list = implemented_specifiers();
        assert!(list.starts_with("vael:age, vael:assert"));
        assert!(list.ends_with("vael:uuid/v4, vael:uuid/v7"));
        assert!(!list.contains(", age"));
    }

    #[test]
    fn shim_substitutes_the_build_target_constants() {
        let shim = vael_shim();
        assert!(!shim.contains(OS_PLATFORM_TOKEN));
        assert!(!shim.contains(OS_ARCH_TOKEN));
        assert!(!shim.contains(ASSERT_IMPL_TOKEN));
        assert!(!shim.contains(PATH_IMPL_TOKEN));
        assert!(!shim.contains(TEST_IMPL_TOKEN));
        assert!(!shim.contains(NATIVE_ENTRIES_TOKEN));
        assert!(!shim.contains(NATIVE_STRINGS_TOKEN));
        assert!(!shim.contains(SLICES_IMPL_TOKEN));
        assert!(!shim.contains(SLICES_REGISTRY_TOKEN));
        assert!(!shim.contains(DB_IMPL_TOKEN));
        assert!(!shim.contains(DB_REGISTRY_TOKEN));
        assert!(!shim.contains(TEMPLATE_IMPL_TOKEN));
        assert!(!shim.contains(TEMPLATE_REGISTRY_TOKEN));
        assert!(
            shim.contains(&format!(
                "platform: \"{}\"",
                vaeljs_builtins::os::platform()
            )),
            "platform literal is missing from the shim"
        );
        assert!(
            shim.contains(&format!("arch: \"{}\"", vaeljs_builtins::os::arch())),
            "arch literal is missing from the shim"
        );

        #[cfg(feature = "vael-chars")]
        {
            assert!(
                shim.contains("...globalThis.__vaelNativeBuiltins,"),
                "the native chars entries were not spliced into the registry"
            );
            assert!(
                !shim.contains("const chars = "),
                "a JavaScript chars implementation was spliced in"
            );
            assert!(
                !shim.contains("function isChar("),
                "a JavaScript chars implementation is present"
            );
        }
        #[cfg(not(feature = "vael-chars"))]
        {
            assert!(
                !shim.contains("...globalThis.__vaelNativeBuiltins,"),
                "the disabled chars registry entry is present"
            );
            assert!(
                !shim.contains("const chars = "),
                "the disabled chars declaration is present"
            );
            assert!(
                !shim.contains("function isChar("),
                "the disabled chars implementation is present"
            );
        }

        #[cfg(feature = "vael-strings")]
        {
            assert!(
                shim.contains("...globalThis.__vaelNativeStrings,"),
                "the native strings entry was not spliced into the registry"
            );
            assert!(
                !shim.contains("const strings = Object.freeze("),
                "a JavaScript strings implementation was spliced in"
            );
            assert!(
                !shim.contains("function equalFold("),
                "a JavaScript strings implementation is present"
            );
        }
        #[cfg(not(feature = "vael-strings"))]
        {
            assert!(
                !shim.contains("globalThis.__vaelNativeStrings"),
                "the disabled strings registry entry is present"
            );
            assert!(
                !shim.contains("const strings = Object.freeze("),
                "the disabled strings declaration is present"
            );
            assert!(
                !shim.contains("function equalFold("),
                "the disabled strings implementation is present"
            );
        }

        #[cfg(feature = "vael-slices")]
        {
            assert!(
                shim.contains("function createVaelSlices(host)"),
                "the slices implementation is missing"
            );
            assert!(
                shim.contains("[\"vael:slices\", slices],"),
                "the slices registry entry is missing"
            );
        }
        #[cfg(not(feature = "vael-slices"))]
        {
            assert!(
                !shim.contains("function createVaelSlices(host)"),
                "the disabled slices implementation is present"
            );
            assert!(
                !shim.contains("class CharSlice"),
                "the disabled slices source is present"
            );
            assert!(
                !shim.contains("[\"vael:slices\", slices],"),
                "the disabled slices registry entry is present"
            );
        }

        #[cfg(feature = "vael-db")]
        {
            assert!(
                shim.contains("op_vael_db_open"),
                "the db implementation is missing"
            );
            assert!(
                shim.contains("[\"vael:db\", db],"),
                "the db registry entry is missing"
            );
        }
        #[cfg(not(feature = "vael-db"))]
        {
            assert!(
                !shim.contains("op_vael_db_open"),
                "the disabled db implementation is present"
            );
            assert!(
                !shim.contains("[\"vael:db\", db],"),
                "the disabled db registry entry is present"
            );
        }
    }

    /// A disabled module must be absent from both the name table the loader
    /// consults and the source table it serves. The enabled module is present.
    #[test]
    fn module_tables_follow_the_chars_strings_and_slices_features() {
        let chars = Url::parse("vael:chars").expect("valid chars specifier");
        let strings = Url::parse("vael:strings").expect("valid strings specifier");
        let slices = Url::parse("vael:slices").expect("valid slices specifier");
        let db = Url::parse("vael:db").expect("valid db specifier");

        #[cfg(feature = "vael-chars")]
        {
            assert_eq!(super::resolve("vael:chars"), Some(super::CHARS_SPECIFIER));
            assert_eq!(super::source(&chars), Some(""));
            assert!(super::implemented_specifiers().contains("vael:chars"));
        }
        #[cfg(not(feature = "vael-chars"))]
        {
            assert_eq!(super::resolve("vael:chars"), None);
            assert!(super::source(&chars).is_none());
            assert!(!super::implemented_specifiers().contains("vael:chars"));
        }

        #[cfg(feature = "vael-strings")]
        {
            assert_eq!(
                super::resolve("vael:strings"),
                Some(super::STRINGS_SPECIFIER)
            );
            assert_eq!(super::source(&strings), Some(""));
            assert!(super::implemented_specifiers().contains("vael:strings"));
        }
        #[cfg(not(feature = "vael-strings"))]
        {
            assert_eq!(super::resolve("vael:strings"), None);
            assert!(super::source(&strings).is_none());
            assert!(!super::implemented_specifiers().contains("vael:strings"));
        }

        #[cfg(feature = "vael-slices")]
        {
            assert_eq!(super::resolve("vael:slices"), Some(super::SLICES_SPECIFIER));
            assert!(super::source(&slices).is_some());
            assert!(super::implemented_specifiers().contains("vael:slices"));
        }
        #[cfg(not(feature = "vael-slices"))]
        {
            assert_eq!(super::resolve("vael:slices"), None);
            assert!(super::source(&slices).is_none());
            assert!(!super::implemented_specifiers().contains("vael:slices"));
        }

        #[cfg(feature = "vael-db")]
        {
            assert_eq!(super::resolve("vael:db"), Some(super::DB_SPECIFIER));
            assert!(super::source(&db).is_some());
            assert!(super::implemented_specifiers().contains("vael:db"));
        }
        #[cfg(not(feature = "vael-db"))]
        {
            assert_eq!(super::resolve("vael:db"), None);
            assert!(super::source(&db).is_none());
            assert!(!super::implemented_specifiers().contains("vael:db"));
        }
    }
}
