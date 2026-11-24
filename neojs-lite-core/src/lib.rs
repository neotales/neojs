//! A small embeddable QuickJS runtime for NeoJS Lite.
//!
//! This is the primary NeoJS Lite interface: embed [`Runtime`] and run a module
//! file. The `neojs-lite` executable is a secondary command-line wrapper.
//!
//! `neojs-lite-core` executes ECMAScript module files with [rquickjs] and
//! QuickJS-NG. It is a separate crate from the V8-based `neojs-core`; the
//! default `cargo build` in this workspace does not compile it and does not put
//! QuickJS or `rquickjs` in the V8 graph. Both runtimes share the engine-neutral
//! module policy in `neojs-module-policy`, so extension handling and failure
//! reasons agree, and the shared built-ins in `neojs-builtins`, so module
//! behavior and imports agree where the modules are implemented.
//!
//! # What works
//!
//! * [`Runtime::run_file`] reads an entry module and evaluates it.
//! * [`Runtime::run_source`] evaluates an in-memory module; its name is the
//!   identity and base for relative imports.
//! * [`Runtime::run_tests`] runs tests registered with `neo:test` and returns a
//!   [`TestReport`] with pass/fail/pending/unhandled/unsettled counts.
//! * Relative imports, absolute paths, and `file:` URLs are resolved from the
//!   importing module. Static imports and dynamic `import()` use the same loader.
//! * With the opt-in `manifest` Cargo feature enabled, embedders can also
//!   depend on `neojs-manifest`, build an `ImportResolver` via
//!   `neojs_manifest::discover_manifest` (`ImportResolver::from_manifest` or
//!   `Workspace::load` + `ImportResolver::from_workspace`), and pass it to
//!   `Runtime::with_import_resolver`; [`Runtime::new`] performs zero implicit
//!   manifest discovery.
//! * Top-level `await` works when it settles from the QuickJS job queue, for
//!   example `await Promise.resolve(value)`.
//! * After the entry settles, queued jobs are drained, so a trailing `.then` or
//!   an unawaited dynamic import still runs. The drain is bounded and fails
//!   closed on a nonconverging chain. A queued job that throws is an error.
//! * A promise that is still rejected without a handler when the queue empties
//!   is an error (or recorded in [`TestReport::unhandled`] when `neo:test` is
//!   active). A rejection handled later in the queue is not an error. Entry and
//!   rejection diagnostics never invoke getters, proxy traps, `toString`, or
//!   `String`.
//! * `console.log` and `console.error` write to the process streams.
//! * A minimal `Neo` global reports `engine` `"neo-lite"`. With the default
//!   `neo-assert`, `neo-chars`, `neo-fs`, `neo-os`, `neo-strings`, `neo-test`,
//!   and `neo-uuid` features the registry contains the canonical shared modules
//!   `neo:assert`, `neo:chars`, `neo:fs`, `neo:os`, `neo:strings`, `neo:test`,
//!   `neo:uuid`, `neo:uuid/v4`, and `neo:uuid/v7`.
//!   `Neo.getBuiltInModulesList()` returns one frozen array of canonical
//!   specifiers, and `Neo.getBuiltInModule(specifier)` returns the frozen
//!   registry object whose named exports share identity with the module
//!   namespace exports; the namespace itself is a distinct object. A bare name
//!   or another scheme returns `undefined`; a non-string name throws
//!   `TypeError`. With `default-features = false` the registry and list are
//!   empty and no module source is compiled in. The implementations come from
//!   `neojs-builtins` (`chars`, `os`, `strings`, and `uuid` bound as native
//!   QuickJS modules, `assert` and `test` evaluated from the shared ECMAScript
//!   factories with native QuickJS Promise and queue hooks) and `neojs-fs-core`
//!   (`fs` bound as a native QuickJS module).
//!
//! # What is not implemented yet
//!
//! The runtime is deliberately focused and does **not** claim parity with
//! `neojs` (V8):
//!
//! * **No TypeScript.** `.ts` and `.mts` entry or imported files are rejected
//!   with a clear reason. There is no type stripping in this crate.
//! * **No host timers or external asynchrony.** There are no timers and no event
//!   loop beyond QuickJS's own job queue. A top-level `await` that depends on
//!   an external event fails with an error instead of hanging, and
//!   `test(..., { timeout })` is rejected explicitly.
//! * **Most built-in `neo:` modules are absent.** With their features enabled
//!   this crate implements the shared `neo:assert`, `neo:chars`, `neo:fs`,
//!   `neo:os`, `neo:strings`, `neo:test`, `neo:uuid`, `neo:uuid/v4`, and
//!   `neo:uuid/v7` modules only. Other V8 modules are not importable here, and
//!   the `Neo` list does not claim them.
//! * **No sandbox.** Permissions, host capabilities, and isolation are not
//!   implemented.
//!
//! These are current limitations, not planned API contracts. Check the tests
//! and the crate-level documentation for the surface that exists today.

mod loader;
mod runtime;

#[cfg(feature = "neo-chars")]
mod chars;
#[cfg(feature = "neo-fs")]
mod fs;
mod neo;
#[cfg(feature = "neo-os")]
mod os;
#[cfg(any(feature = "neo-assert", feature = "neo-test"))]
mod promise;
#[cfg(feature = "neo-strings")]
mod strings;
#[cfg(feature = "neo-uuid")]
mod uuid;

#[cfg(feature = "manifest")]
pub use neojs_manifest::ImportResolver;
pub use runtime::{Error, Runtime, TestReport};

/// Test-only hooks. Compiled only with the off-by-default `test-support`
/// feature, which a normal build never enables.
#[cfg(all(
    feature = "test-support",
    any(feature = "neo-strings", feature = "neo-uuid")
))]
#[doc(hidden)]
pub mod test_support {
    /// Return `(units, windows)` copied from QuickJS strings by the last
    /// `indexOfFold`/`lastIndexOfFold` calls.
    ///
    /// Used to prove that a small `fromIndex` reads a bounded prefix instead of
    /// copying the whole text.
    #[cfg(feature = "neo-strings")]
    pub fn strings_text_reads() -> (usize, usize) {
        crate::strings::test_reads()
    }

    /// Return the number of source read attempts, including a failed one.
    #[cfg(feature = "neo-strings")]
    pub fn strings_text_attempts() -> usize {
        crate::strings::test_attempts()
    }

    /// Reset the streaming-search read counters and injected failure.
    #[cfg(feature = "neo-strings")]
    pub fn reset_strings_text_reads() {
        crate::strings::reset_test_reads();
    }

    /// Make the read after `loads` successful windows fail with a
    /// guest-visible exception.
    #[cfg(feature = "neo-strings")]
    pub fn fail_strings_after_loads(loads: usize) {
        crate::strings::fail_after_loads(loads);
    }

    /// Make the next UUID entropy draw fail with `NEO_UUID_RANDOM`.
    ///
    /// The injection is consumed once and exists so the JavaScript error path
    /// can be tested deterministically without a production hook.
    #[cfg(feature = "neo-uuid")]
    pub fn fail_next_uuid_random() {
        crate::uuid::fail_next_uuid_random();
    }
}
