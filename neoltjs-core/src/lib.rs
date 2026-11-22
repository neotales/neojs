//! A small embeddable QuickJS runtime for NeoLTJS.
//!
//! `neoltjs-core` executes ECMAScript module files with [rquickjs] and
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
//! * `console.log` and `console.error` write to the process streams.
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
//! * **No built-in `neo:` modules.** The QuickJS crate currently loads local
//!   ECMAScript files only.
//! * **No sandbox.** Permissions, host capabilities, and isolation are not
//!   implemented.
//!
//! These are current limitations, not planned API contracts. Check the tests
//! and the CLI help for the surface that exists today.

mod loader;
mod runtime;

pub use runtime::{Error, Runtime};
