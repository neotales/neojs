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
//! reasons agree, and the shared `neo:chars` and `neo:strings` Rust logic in
//! `neojs-builtins`, so module behavior and imports agree where the modules are
//! implemented.
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
//! * A minimal `Neo` global reports `engine` `"neo-lite"`. With the default
//!   `neo-chars` and `neo-strings` features the registry contains the canonical
//!   shared modules `neo:chars` and `neo:strings`.
//!   `Neo.getBuiltInModulesList()` returns one frozen array of canonical
//!   specifiers, and `Neo.getBuiltInModule(specifier)` returns the frozen
//!   registry object whose named functions are the same function objects the
//!   module namespace exports; the namespace itself is a distinct object. A
//!   bare name or another scheme returns `undefined`; a non-string name throws
//!   `TypeError`. With `default-features = false` the registry and list are
//!   empty and no module source is compiled in. The implementations come from
//!   the engine-neutral `neojs-builtins` crate, bound as native QuickJS
//!   modules; the V8 runtime uses the same Rust logic.
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
//! * **Most built-in `neo:` modules are absent.** With their features enabled
//!   this crate implements the shared `neo:chars` and `neo:strings` modules
//!   only. Other V8 modules are not importable here, and the `Neo` list does
//!   not claim them.
//! * **No sandbox.** Permissions, host capabilities, and isolation are not
//!   implemented.
//!
//! These are current limitations, not planned API contracts. Check the tests
//! and the crate-level documentation for the surface that exists today.

mod loader;
mod runtime;

#[cfg(feature = "neo-chars")]
mod chars;
mod neo;
#[cfg(feature = "neo-strings")]
mod strings;

pub use runtime::{Error, Runtime};
