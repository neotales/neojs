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
//! [`AGE_MODULE`], [`ASSERT_MODULE`], [`CHARS_MODULE`], [`CONSOLE_MODULE`],
//! [`CRON_MODULE`], [`CRYPTO_MODULE`], [`OS_MODULE`], [`PATH_MODULE`],
//! [`SLICES_MODULE`], [`SSH_MODULE`], [`STDIO_MODULE`], [`STRINGS_MODULE`],
//! [`TEST_MODULE`], [`UUID_MODULE`], [`UUID_V4_MODULE`], and [`UUID_V7_MODULE`]
//! are not evaluated as scripts. They are the sources returned by the module
//! loader for the canonical built-in specifiers [`AGE_SPECIFIER`],
//! [`ASSERT_SPECIFIER`], [`CHARS_SPECIFIER`], [`CONSOLE_SPECIFIER`],
//! [`CRON_SPECIFIER`], [`CRYPTO_SPECIFIER`], [`OS_SPECIFIER`],
//! [`PATH_SPECIFIER`], [`SLICES_SPECIFIER`], [`SSH_SPECIFIER`],
//! [`STDIO_SPECIFIER`], [`STRINGS_SPECIFIER`], [`TEST_SPECIFIER`],
//! [`UUID_SPECIFIER`], [`UUID_V4_SPECIFIER`], and [`UUID_V7_SPECIFIER`].
//! Only those exact `neo:` specifiers are accepted: a bare name is not a
//! built-in module specifier and the loader rejects it. All read the public
//! module objects installed by [`neo_shim`], so the registry and the module
//! bindings share identity. `assert` is the frozen callable
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
/// runtime. `Neo.getBuiltInModulesList` returns one frozen array of the
/// canonical `neo:` specifiers, built from the registry in insertion order;
/// repeated calls return that same array. The `age`,
/// `assert`, `chars`, `cron`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`,
/// `strings`, `test`, `uuid`,
/// `uuid/v4`, and `uuid/v7` module objects are frozen and
/// cached, so repeated `Neo.getBuiltInModule` calls with the canonical
/// specifier return the same object for each. The `console` module object is
/// the console object the runtime installed, captured after the console shim
/// runs. At installation time the
/// registry, the module bindings, and the global console share the same
/// function objects; a method replaced before the console module is evaluated
/// is what its named exports capture. Names that are not implemented return
/// `undefined`.
const NEO_SHIM_SOURCE: &str = include_str!("builtins/neo.js");

/// The placeholder replaced with the build target's `std::env::consts::OS`.
const OS_PLATFORM_TOKEN: &str = "__NEO_OS_PLATFORM__";

/// The placeholder replaced with the build target's `std::env::consts::ARCH`.
const OS_ARCH_TOKEN: &str = "__NEO_OS_ARCH__";

