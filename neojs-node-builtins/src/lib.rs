#![forbid(unsafe_code)]
//! Engine-neutral policy and synchronous process core for opt-in Node support.
//!
//! This crate is shared by the V8 adapter (`neojs-node`) and the QuickJS adapter
//! (`neojs-lite-node`). It depends on neither engine. It owns the decisions that
//! must agree between them:
//!
//! * which Node module specifiers are implemented;
//! * how a child-process program is authorised;
//! * the bounded synchronous process runner and its error codes;
//! * the JSON protocol used by both JavaScript module wrappers.
//!
//! # Support, exactly
//!
//! The only implemented Node module is `node:child_process`. Its module object
//! provides `spawnSync` and `execFileSync` with `encoding: "utf8"` required.
//! `spawn`, `exec`, `execFile`, `execSync`, `fork`, and every asynchronous or
//! callback form throw `ERR_NODE_UNSUPPORTED`; they are not implemented as a
//! blocking call or a fake resolved Promise.
//!
//! Importing a Node module never reaches the NeoJS built-in registry. A Node
//! specifier is always written with the `node:` prefix, and a bare name such as
//! `child_process` is never an alias. In an opted-in runtime, a bare name that
//! is a familiar Node module name (for example `crypto` or `path`) is rejected
//! instead of resolving to a Neo module, so the Neo API must be written with its
//! own `neo:` prefix. If a Node module is not implemented,
//! resolution fails with a reason; it is never substituted with a `neo:` module.
//!
//! # Authority and limits
//!
//! Process execution is disabled by default. A [`ChildProcessPolicy`] starts
//! with no allowed programs, so the module can be imported but every execution
//! is denied. An embedding application must add an exact, canonical executable
//! path. There is no `PATH` lookup, no shell, no inherited environment, and no
//! `stdio: inherit`.
//!
//! The policy also caps wall-clock time, input, output per stream, argument
//! count and bytes, and environment size. A timeout or an output overflow kills
//! the direct child and returns partial output with a flag; the JavaScript
//! wrapper turns that into the documented Node-shaped error.
//!
//! This is **not a sandbox**. An allowed executable runs with the authority of
//! the neojs process. It can read and write files, use the network, and start
//! its own descendants according to the operating system's permissions. The
//! timeout kills the direct child only; a descendant that outlives it is not
//! tracked or killed. The policy is an explicit permission gate and a resource
//! bound, not containment.

mod child_process;
mod module;
mod protocol;

pub use child_process::{ChildProcessError, ChildProcessPolicy, Limits, NodeOptions, PolicyError};
pub use module::{
    CHILD_PROCESS_SPECIFIER, SUPPORTED_MODULES, is_known_node_bare_name, module_source,
    unsupported_module_reason,
};
pub use protocol::handle_request;
