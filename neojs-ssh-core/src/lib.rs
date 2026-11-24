//! Engine-neutral synchronous SSH core for `neo:ssh`.
//!
//! This crate provides synchronous, fail-closed SSH command execution shared
//! by both the V8 runtime (`neojs-core`) and the QuickJS runtime
//! (`neojs-lite-core`). It carries zero JavaScript engine dependencies and zero
//! `tokio` dependencies.
//!
//! The SSH protocol, key exchange, authentication, transport protection,
//! session channels, and host-key verification are provided by the
//! MIT-licensed `neotales-ssh` crate.
//!
//! # Security and resource contract
//!
//! * **Mandatory trust**: [`HostTrust`] requires exactly one of an explicit
//!   SSH-format Ed25519 public-key blob ([`HostTrust::host_key`]) or an
//!   explicit `known_hosts` document ([`HostTrust::known_hosts`]). There is no
//!   unknown-host mode, no trust-on-first-use, and no ambient
//!   `~/.ssh/known_hosts` lookup.
//! * **Explicit credentials & zeroization**: [`AuthConfig`] requires exactly
//!   one of a password, an in-memory private key (PKCS#8 DER/PEM or OpenSSH
//!   binary/PEM, with an optional passphrase for OpenSSH containers only), or
//!   an explicit Unix-only `Agent` opt-in. Secrets are held in [`Zeroizing`]
//!   buffers so validation failures and drops wipe Rust copies, and error
//!   strings never echo credentials, keys, commands, stdout, or peer banners.
//! * **Agent isolation**: [`AuthConfig::Agent`] reads `SSH_AUTH_SOCK` only
//!   after server host-key verification succeeds, caps enumeration at
//!   [`MAX_AGENT_IDENTITIES`] (16) identities, and never forwards the agent.
//! * **Bounded deadlines and sizes**: TCP dial, handshake/auth, and `exec`
//!   sessions enforce configured deadlines ([`DEFAULT_CONNECT_TIMEOUT_MS`] /
//!   [`DEFAULT_EXEC_TIMEOUT_MS`]) and documented bounds ([`MAX_COMMAND_BYTES`],
//!   [`MAX_OUTPUT_BYTES`], [`MAX_OPEN_CONNECTIONS`]).
//! * **Fail-closed connection lifecycle**: any `exec` error after borrowing a
//!   connection closes and drops the transport.
//!
//! # Example
//!
//! ```
//! use neojs_ssh_core::{AuthConfig, HostTrust, SshErrorCode, Zeroizing};
//!
//! // Missing both hostKey and knownHosts is refused with NEO_SSH_HOST_KEY_REQUIRED.
//! let trust_err = HostTrust::from_parts(None, None).unwrap_err();
//! assert_eq!(trust_err.code(), SshErrorCode::HostKeyRequired);
//!
//! // Supplying multiple auth methods is refused with NEO_SSH_AUTH_METHOD.
//! let auth_err = AuthConfig::from_parts(
//!     Some(Zeroizing::new("secret".to_owned())),
//!     None,
//!     true,
//!     None,
//! )
//! .unwrap_err();
//! assert_eq!(auth_err.code(), SshErrorCode::AuthMethod);
//! ```
//!
//! Connecting to a host with an explicit Ed25519 host-key pin and running one
//! command:
//!
//! ```no_run
//! use neojs_ssh_core::{AuthConfig, ConnectOptions, HostTrust, Zeroizing, connect};
//!
//! # fn main() -> Result<(), neojs_ssh_core::SshError> {
//! let host_key_blob: &[u8] = &[]; // SSH wire-format "ssh-ed25519" blob
//! let trust = HostTrust::host_key(host_key_blob)?;
//! let auth = AuthConfig::Password(Zeroizing::new("password".to_owned()));
//! let options = ConnectOptions::new("127.0.0.1", 22, "user", auth, trust);
//!
//! let mut conn = connect(options)?;
//! let output = conn.exec(b"uname -a")?;
//! assert_eq!(output.status, 0);
//! conn.close();
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod error;
mod types;

pub use client::{Connection, SshState, connect, validate_host, validate_username};
pub use error::{SshError, SshErrorCode};
pub use types::{
    AuthConfig, ConnectOptions, DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_EXEC_TIMEOUT_MS, ExecOutput,
    HostTrust, MAX_AGENT_IDENTITIES, MAX_COMMAND_BYTES, MAX_CONNECT_TIMEOUT_MS,
    MAX_EXEC_TIMEOUT_MS, MAX_HOST_BYTES, MAX_HOST_KEY_BYTES, MAX_KNOWN_HOSTS_BYTES,
    MAX_OPEN_CONNECTIONS, MAX_OUTPUT_BYTES, MAX_PASSPHRASE_BYTES, MAX_PASSWORD_BYTES,
    MAX_PRIVATE_KEY_BYTES, MAX_USERNAME_BYTES,
};
pub use zeroize::Zeroizing;

/// The canonical `neo:ssh` import specifier and module registry key.
pub const SPECIFIER: &str = "neo:ssh";

/// The named exports of the `neo:ssh` JavaScript module, in property order.
pub const EXPORTS: &[&str] = &[
    "connect",
    "MAX_COMMAND_BYTES",
    "MAX_OUTPUT_BYTES",
    "DEFAULT_CONNECT_TIMEOUT_MS",
    "DEFAULT_EXEC_TIMEOUT_MS",
];

/// Test-only hooks. Compiled only with the off-by-default `test-support`
/// feature, which a normal build never enables.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support {
    /// Makes the next required peer-close reply fail with
    /// [`crate::SshErrorCode::Io`].
    ///
    /// Has no effect unless `test-support` is enabled.
    pub fn fail_next_peer_close_reply() {
        crate::client::FAIL_NEXT_PEER_CLOSE_REPLY.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
