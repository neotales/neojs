//! Typed SSH error codes and error values for `neo:ssh`.
//!
//! Error messages describe the programmatic and human cause without echoing
//! passwords, passphrases, key bytes, command bytes, stdout bytes, or peer
//! banners.

use std::fmt;
use std::io;

use thiserror::Error;

/// Stable machine-readable error codes for [`SshError`].
///
/// Bindings expose [`SshErrorCode::as_str`] as the JavaScript error's `code`
/// property. Callers match on the variant or code string rather than parsing
/// human-readable error messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum SshErrorCode {
    /// An option value is structurally invalid.
    Argument,
    /// Neither or both of `hostKey` and `knownHosts` were supplied.
    HostKeyRequired,
    /// The host-key pin blob is invalid or the server host key is not trusted.
    HostKey,
    /// The `knownHosts` text is not a valid known-hosts document.
    KnownHosts,
    /// Not exactly one of `password`, `privateKey`, or `agent: true` was supplied.
    AuthMethod,
    /// The private key or passphrase is unsupported or malformed.
    Credential,
    /// The opted-in SSH agent is unavailable, empty, oversized, or refused to sign.
    Agent,
    /// DNS resolution or TCP connection failed.
    Connect,
    /// A documented connect or execution deadline expired.
    Timeout,
    /// The SSH handshake or algorithm negotiation failed.
    Handshake,
    /// User authentication failed.
    Auth,
    /// The server rejected a session request or the session protocol failed.
    Session,
    /// An input, captured output, or open-connection count exceeded a documented bound.
    Limit,
    /// The connection is closed or already unusable.
    Closed,
    /// The protected transport reported an I/O failure.
    Io,
    /// An unexpected host-side failure occurred.
    Internal,
}

impl SshErrorCode {
    /// Returns the stable `NEO_SSH_*` error code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Argument => "NEO_SSH_ARGUMENT",
            Self::HostKeyRequired => "NEO_SSH_HOST_KEY_REQUIRED",
            Self::HostKey => "NEO_SSH_HOST_KEY",
            Self::KnownHosts => "NEO_SSH_KNOWN_HOSTS",
            Self::AuthMethod => "NEO_SSH_AUTH_METHOD",
            Self::Credential => "NEO_SSH_CREDENTIAL",
            Self::Agent => "NEO_SSH_AGENT",
            Self::Connect => "NEO_SSH_CONNECT",
            Self::Timeout => "NEO_SSH_TIMEOUT",
            Self::Handshake => "NEO_SSH_HANDSHAKE",
            Self::Auth => "NEO_SSH_AUTH",
            Self::Session => "NEO_SSH_SESSION",
            Self::Limit => "NEO_SSH_LIMIT",
            Self::Closed => "NEO_SSH_CLOSED",
            Self::Io => "NEO_SSH_IO",
            Self::Internal => "NEO_SSH_INTERNAL",
        }
    }

    /// Returns the default fixed human-readable message for this error code.
    #[must_use]
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::Argument => "an SSH option is invalid",
            Self::HostKeyRequired => {
                "connect requires exactly one of hostKey or knownHosts; no insecure default is available"
            }
            Self::HostKey => "the server host key was not trusted; refusing to connect",
            Self::KnownHosts => "the knownHosts text is not a valid known-hosts document",
            Self::AuthMethod => "connect requires exactly one of password, privateKey, or agent",
            Self::Credential => "the private key or passphrase is not a supported credential",
            Self::Agent => "the SSH agent is unavailable or rejected the request",
            Self::Connect => "cannot connect to the SSH server",
            Self::Timeout => "the SSH operation exceeded its deadline",
            Self::Handshake => "the SSH handshake failed",
            Self::Auth => "SSH authentication failed",
            Self::Session => "the SSH session failed",
            Self::Limit => "SSH data exceeds a documented size limit",
            Self::Closed => "the SSH connection is closed",
            Self::Io => "the SSH transport failed",
            Self::Internal => "the SSH module encountered an internal error",
        }
    }
}

impl fmt::Display for SshErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

struct OptSource<'a>(&'a Option<io::Error>);

