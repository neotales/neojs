#![forbid(unsafe_code)]
//! Engine-neutral policy and synchronous process core for opt-in Node support.
//!
//! This crate is shared by the V8 adapter (`neojs-node`) and the QuickJS adapter
//! (`neojs-lite-node`). It depends on neither engine. It owns the decisions that
//! must agree between them:
//!
//! * which Node module specifiers are implemented;
//! * how a child-process program is authorised;
//! * the synchronous process runner with bounded buffers and a polling/kill
//!   deadline, and its error codes;
//! * the bounded JSON request protocol used by the shared protocol tests.
//!
//! # Support, exactly
//!
//! The implemented Node modules are `node:child_process` and `node:path`.
//! `node:child_process` provides `spawnSync` and `execFileSync` with
//! `encoding: "utf8"` required. `spawn`, `exec`, `execFile`, `execSync`,
//! `fork`, and every asynchronous or callback form throw
//! `ERR_NODE_UNSUPPORTED`; they are not implemented as a blocking call or a
//! fake resolved Promise. `node:path` provides `basename`, `dirname`,
//! `extname`, `format`, `isAbsolute`, `join`, `normalize`, `parse`, `relative`,
//! `resolve`, `toNamespacedPath`, `sep`, `delimiter`, and the `posix`/`win32`
//! views. `matchesGlob` throws `ERR_NODE_UNSUPPORTED`.
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
//! The policy also sets a polling/kill deadline and caps input, output per
//! stream, argument count and bytes, and environment size. The deadline is not
//! a hard wall-clock bound on the whole call: after the kill the runner reaps
//! the direct child with a blocking `wait()`, so the call can outlive the
//! deadline if the operating system does not complete the kill or reap. On
//! Unix, pipes are non-blocking and driven by `poll` on the calling thread; no
//! reader or writer thread is created. A timeout or output overflow kills the
//! child's process group, then drains for a short bounded grace. A direct child
//! that exits while a descendant still holds a piped descriptor gets a bounded
//! drain followed by a group kill; because output may be incomplete, that case
//! is reported as an explicit incomplete error, never a successful status. An
//! unexpected pipe read error is reported as an explicit I/O error, never
//! treated as end-of-file. A descendant that creates a new process group and
//! escapes is not tracked. The optional JSON protocol used by the shared tests
//! has a hard serialized-size cap before parsing; neither engine adapter uses
//! it. Unix pipe polling and process-group kill are implemented; another
//! platform rejects before spawning.
//!
//! This is **not a sandbox**. An allowed executable runs with the authority of
//! the neojs process. It can read and write files, use the network, and start
//! its own descendants according to the operating system's permissions. The
//! timeout and inherited-pipe cleanup target the child's process group; a
//! descendant that escapes that group is not tracked or killed. The allowlist
//! matches canonical paths, not inodes or contents: the embedding application
//! must protect the executable and every ancestor directory from untrusted
//! writes. The policy is an explicit permission gate and a resource bound, not
//! containment.

mod child_process;
mod module;
mod path;
mod protocol;

pub use child_process::{
    ChildProcessError, ChildProcessPolicy, Limits, MAX_REQUEST_BYTES, NodeOptions, PolicyError,
    SyncRequest, SyncResult, run_sync,
};
pub use module::{
    CHILD_PROCESS_SPECIFIER, PATH_SPECIFIER, SUPPORTED_MODULES, is_known_node_bare_name,
    unsupported_module_reason,
};
pub use path::{
    MAX_PATH_INPUT_BYTES, ParsedPath, PathFlavor, PathObject, basename, dirname, extname, format,
    host_cwd, host_flavor, is_absolute, join, normalize, parse, relative, resolve,
    to_namespaced_path,
};
pub use protocol::handle_request;
