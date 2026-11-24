//! Configuration, credential, trust, and result types for `neo:ssh`.

use std::fmt;
use std::time::Duration;

use neotales_ssh::hostkey::Ed25519PublicKey;
use neotales_ssh::known_hosts::KnownHosts;
use neotales_ssh::private_key;
use zeroize::Zeroizing;

use crate::error::SshError;

/// Largest command accepted by [`crate::Connection::exec`], in bytes, before the request is sent.
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;

/// Largest stdout captured from one [`crate::Connection::exec`], in bytes.
///
/// A command that produces more fails with [`crate::SshErrorCode::Limit`] and
/// closes the connection rather than draining unbounded output.
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Default deadline for the TCP connect plus handshake and authentication, in milliseconds.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 15_000;

/// Default deadline for one `exec`, in milliseconds.
pub const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;

/// Largest accepted `connectTimeoutMs`.
pub const MAX_CONNECT_TIMEOUT_MS: u64 = 120_000;

/// Largest accepted `execTimeoutMs`.
pub const MAX_EXEC_TIMEOUT_MS: u64 = 600_000;

/// Largest accepted host name, in bytes.
pub const MAX_HOST_BYTES: usize = 1024;

/// Largest accepted user name, in bytes.
pub const MAX_USERNAME_BYTES: usize = 256;

/// Largest accepted password, in bytes.
pub const MAX_PASSWORD_BYTES: usize = 4096;

/// Largest accepted passphrase, in bytes.
pub const MAX_PASSPHRASE_BYTES: usize = 1024;

/// Largest accepted `knownHosts` document, in bytes.
pub const MAX_KNOWN_HOSTS_BYTES: usize = 1024 * 1024;

/// Largest number of agent identities tried in explicit agent mode.
///
/// The agent enumerates at most this many; more fails with a fixed
/// [`crate::SshErrorCode::Agent`] error rather than an unbounded attempt set.
pub const MAX_AGENT_IDENTITIES: usize = 16;

/// Largest accepted host-key pin blob, in bytes.
///
/// An Ed25519 blob is 51 bytes. This bound covers the complete SSH string
/// encoding without accepting an arbitrarily large buffer.
pub const MAX_HOST_KEY_BYTES: usize = 1024;

/// Largest accepted private-key input (DER or PEM), in bytes.
pub const MAX_PRIVATE_KEY_BYTES: usize = private_key::MAX_PRIVATE_KEY_DER;

/// Largest number of simultaneously open connections held in [`crate::SshState`].
pub const MAX_OPEN_CONNECTIONS: usize = 1024;

#[derive(Clone)]
pub(crate) enum TrustInner {
    Pin(Ed25519PublicKey),
    KnownHosts(KnownHosts),
}

/// Explicit server host-key trust policy.
///
/// Exactly one of [`HostTrust::host_key`] or [`HostTrust::known_hosts`] must be
/// supplied. There is deliberately no insecure default, no trust-on-first-use
/// mode, and no ambient `~/.ssh/known_hosts` lookup.
#[derive(Clone)]
pub struct HostTrust {
    pub(crate) inner: TrustInner,
}

impl HostTrust {
    /// Trusts exactly one SSH-format Ed25519 public-key blob
    /// (`string "ssh-ed25519" || string key`).
    ///
    /// Fails with [`crate::SshErrorCode::Limit`] if `blob` exceeds
    /// [`MAX_HOST_KEY_BYTES`] or [`crate::SshErrorCode::HostKey`] if `blob` is
    /// not a valid Ed25519 SSH key blob.
    pub fn host_key(blob: &[u8]) -> Result<Self, SshError> {
        if blob.len() > MAX_HOST_KEY_BYTES {
            return Err(SshError::default_limit());
        }
        let pin = Ed25519PublicKey::from_blob(blob).map_err(|_| SshError::HostKey)?;
        Ok(Self {
            inner: TrustInner::Pin(pin),
        })
    }

