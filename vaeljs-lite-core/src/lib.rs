//! A small embeddable QuickJS runtime for VaelJS Lite.
//!
//! This is the primary VaelJS Lite interface: embed [`Runtime`] and run a module
//! file. The `vaeljs-lite` executable is a secondary command-line wrapper.
//!
//! `vaeljs-lite-core` executes ECMAScript and erasable TypeScript module files
//! with [rquickjs] and QuickJS-NG. It is a separate crate from the V8-based
//! `vaeljs-core`; the default `cargo build` in this workspace does not compile it
//! and does not put QuickJS or `rquickjs` in the V8 graph. Both runtimes share
//! the engine-neutral module policy and erasable TypeScript stripper in
//! `vaeljs-module-policy`, so extension handling, TypeScript diagnostics, and
//! failure reasons agree, and the shared built-ins in `vaeljs-builtins`, so
//! module behavior and imports agree where the modules are implemented.
//!
//! # What works
//!
//! * [`Runtime::run_file`] reads an entry module (`.js`, `.mjs`, extensionless,
//!   and `.ts`/`.mts` when the `typescript` feature is enabled) and evaluates it.
//! * [`Runtime::run_source`] evaluates an in-memory module; its name is the
//!   identity and base for relative imports.
//! * [`Runtime::run_tests`] runs tests registered with `vael:test` and returns a
//!   [`TestReport`] with pass/fail/pending/unhandled/unsettled counts.
//! * Relative imports, absolute paths, and `file:` URLs are resolved from the
//!   importing module. Static imports and dynamic `import()` use the same loader.
//! * Erasable TypeScript (`.ts` and `.mts`), enabled by default via the
//!   `typescript` feature (and opt-outable with `--no-default-features`), strips
//!   type annotations, interfaces, type aliases, generics, `as`/`satisfies`, and
//!   `import type` via `vaeljs-module-policy` while preserving line and column
//!   numbers for QuickJS stack traces. No type checking happens, and constructs
//!   that need a transform (`enum`, `namespace`, parameter properties) are
//!   rejected. `.tsx`, `.jsx`, `.cts`, and declaration files are rejected by
//!   extension. When the `typescript` feature is disabled, `.ts` and `.mts` are
//!   rejected with [`Error::UnsupportedEntry`] / a loader error stating
//!   TypeScript is not enabled in the build, and `deno_ast` is absent from the
//!   dependency graph.
//! * With the opt-in `manifest` Cargo feature enabled, embedders can also
//!   depend on `vaeljs-manifest`, build an `ImportResolver` via
//!   `vaeljs_manifest::discover_manifest` (`ImportResolver::from_manifest` or
//!   `Workspace::load` + `ImportResolver::from_workspace`), and pass it to
//!   `Runtime::with_import_resolver`; [`Runtime::new`] performs zero implicit
//!   manifest discovery.
//! * Top-level `await` works when it settles from the QuickJS job queue, for
//!   example `await Promise.resolve(value)`.
//! * After the entry settles, queued jobs are drained, so a trailing `.then` or
//!   an unawaited dynamic import still runs. The drain is bounded and fails
//!   closed on a nonconverging chain. A queued job that throws is an error.
//! * A promise that is still rejected without a handler when the queue empties
//!   is an error (or recorded in [`TestReport::unhandled`] when `vael:test` is
//!   active). A rejection handled later in the queue is not an error. Entry and
//!   rejection diagnostics never invoke getters, proxy traps, `toString`, or
//!   `String`.
//! * `console.{log, debug, info, warn, error}` write formatted arguments to
//!   stdout/stderr (when `vael-console` is enabled; minimal `log`/`error` when
//!   opted out).
//! * A minimal `Vael` global reports `engine` `"vael-lite"`. With the default
//!   `vael-assert`, `vael-chars`, `vael-console`, `vael-crypto`, `vael-fs`, `vael-os`,
//!   `vael-path`, `vael-slices`, `vael-ssh`, `vael-strings`, `vael-test`, and
//!   `vael-uuid` features the registry contains the canonical shared modules
//!   `vael:assert`, `vael:chars`, `vael:console`, `vael:crypto`, `vael:fs`, `vael:os`,
//!   `vael:path`, `vael:slices`, `vael:ssh`, `vael:strings`, `vael:test`, `vael:uuid`,
//!   `vael:uuid/v4`, and `vael:uuid/v7`.
//!   `Vael.getBuiltInModulesList()` returns one frozen array of canonical
//!   specifiers, and `Vael.getBuiltInModule(specifier)` returns the cached
//!   registry object (frozen for every module except `vael:console`, which is
//!   the live mutable `globalThis.console` object) whose named exports share
//!   identity with the module namespace exports; the namespace itself is a
//!   distinct object. A bare name
//!   or another scheme returns `undefined`; a non-string name throws
//!   `TypeError`. With `default-features = false` the registry and list are
//!   empty and no module source is compiled in. The implementations come from
//!   `vaeljs-builtins` (`chars`, `os`, `strings`, and `uuid` bound as native
//!   QuickJS modules; `assert`, `console`, `path`, `slices`, and `test`
//!   evaluated from the shared ECMAScript factories with native QuickJS host
//!   hooks), `vaeljs-crypto-core`
//!   (`crypto` synchronous exports bound as a native QuickJS module),
//!   `vaeljs-fs-core` (`fs` bound as a native QuickJS module), and
//!   `vaeljs-ssh-core` (`ssh` bound as a native QuickJS module).
//!
//! # What is not implemented yet
//!
//! The runtime is deliberately focused and does **not** claim parity with
//! `vaeljs` (V8):
//!
//! * **No non-erasable TypeScript, JSX, or type checking.** Only erasable `.ts`
//!   and `.mts` syntax is supported (and only when the `typescript` feature is
//!   enabled). Transforms (`enum`, `namespace`, parameter properties), `.tsx`,
//!   `.jsx`, `.cts`, and declaration files are rejected.
//! * **No host timers or external asynchrony.** There are no timers and no event
//!   loop beyond QuickJS's own job queue. A top-level `await` that depends on
//!   an external event fails with an error instead of hanging, and
//!   `test(..., { timeout })` is rejected explicitly.
//! * **Most V8-only async/platform `vael:` modules are absent.** With their
//!   features enabled this crate implements the shared `vael:assert`, `vael:chars`,
//!   `vael:console`, `vael:crypto` (13 synchronous exports only), `vael:fs`,
//!   `vael:os`, `vael:path`, `vael:slices`, `vael:ssh`, `vael:strings`, `vael:test`,
//!   `vael:uuid`, `vael:uuid/v4`, and `vael:uuid/v7` modules only. Other V8
//!   modules (`vael:age`, `vael:cron`, `vael:stdio`, and `*Async` crypto helpers)
//!   are not importable here, and the `Vael` list does not claim them.
//! * **No sandbox.** Permissions, host capabilities, and isolation are not
//!   implemented.
//!
//! These are current limitations, not planned API contracts. Check the tests
//! and the crate-level documentation for the surface that exists today.