impl fmt::Display for OptSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = self.0 {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

/// Typed error returned by every fallible operation in [`crate`].
///
/// Each variant maps deterministically to a stable [`SshErrorCode`] via
/// [`SshError::code`], preserves any underlying [`std::io::Error`] via
/// [`std::error::Error::source`], and never echoes passwords, passphrases, key
/// bytes, command bytes, stdout bytes, or peer banners.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SshError {
    /// An option value is structurally invalid.
    #[error("{reason}")]
    Argument {
        /// Fixed human-readable reason the option was rejected.
        reason: &'static str,
    },
    /// Neither or both of `hostKey` and `knownHosts` were supplied.
    #[error(
        "connect requires exactly one of hostKey or knownHosts; no insecure default is available"
    )]
    HostKeyRequired,
    /// The host-key pin blob is invalid or the server host key was not trusted.
    #[error("the server host key was not trusted; refusing to connect")]
    HostKey,
    /// The `knownHosts` text is not a valid known-hosts document.
    #[error("the knownHosts text is not a valid known-hosts document")]
    KnownHosts,
    /// Not exactly one of `password`, `privateKey`, or `agent: true` was supplied.
    #[error("connect requires exactly one of password, privateKey, or agent")]
    AuthMethod,
    /// The private key or passphrase is not a supported credential.
    #[error("the private key or passphrase is not a supported credential")]
    Credential,
    /// The opted-in SSH agent is unavailable, empty, oversized, or refused to sign.
    #[error("{reason}")]
    Agent {
        /// Fixed human-readable description of the agent failure.
        reason: &'static str,
    },
    /// DNS resolution or TCP connection failed.
    #[error("cannot connect to the SSH server{}", OptSource(.source))]
    Connect {
        /// Underlying OS error when available.
        #[source]
        source: Option<io::Error>,
    },
    /// A documented connect or execution deadline expired.
    #[error("the SSH operation exceeded its deadline")]
    Timeout,
    /// The SSH handshake or algorithm negotiation failed.
    #[error("the SSH handshake failed")]
    Handshake,
    /// User authentication failed.
    #[error("SSH authentication failed")]
    Auth,
    /// The server rejected a session request or the session protocol failed.
    #[error("the SSH session failed")]
    Session,
    /// An input, captured output, or open-connection count exceeded a documented bound.
    #[error("{reason}")]
    Limit {
        /// Fixed human-readable description of the limit exceeded.
        reason: &'static str,
    },
    /// The connection is closed or already unusable.
    #[error("the SSH connection is closed")]
    Closed,
    /// The protected transport reported an I/O failure.
    #[error("the SSH transport failed{}", OptSource(.source))]
    Io {
        /// Underlying OS error when available.
        #[source]
        source: Option<io::Error>,
    },
    /// An unexpected host-side failure occurred.
    #[error("the SSH module encountered an internal error")]
    Internal,
}

impl SshError {
    /// Returns the stable [`SshErrorCode`] for this error.
    #[must_use]
    pub const fn code(&self) -> SshErrorCode {
        match self {
            Self::Argument { .. } => SshErrorCode::Argument,
            Self::HostKeyRequired => SshErrorCode::HostKeyRequired,
            Self::HostKey => SshErrorCode::HostKey,
            Self::KnownHosts => SshErrorCode::KnownHosts,
            Self::AuthMethod => SshErrorCode::AuthMethod,
            Self::Credential => SshErrorCode::Credential,
            Self::Agent { .. } => SshErrorCode::Agent,
            Self::Connect { .. } => SshErrorCode::Connect,
            Self::Timeout => SshErrorCode::Timeout,
            Self::Handshake => SshErrorCode::Handshake,
            Self::Auth => SshErrorCode::Auth,
            Self::Session => SshErrorCode::Session,
            Self::Limit { .. } => SshErrorCode::Limit,
            Self::Closed => SshErrorCode::Closed,
            Self::Io { .. } => SshErrorCode::Io,
            Self::Internal => SshErrorCode::Internal,
        }
    }

    /// Constructs a [`SshErrorCode::Argument`] error with a fixed reason string.
    #[must_use]
    pub const fn argument(reason: &'static str) -> Self {
        Self::Argument { reason }
    }

    /// Constructs a [`SshErrorCode::Internal`] error.
    #[must_use]
    pub const fn internal() -> Self {
        Self::Internal
    }

    /// Constructs a [`SshErrorCode::Limit`] error with a fixed reason string.
    #[must_use]
    pub const fn limit(reason: &'static str) -> Self {
        Self::Limit { reason }
    }

    /// Constructs a [`SshErrorCode::Argument`] error with the default fixed message.
    pub(crate) const fn default_argument() -> Self {
        Self::Argument {
            reason: SshErrorCode::Argument.default_message(),
        }
    }

    /// Constructs a [`SshErrorCode::Limit`] error with the default fixed message.
    pub(crate) const fn default_limit() -> Self {
        Self::Limit {
            reason: SshErrorCode::Limit.default_message(),
        }
    }

    /// Constructs a [`SshErrorCode::Agent`] error with a fixed reason string.
    pub(crate) const fn agent(reason: &'static str) -> Self {
        Self::Agent { reason }
    }

    /// Constructs a [`SshErrorCode::Connect`] error wrapping an [`io::Error`].
    pub(crate) fn connect(source: io::Error) -> Self {
        Self::Connect {
            source: Some(source),
        }
    }

    /// Constructs a [`SshErrorCode::Io`] error wrapping an [`io::Error`].
    pub(crate) fn io(source: io::Error) -> Self {
        Self::Io {
            source: Some(source),
        }
    }
}
