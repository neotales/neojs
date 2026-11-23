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
//! reasons agree.
//!
//! # What works
//!
//! * [`Runtime::run_file`] reads an entry module and evaluates it.
//! * [`Runtime::run_source`] evaluates an in-memory module; its name is the
//!   identity and base for relative imports.
//! * Relative imports, absolute paths, and `file:` URLs are resolved from the
//!   importing module. Static imports and dynamic `import()` use the same loader.
//! * Top-level `await` works when it settles from the QuickJS job queue, for
//!   example `await Promise.resolve(value)`.
//! * After the entry settles, queued jobs are drained, so a trailing `.then` or
//!   an unawaited dynamic import still runs. The drain is bounded and fails
//!   closed on a nonconverging chain. A queued job that throws is an error.
//! * A promise that is still rejected without a handler when the queue empties
//!   is an error. A rejection handled later in the queue is not an error.
//!   Entry and rejection diagnostics never invoke getters, proxy traps,
//!   `toString`, or `String`.
//! * `console.log` and `console.error` write to the process streams.
//!
//! # Optional `neo:crypto`
//!
//! The `crypto` Cargo feature is a positive opt-in and is **off by default**.
//! With it enabled, the runtime registers a native QuickJS module under the
//! canonical specifier `neo:crypto`; `import ... from "neo:crypto"` (with a
//! case-insensitive `neo:` scheme) reaches that one instance. Neo built-ins
//! always require the prefix: a bare `crypto` specifier is **not** an alias and
//! stays a normal bare specifier, reserved for Node compatibility and packages.
//! The module exports exactly `generateEnvelopeKey`, `randomBytes`, `encrypt`,
//! and `decrypt`, and shares its NEO1 envelope and CSPRNG backend with the V8
//! runtime through `neojs-crypto-core`. It is synchronous because Lite has no
//! host asynchronous work; no async or other `crypto` export is advertised.
//!
//! The `neojs-lite` command enables the feature explicitly. A library embedder
//! that leaves it off gets no built-in module, and importing `neo:crypto`
//! fails with a reason that names the disabled feature. Cargo unifies features
//! across a build graph: if any dependency enables the feature, every user of
//! this crate in that build gets it too.
//!
//! [`enabled_builtin_modules`] is the build-time list of actually enabled
//! modules. It is the truthful integration point for a future introspection
//! surface; this crate does not install an introspection global.
//!
//! # What is not implemented yet
//!
//! The runtime is deliberately focused and does **not** claim parity with
//! `neojs` (V8):
//!
//! * **No TypeScript.** `.ts` and `.mts` entry or imported files are rejected
//!   with a clear reason. There is no type stripping in this crate.
//! * **No host asynchrony.** There are no timers, no host promise integration,
//!   and no event loop beyond QuickJS's own job queue. A top-level `await` that
//!   depends on an external event fails with an error instead of hanging.
//! * **Only the optional `neo:crypto` built-in.** No other `neo:` module and no
//!   `Neo.getBuiltInModule` registry exists in Lite.
//! * **No sandbox.** Permissions, host capabilities, and isolation are not
//!   implemented.
//!
//! These are current limitations, not planned API contracts. Check the tests
//! and the crate-level documentation for the surface that exists today.

mod builtins;
#[cfg(feature = "crypto")]
mod crypto;
mod loader;
mod runtime;

pub use builtins::enabled_builtin_modules;
pub use runtime::{Error, Runtime};