mod loader;
mod runtime;

#[cfg(feature = "vael-chars")]
mod chars;
#[cfg(feature = "vael-crypto")]
mod crypto;
#[cfg(feature = "vael-exec")]
mod exec;
#[cfg(feature = "vael-fetch")]
mod fetch;
#[cfg(feature = "vael-fs")]
mod fs;
#[cfg(any(
    feature = "vael-crypto",
    feature = "vael-exec",
    feature = "vael-fetch",
    feature = "vael-fs",
    feature = "vael-ssh",
    feature = "vael-template",
    feature = "node-compat"
))]
mod native;
#[cfg(feature = "node-compat")]
mod node_compat;
#[cfg(feature = "npm")]
mod npm_loader;
#[cfg(feature = "vael-os")]
mod os;
#[cfg(any(feature = "vael-assert", feature = "vael-test"))]
mod promise;
#[cfg(feature = "remote")]
mod remote_loader;
#[cfg(feature = "vael-ssh")]
mod ssh;
#[cfg(feature = "vael-strings")]
mod strings;
#[cfg(feature = "vael-template")]
mod template;
#[cfg(feature = "vael-uuid")]
mod uuid;
mod vael;

#[cfg(feature = "npm")]
pub use npm_loader::NpmOptions;
pub use runtime::{Error, Runtime, RuntimeOptions, TestReport};
#[cfg(feature = "manifest")]
pub use vaeljs_manifest::ImportResolver;
#[cfg(feature = "remote")]
pub use vaeljs_remote::RemoteOptions;

/// Test-only hooks. Compiled only with the off-by-default `test-support`
/// feature, which a normal build never enables.
#[cfg(all(
    feature = "test-support",
    any(feature = "vael-strings", feature = "vael-uuid", feature = "remote")
))]
#[doc(hidden)]
pub mod test_support {
    /// Scripted HTTP response for `Runtime::with_options_and_transport` tests.
    #[cfg(feature = "remote")]
    pub use vaeljs_remote::TestRemoteResponse;
    /// Return `(units, windows)` copied from QuickJS strings by the last
    /// `indexOfFold`/`lastIndexOfFold` calls.
    ///
    /// Used to prove that a small `fromIndex` reads a bounded prefix instead of
    /// copying the whole text.
    #[cfg(feature = "vael-strings")]
    pub fn strings_text_reads() -> (usize, usize) {
        crate::strings::test_reads()
    }

    /// Return the number of source read attempts, including a failed one.
    #[cfg(feature = "vael-strings")]
    pub fn strings_text_attempts() -> usize {
        crate::strings::test_attempts()
    }

    /// Reset the streaming-search read counters and injected failure.
    #[cfg(feature = "vael-strings")]
    pub fn reset_strings_text_reads() {
        crate::strings::reset_test_reads();
    }

    /// Make the read after `loads` successful windows fail with a
    /// guest-visible exception.
    #[cfg(feature = "vael-strings")]
    pub fn fail_strings_after_loads(loads: usize) {
        crate::strings::fail_after_loads(loads);
    }

    /// Make the next UUID entropy draw fail with `VAEL_UUID_RANDOM`.
    ///
    /// The injection is consumed once and exists so the JavaScript error path
    /// can be tested deterministically without a production hook.
    #[cfg(feature = "vael-uuid")]
    pub fn fail_next_uuid_random() {
        crate::uuid::fail_next_uuid_random();
    }
}