/// Returns the shim with the build target's `os` constants substituted.
///
/// [`NEO_SHIM_SOURCE`] holds two placeholders, one per `neo:os` value. They are
/// replaced with ECMAScript string literals for this build's
/// `std::env::consts::OS` and `std::env::consts::ARCH`, so the module reports the
/// target the runtime was compiled for and reads no host state at runtime.
pub(crate) fn neo_shim() -> String {
    NEO_SHIM_SOURCE
        .replace(OS_PLATFORM_TOKEN, &js_string(std::env::consts::OS))
        .replace(OS_ARCH_TOKEN, &js_string(std::env::consts::ARCH))
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
/// Only `neo:age` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const AGE_SPECIFIER: &str = "neo:age";

/// The ECMAScript module source served for [`AGE_SPECIFIER`].
///
/// It re-exports the function objects from the public `age` module object, so
/// the module and `Neo.getBuiltInModule("neo:age")` share identity. The
/// functions call host ops registered by [`crate::age`].
const AGE_MODULE: &str = include_str!("builtins/age.esm.js");

/// The canonical module specifier for the `assert` built-in.
///
/// Only `neo:assert` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
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
/// Only `neo:chars` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const CHARS_SPECIFIER: &str = "neo:chars";

/// The ECMAScript module source served for [`CHARS_SPECIFIER`].
///
/// It re-exports the function objects from the public `chars` module object, so
/// the module and `Neo.getBuiltInModule("neo:chars")` share identity.
const CHARS_MODULE: &str = include_str!("builtins/chars.esm.js");

/// The canonical module specifier for the `console` built-in.
///
/// Only `neo:console` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
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
/// Only `neo:cron` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const CRON_SPECIFIER: &str = "neo:cron";

/// The ECMAScript module source served for [`CRON_SPECIFIER`].
///
/// It re-exports `cron`, `next`, and `CronError` from the public `cron` module
/// object, so the module and `Neo.getBuiltInModule("neo:cron")` share
/// identity. The functions call host ops registered by [`crate::cron`];
/// scheduling uses the runtime's own Tokio timer and a pending op keeps the
/// event loop alive until the job is disposed.
const CRON_MODULE: &str = include_str!("builtins/cron.esm.js");

/// The canonical module specifier for the `crypto` built-in.
///
/// Only `neo:crypto` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const CRYPTO_SPECIFIER: &str = "neo:crypto";

/// The ECMAScript module source served for [`CRYPTO_SPECIFIER`].
///
/// It re-exports the function objects from the public `crypto` module object,
/// so the module and `Neo.getBuiltInModule("neo:crypto")` share identity. The
/// functions call host ops registered by [`crate::crypto`],
/// [`crate::crypto_cipher`], [`crate::crypto_hash`], and
/// [`crate::crypto_shamir`].
const CRYPTO_MODULE: &str = include_str!("builtins/crypto.esm.js");

/// The canonical module specifier for the `os` built-in.
///
/// Only `neo:os` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const OS_SPECIFIER: &str = "neo:os";

/// The ECMAScript module source served for [`OS_SPECIFIER`].
///
/// It re-exports the two build-target constants from the public `os` module
/// object, so the module and `Neo.getBuiltInModule("neo:os")` share identity.
/// The values are substituted into [`neo_shim`]; the module calls no host op and
/// reads no environment variable or operating-system file.
const OS_MODULE: &str = include_str!("builtins/os.esm.js");

/// The canonical module specifier for the `ssh` built-in.
///
/// Only `neo:ssh` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const SSH_SPECIFIER: &str = "neo:ssh";

/// The ECMAScript module source served for [`SSH_SPECIFIER`].
///
/// It re-exports the synchronous `connect` function and the documented limits
/// from the public `ssh` module object, so the module and
/// `Neo.getBuiltInModule("neo:ssh")` share identity. `connect` calls host ops
/// registered by [`crate::ssh`]; it requires an explicit host-key pin or
/// `known_hosts` document and never accepts an unknown host key.
const SSH_MODULE: &str = include_str!("builtins/ssh.esm.js");

/// The canonical module specifier for the `stdio` built-in.
///
/// Only `neo:stdio` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const STDIO_SPECIFIER: &str = "neo:stdio";

/// The ECMAScript module source served for [`STDIO_SPECIFIER`].
///
/// It re-exports the classes, constant, and shared stream objects from the
/// public `stdio` module object, so the module and
/// `Neo.getBuiltInModule("neo:stdio")` share identity. The stream methods call
/// host ops registered by [`crate::stdio`].
const STDIO_MODULE: &str = include_str!("builtins/stdio.esm.js");

/// The canonical module specifier for the `path` built-in.
///
/// Only `neo:path` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const PATH_SPECIFIER: &str = "neo:path";

/// The ECMAScript module source served for [`PATH_SPECIFIER`].
///
/// It re-exports the frozen POSIX and Windows namespaces, their functions, and
/// the host-selected aliases from the public `path` module object, so the module
/// and `Neo.getBuiltInModule("neo:path")` share identity. The functions are
/// lexical; their host ops report the build dialect and the process working
/// directory, the latter only when an operation lacks an explicit absolute base.
const PATH_MODULE: &str = include_str!("builtins/path.esm.js");

/// The canonical module specifier for the `slices` built-in.
///
/// Only `neo:slices` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const SLICES_SPECIFIER: &str = "neo:slices";

/// The ECMAScript module source served for [`SLICES_SPECIFIER`].
///
/// It re-exports the view classes and factories from the public `slices` module
/// object, so the module and `Neo.getBuiltInModule("neo:slices")` share
/// identity. The implementations are plain ECMAScript in [`neo_shim`]; no host
/// op is called. Views alias the caller's Array or typed array; they are not copies,
/// a sandbox, or a cross-realm/cross-thread zero-copy mechanism.
const SLICES_MODULE: &str = include_str!("builtins/slices.esm.js");

/// The canonical module specifier for the `strings` built-in.
///
/// Only `neo:strings` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const STRINGS_SPECIFIER: &str = "neo:strings";

/// The ECMAScript module source served for [`STRINGS_SPECIFIER`].
///
/// It re-exports the function objects from the public `strings` module object,
/// so the module and `Neo.getBuiltInModule("neo:strings")` share identity.
/// The implementations are plain ECMAScript in [`neo_shim`]; no host op is
/// called.
const STRINGS_MODULE: &str = include_str!("builtins/strings.esm.js");

/// The canonical module specifier for the `test` built-in.
///
/// Only `neo:test` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const TEST_SPECIFIER: &str = "neo:test";

/// The ECMAScript module source served for [`TEST_SPECIFIER`].
///
/// It re-exports the callable `test` function and the runner helpers from the
/// public `test` module object, so the module and
/// `Neo.getBuiltInModule("neo:test")` share identity. The implementation is in
/// [`neo_shim`]; it calls no host op.
const TEST_MODULE: &str = include_str!("builtins/test.esm.js");

/// The canonical module specifier for the `uuid` built-in.
///
/// Only `neo:uuid` resolves to this URL. It is the canonical and only module
/// specifier for this built-in, so the engine caches one module instance for
/// every importer. It is not a `file:` URL and is never read from disk.
pub(crate) const UUID_SPECIFIER: &str = "neo:uuid";

/// The ECMAScript module source served for [`UUID_SPECIFIER`].
///
/// It re-exports the version 4 and version 7 generators from the public `uuid`
/// module object, so the module and `Neo.getBuiltInModule("neo:uuid")` share
/// identity. The functions call host ops registered by [`crate::uuid`]; they
/// draw from the operating system CSPRNG and return canonical lowercase text.
const UUID_MODULE: &str = include_str!("builtins/uuid.esm.js");

/// The canonical module specifier for the version 4 UUID built-in.
///
/// Only `neo:uuid/v4` resolves to this URL. It is a thin module over the
/// same function as `uuid.v4`, with that function as both the named
/// `v4` export and the default export. It is not read from disk.
pub(crate) const UUID_V4_SPECIFIER: &str = "neo:uuid/v4";

/// The ECMAScript module source served for [`UUID_V4_SPECIFIER`].
///
/// It re-exports `uuid.v4` as the named `v4` export and as the default export,
/// so both exports share one function object.
const UUID_V4_MODULE: &str = include_str!("builtins/uuid-v4.esm.js");

/// The canonical module specifier for the version 7 UUID built-in.
///
/// Only `neo:uuid/v7` resolves to this URL. It is a thin module over the
/// same function as `uuid.v7`, with that function as both the named
/// `v7` export and the default export. It is not read from disk.
pub(crate) const UUID_V7_SPECIFIER: &str = "neo:uuid/v7";

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
/// specifier and `Neo.getBuiltInModule` requires the canonical form.
const BUILTINS: &[(&str, &str)] = &[
    ("age", AGE_SPECIFIER),
    ("assert", ASSERT_SPECIFIER),
    ("chars", CHARS_SPECIFIER),
    ("console", CONSOLE_SPECIFIER),
    ("cron", CRON_SPECIFIER),
    ("crypto", CRYPTO_SPECIFIER),
    ("os", OS_SPECIFIER),
    ("path", PATH_SPECIFIER),
    ("slices", SLICES_SPECIFIER),
    ("ssh", SSH_SPECIFIER),
    ("stdio", STDIO_SPECIFIER),
    ("strings", STRINGS_SPECIFIER),
    ("test", TEST_SPECIFIER),
    ("uuid", UUID_SPECIFIER),
    ("uuid/v4", UUID_V4_SPECIFIER),
    ("uuid/v7", UUID_V7_SPECIFIER),
];

/// Returns the canonical specifier for a built-in registry name, or `None`.
///
/// This accepts the bare registry name. It is used to match the path of a
/// `neo:` URL, not to accept a bare import specifier. `Neo.getBuiltInModule`
/// takes the canonical specifier instead.
pub(crate) fn resolve_name(name: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|(registry, _)| *registry == name)
        .map(|(_, canonical)| *canonical)
}

