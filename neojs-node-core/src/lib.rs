//! Engine-neutral shared Rust backend for essential `node:*` built-in modules.
//!
//! `neojs-node-core` implements the safe, synchronous, cross-platform Rust
//! primitives shared across JavaScript runtimes (`neojs-core` / V8 and
//! `neojs-lite-core` / QuickJS) for:
//!
//! * [`path`]: POSIX and Win32 lexical path manipulation (`join`, `resolve`,
//!   `normalize`, `relative`, `dirname`, `basename`, `extname`, `parse`, `format`,
//!   `is_absolute`, `sep`, `delimiter`) with bounded input lengths.
//! * [`os`]: Node-vocabulary operating system metadata (`platform`, `arch`,
//!   `eol`, `homedir`, `tmpdir`, `hostname`, `cpus_count`, `endianness`,
//!   `os_type`, `machine`).
//! * [`process`]: Current working directory, environment inspection, process
//!   identifier, and monotonic nanosecond clock.
//! * [`child_process`]: Safe, shell-free synchronous process execution with
//!   concurrent bounded pipe draining and timeout reaping.
//! * [`url`]: Hardened `file:` URL to local path conversion and formatting.
//! * [`buffer`]: Bounded encoding and decoding for `utf8`, `hex`, `base64`,
//!   `base64url`, `latin1`, and `ascii`.
//! * [`canonical`]: Supported `node:*` module list and bare-name guard.
//! * [`fs`] and [`crypto`]: Direct re-exports of [`neojs_fs_core`] and
//!   [`neojs_crypto_core`] for `node:fs`, `node:fs/promises`, and `node:crypto`.
//!
//! This crate carries `#![forbid(unsafe_code)]`, zero JavaScript engine
//! dependencies (`v8`, `deno_core`, `rquickjs`), and zero `tokio` dependencies.
//!
//! # Example
//!
//! ```
//! use neojs_node_core::buffer::{Encoding, decode, encode};
//! use neojs_node_core::canonical::{SUPPORTED_NODE_MODULES, is_known_bare_node_module};
//! use neojs_node_core::path::{Style, join};
//!
//! assert_eq!(join(Style::Posix, &["/opt", "neojs", "bin"]).unwrap(), "/opt/neojs/bin");
//! let b64 = encode(b"neojs", Encoding::Base64);
//! assert_eq!(decode(&b64, Encoding::Base64).unwrap(), b"neojs");
//! assert!(SUPPORTED_NODE_MODULES.contains(&"node:path"));
//! assert!(is_known_bare_node_module("fs"));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod buffer;
pub mod canonical;
pub mod child_process;
pub mod error;
pub mod helpers;
pub mod os;
pub mod path;
pub mod process;
pub mod url;

pub use error::{NodeCoreError, NodeCoreErrorCode};
pub use helpers::NODE_COMPAT_FACTORY_SOURCE;
pub use neojs_crypto_core as crypto;
pub use neojs_fs_core as fs;