    /// Trusts hosts matched by an explicit `known_hosts` document.
    ///
    /// Fails with [`crate::SshErrorCode::Limit`] if `text` exceeds
    /// [`MAX_KNOWN_HOSTS_BYTES`] or [`crate::SshErrorCode::KnownHosts`] if
    /// `text` cannot be parsed as a known-hosts document.
    pub fn known_hosts(text: &str) -> Result<Self, SshError> {
        if text.len() > MAX_KNOWN_HOSTS_BYTES {
            return Err(SshError::default_limit());
        }
        let hosts = KnownHosts::parse(text).map_err(|_| SshError::KnownHosts)?;
        Ok(Self {
            inner: TrustInner::KnownHosts(hosts),
        })
    }

    /// Validates and constructs a [`HostTrust`] from optional `hostKey` and
    /// `knownHosts` inputs.
    ///
    /// Exactly one of `host_key` or `known_hosts` must be `Some`; supplying
    /// neither or both fails with [`crate::SshErrorCode::HostKeyRequired`].
    pub fn from_parts(
        host_key: Option<&[u8]>,
        known_hosts: Option<&str>,
    ) -> Result<Self, SshError> {
        if let Some(blob) = host_key
            && blob.len() > MAX_HOST_KEY_BYTES
        {
            return Err(SshError::default_limit());
        }
        if let Some(text) = known_hosts
            && text.len() > MAX_KNOWN_HOSTS_BYTES
        {
            return Err(SshError::default_limit());
        }
        match (host_key, known_hosts) {
            (Some(blob), None) => Self::host_key(blob),
            (None, Some(text)) => Self::known_hosts(text),
            _ => Err(SshError::HostKeyRequired),
        }
    }
}

impl fmt::Debug for HostTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner {
            TrustInner::Pin(_) => f.write_str("HostTrust::HostKey(<ed25519>)"),
            TrustInner::KnownHosts(_) => f.write_str("HostTrust::KnownHosts(<parsed>)"),
        }
    }
}

/// Explicit authentication credential source.
///
/// Secrets are wrapped in [`Zeroizing`] so every validation error return and
/// drop wipes the Rust buffers. `Debug` redacts all secret material.
pub enum AuthConfig {
    /// Password authentication.
    Password(Zeroizing<String>),
    /// In-memory private key authentication (unencrypted PKCS#8 DER/PEM or
    /// binary/PEM OpenSSH container).
    PrivateKey {
        /// In-memory private key bytes.
        bytes: Zeroizing<Vec<u8>>,
        /// Optional passphrase for OpenSSH containers; rejected for PKCS#8.
        passphrase: Option<Zeroizing<String>>,
    },
    /// Explicit Unix-only `SSH_AUTH_SOCK` agent opt-in.
    ///
    /// The agent socket is never read before server host-key verification
    /// succeeds, and agent forwarding is never enabled.
    Agent,
}