/// Returns the canonical specifier for an import specifier, or `None`.
///
/// Only the exact canonical `neo:` specifier is accepted. A bare name is not a
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
        CHARS_SPECIFIER => Some(CHARS_MODULE),
        CONSOLE_SPECIFIER => Some(CONSOLE_MODULE),
        CRON_SPECIFIER => Some(CRON_MODULE),
        CRYPTO_SPECIFIER => Some(CRYPTO_MODULE),
        OS_SPECIFIER => Some(OS_MODULE),
        PATH_SPECIFIER => Some(PATH_MODULE),
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
    use super::neo_shim;
    use super::resolve;
    use super::resolve_name;

    /// The shim is not valid JavaScript until the two `os` placeholders are
    /// replaced, so a missing substitution must fail here rather than at the
    /// first `Runtime::new`.
    /// Only the canonical `neo:` specifier is an import specifier. A bare
    /// registry name is not, and case folding belongs to the URL parser in the
    /// loader rather than to this exact lookup.
    #[test]
    fn only_canonical_specifiers_resolve() {
        assert_eq!(resolve("neo:age"), Some("neo:age"));
        assert_eq!(resolve("neo:uuid/v4"), Some("neo:uuid/v4"));
        assert_eq!(resolve("age"), None);
        assert_eq!(resolve("uuid/v4"), None);
        assert_eq!(resolve("NEO:age"), None);
        assert_eq!(resolve("neo:nope"), None);
        assert_eq!(resolve("./age.js"), None);
    }

    /// The registry keeps bare names as internal keys only. Error reporting can
    /// suggest the canonical spelling without ever treating the bare name as a
    /// module.
    #[test]
    fn bare_registry_names_are_internal_keys_only() {
        assert_eq!(resolve_name("age"), Some("neo:age"));
        assert_eq!(resolve_name("uuid/v7"), Some("neo:uuid/v7"));
        assert_eq!(resolve_name("neo:age"), None);
        assert!(is_registry_name("age"));
        assert!(!is_registry_name("neo:age"));
        assert!(!is_registry_name("lodash"));
    }

    /// Error messages list canonical specifiers only, never the internal bare
    /// registry names that a detached package resolver could later own.
    #[test]
    fn implemented_specifiers_lists_canonical_names_only() {
        let list = implemented_specifiers();
        assert!(list.starts_with("neo:age, neo:assert"));
        assert!(list.ends_with("neo:uuid/v4, neo:uuid/v7"));
        assert!(!list.contains(", age"));
    }

    #[test]
    fn shim_substitutes_the_build_target_constants() {
        let shim = neo_shim();
        assert!(!shim.contains(OS_PLATFORM_TOKEN));
        assert!(!shim.contains(OS_ARCH_TOKEN));
        assert!(
            shim.contains(&format!("platform: \"{}\"", std::env::consts::OS)),
            "platform literal is missing from the shim"
        );
        assert!(
            shim.contains(&format!("arch: \"{}\"", std::env::consts::ARCH)),
            "arch literal is missing from the shim"
        );
    }
}