impl AuthConfig {
    /// Validates and constructs an [`AuthConfig`] from optional binding inputs.
    ///
    /// Enforces that:
    /// * secrets respect [`MAX_PASSWORD_BYTES`], [`MAX_PRIVATE_KEY_BYTES`], and
    ///   [`MAX_PASSPHRASE_BYTES`];
    /// * exactly one of `password`, `private_key`, or `agent == true` is set
    ///   (otherwise [`crate::SshErrorCode::AuthMethod`]);
    /// * `passphrase` is only supplied with `private_key` (otherwise
    ///   [`crate::SshErrorCode::Argument`]);
    /// * `private_key` is non-empty (otherwise
    ///   [`crate::SshErrorCode::Credential`]).
    ///
    /// Any owned secrets passed in are wiped via [`Zeroizing`] on error.
    pub fn from_parts(
        password: Option<Zeroizing<String>>,
        private_key: Option<Zeroizing<Vec<u8>>>,
        agent: bool,
        passphrase: Option<Zeroizing<String>>,
    ) -> Result<Self, SshError> {
        if let Some(ref password) = password
            && password.len() > MAX_PASSWORD_BYTES
        {
            return Err(SshError::default_limit());
        }
        if let Some(ref key) = private_key
            && key.len() > MAX_PRIVATE_KEY_BYTES
        {
            return Err(SshError::default_limit());
        }
        if let Some(ref passphrase) = passphrase
            && passphrase.len() > MAX_PASSPHRASE_BYTES
        {
            return Err(SshError::default_limit());
        }

        let auth_count = usize::from(password.is_some())
            + usize::from(private_key.is_some())
            + usize::from(agent);
        if auth_count != 1 {
            return Err(SshError::AuthMethod);
        }
        if agent && passphrase.is_some() {
            return Err(SshError::default_argument());
        }
        if agent {
            return Ok(Self::Agent);
        }
        match (password, private_key) {
            (Some(password), None) => {
                if passphrase.is_some() {
                    return Err(SshError::default_argument());
                }
                Ok(Self::Password(password))
            }
            (None, Some(bytes)) => {
                if bytes.is_empty() {
                    return Err(SshError::Credential);
                }
                Ok(Self::PrivateKey { bytes, passphrase })
            }
            _ => Err(SshError::AuthMethod),
        }
    }
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("AuthConfig::Password(<redacted>)"),
            Self::PrivateKey { passphrase, .. } => f
                .debug_struct("AuthConfig::PrivateKey")
                .field("bytes", &"<redacted>")
                .field(
                    "passphrase",
                    &if passphrase.is_some() {
                        "Some(<redacted>)"
                    } else {
                        "None"
                    },
                )
                .finish(),
            Self::Agent => f.write_str("AuthConfig::Agent"),
        }
    }
}

/// Options for establishing one synchronous SSH [`crate::Connection`].
#[derive(Debug)]
pub struct ConnectOptions {
    /// Target host name or IP literal (non-empty, max [`MAX_HOST_BYTES`], no
    /// control or whitespace characters).
    pub host: String,
    /// Target TCP port (`1..=65535`).
    pub port: u16,
    /// SSH user name (non-empty, max [`MAX_USERNAME_BYTES`], no control
    /// characters).
    pub username: String,
    /// Explicit authentication credential source.
    pub auth: AuthConfig,
    /// Mandatory server host-key trust policy.
    pub trust: HostTrust,
    /// Budget covering each TCP dial attempt and socket inactivity during the
    /// handshake and authentication (`1ms..=MAX_CONNECT_TIMEOUT_MS`).
    pub connect_timeout: Duration,
    /// Budget covering each later [`crate::Connection::exec`] call
    /// (`1ms..=MAX_EXEC_TIMEOUT_MS`).
    pub exec_timeout: Duration,
}

impl ConnectOptions {
    /// Creates options with the documented default connect and exec timeouts
    /// ([`DEFAULT_CONNECT_TIMEOUT_MS`] and [`DEFAULT_EXEC_TIMEOUT_MS`]).
    #[must_use]
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        auth: AuthConfig,
        trust: HostTrust,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            auth,
            trust,
            connect_timeout: Duration::from_millis(DEFAULT_CONNECT_TIMEOUT_MS),
            exec_timeout: Duration::from_millis(DEFAULT_EXEC_TIMEOUT_MS),
        }
    }

    /// Sets [`ConnectOptions::connect_timeout`].
    #[must_use]
    pub const fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Sets [`ConnectOptions::exec_timeout`].
    #[must_use]
    pub const fn with_exec_timeout(mut self, timeout: Duration) -> Self {
        self.exec_timeout = timeout;
        self
    }
}

/// Captured result of one synchronous SSH command execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecOutput {
    /// Captured standard output bytes (standard error is discarded).
    pub stdout: Vec<u8>,
    /// Exit status reported by the remote process.
    pub status: u32,
}
