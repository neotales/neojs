//! Synchronous SSH command execution for `neo:ssh`.
//!
//! The JavaScript module object is installed by [`crate::builtins`] and is
//! exposed as `Neo.getBuiltInModule("neo:ssh")` and
//! `import { connect } from "neo:ssh"`. This file registers the private
//! host ops it calls and implements the small synchronous connection API behind
//! them.
//!
//! The SSH protocol, key exchange, authentication, transport protection,
//! session channels, and host-key verification are provided by the
//! MIT-licensed `neotales-ssh` crate. This module does not reimplement them and
//! does not modify the crate. It selects a deliberately narrow slice:
//!
//! * exactly one credential source: one password, one in-memory private key, or
//!   an explicitly opted-in Unix SSH agent. An agent is never implicit, never
//!   forwarded, and is not touched before the host key is trusted. There is no
//!   default identity path and no keyboard-interactive prompt;
//! * one explicit SSH-format Ed25519 host-key pin or an explicit `known_hosts`
//!   document, never a default `~/.ssh/known_hosts` lookup and never a mode
//!   that accepts an unknown host key;
//! * one `exec` request at a time, with stderr discarded, on a connection that
//!   is driven only by blocking calls on the isolate's thread.
//!
//! The crate is used unmodified from crates.io at its pinned alpha version; no
//! fork is needed. This module drives `ClientConfig` and `ClientSessions`
//! directly rather than the convenience builder because the builder dials a
//! socket without a connect timeout and can read a known-hosts path from disk.
//! The lower-level public API exposes the mandatory trust callback, the
//! session events, and the window credit needed for a bounded, deadline-aware
//! command without adding file access or a second protocol implementation.
//!
//! # Trust is mandatory
//!
//! `connect` requires exactly one of `hostKey` (a `Uint8Array` holding the
//! complete `string "ssh-ed25519" || string key` blob) or `knownHosts` (the
//! text of a known-hosts document). Supplying neither is
//! `NEO_SSH_HOST_KEY_REQUIRED`; supplying both is also refused because the two
//! policies are not merged. There is no insecure default, no "accept first
//! use", and no fallback to an existing host-key file. The host key is checked
//! by `neotales-ssh` after the server's signature over the exchange hash has
//! been verified, before NEWKEYS, so a mismatched or unknown key aborts the
//! connection rather than running a command.
//!
//! A pin is Ed25519-only in this slice. An explicit known-hosts document can
//! trust the host-key algorithms that `neotales-ssh` verifies, including
//! certificates when the document supplies a certificate authority. A host key
//! that does not match, including a key that changed since it was recorded, is
//! `NEO_SSH_HOST_KEY` and no session request is sent.
//!
//! # Credentials
//!
//! Exactly one of `password`, `privateKey`, or `agent: true` is required. A
//! password is moved into `ClientAuthMethod::Password`, whose value is wrapped
//! in `zeroize` by the SSH crate and wiped when the method is dropped;
//! `optional_secret_string` and `optional_secret_bytes` also keep the parsed
//! Rust copies in `Zeroizing`, so every validation early return wipes them.
//! `privateKey` is an in-memory `Uint8Array`: unencrypted PKCS#8 DER, a
//! `-----BEGIN PRIVATE KEY-----` PEM block, a binary `openssh-key-v1`
//! container, or an `-----BEGIN OPENSSH PRIVATE KEY-----` PEM block. A
//! `passphrase` string is accepted only with `privateKey` and is wiped by the
//! SSH crate's OpenSSH parser; a passphrase supplied for a PKCS#8 key is
//! refused because that format cannot use one here. Encrypted PKCS#8 is not
//! supported.
//!
//! `agent: true` is an explicit Unix-only opt-in. Only then is
//! `SSH_AUTH_SOCK` read, and only after the server host key has been verified.
//! At most [`MAX_AGENT_IDENTITIES`] identities are tried, with at most two
//! authentication requests per identity, so user authentication is bounded to
//! 32 request attempts.
//!
//! The agent timeout is *not* a hard wall clock. `connect_unix` first calls
//! `UnixStream::connect`, which may block and is not covered by
//! `connectTimeoutMs`; only after the socket is connected does the crate set its
//! 30-second read/write timeout. That 30-second value is an inactivity timeout:
//! a peer that keeps trickling bytes can prolong one request beyond 30 seconds.
//! It applies to identity enumeration and to every signing request. The
//! `connectTimeoutMs` deadline is checked before and after the blocking SSH
//! handshake and authentication phases, not inside an agent connect,
//! enumeration, or signing call. Windows named pipes are rejected with a fixed
//! `NEO_SSH_AGENT` error instead of being used with unbounded blocking I/O.
//! There is no agent forwarding, no implicit agent use, and no agent client
//! surface.
//!
//! In password and private-key modes this module never reads a file, an
//! environment variable, an SSH agent, a terminal, or a configuration file.
//! Key material is copied into `Zeroizing` buffers on the Rust side and wiped
//! on drop. The backend signer retains derived key material for the duration of
//! the connection; this module does not claim to wipe the backend's internals.
//! The JavaScript heap still holds the string or typed array the caller passed,
//! and V8 does not promise to erase it; no zeroization claim is made for
//! JavaScript memory. No password, passphrase, or key byte is ever included in
//! an error message.
//!
//! # Blocking, deadlines, and bounds
//!
//! Every function is synchronous and runs on the thread that owns the isolate.
//! `connect` performs DNS resolution, a TCP connect, the SSH handshake, and
//! authentication; `exec` writes the request and reads until the channel
//! closes, collecting stdout even when the exit status arrives first. Both
//! block the isolate. Neither
//! returns a `Promise`, takes a callback, or wraps blocking work in an
//! already-settled `Promise` to look asynchronous. A later asynchronous surface
//! will be additive, `Promise`-based, and will do its work on a real
//! asynchronous path.
//!
//! `connectTimeoutMs` bounds each TCP connect attempt, starting after DNS
//! resolution. During the handshake and authentication it is installed as the
//! socket read/write inactivity timeout.
//! The SSH crate's handshake is one blocking call with no incremental deadline,
//! so for that phase it is an inactivity bound, not a hard wall-clock limit: a
//! peer that stalls for the whole budget fails with `NEO_SSH_TIMEOUT`, while a
//! peer that keeps trickling bytes can extend the phase. DNS name resolution is
//! performed by the operating system and is *not* included at all; an IP
//! literal avoids it.
//!
//! `execTimeoutMs` is the budget for one `exec`, including opening the channel,
//! receiving the output, the exit status, and closing the channel. Before each
//! session event the elapsed time is checked and the socket read/write timeouts
//! are set to the time remaining, so a peer that stalls fails with
//! `NEO_SSH_TIMEOUT` instead of hanging forever. One dispatched event performs a
//! bounded amount of socket work with that remaining timeout, so the overshoot
//! is bounded by one event rather than by an unbounded number of packets.
//!
//! Command bytes are limited to [`MAX_COMMAND_BYTES`], captured stdout to
//! [`MAX_OUTPUT_BYTES`] across the whole channel lifetime (including data
//! after the exit status), `privateKey` to
//! [`neotales_ssh::private_key::MAX_PRIVATE_KEY_DER`] (and PEM input to the
//! same bound), `knownHosts` text to 1 MiB, the host name to 1024 bytes, and
//! the user name to 256 bytes, and an opted-in agent to
//! [`MAX_AGENT_IDENTITIES`] identities. A refused size is `NEO_SSH_LIMIT` and is
//! reported before the buffer is allocated. These are neojs policy limits, not
//! SSH protocol limits. The Rust allocator, V8, and the JavaScript heap
//! allocate too, so recovery from out-of-memory is not guaranteed.
//!
//! # Resource lifetime
//!
//! A `Connection` owns one TCP stream and one authenticated transport. It
//! becomes unusable when `close()` is called, when the isolate is dropped, or
//! when an `exec` fails after the connection has been borrowed. An argument
//! refusal, including a command over [`MAX_COMMAND_BYTES`], happens before the
//! connection is borrowed and leaves it usable; any later failure drops the
//! connection and closes the socket. On success the session channel is closed
//! before `exec` returns, so the same connection can run another command. Dropping the connection closes
//! the socket; it does not send a graceful SSH disconnect message. No
//! background thread, reader, or keepalive is started.
//!
//! # Scope
//!
//! This slice implements `connect`, `exec`, and `close` only. It is not a
//! general SSH client, not SCP, not SFTP, not a shell or PTY, not port
//! forwarding, and not an agent client or agent-forwarding surface. Agent
//! signing is only the explicit opt-in authentication path described above. It
//! does not read `ssh_config`, does not start a process, and does not provide
//! host-key discovery or first-use acceptance. It is not a permission,
//! sandbox, or isolation boundary: `Deno.core` remains reachable, and any code
//! in the realm can call the same host ops. The module adds network access, the
//! credentials the caller supplies, and, only when opted in, the
//! `SSH_AUTH_SOCK` lookup; it adds no filesystem, process, or stdin authority.
//!
//! # Errors
//!
//! A wrong JavaScript type or an unsupported argument count throws `TypeError`
//! before any host op runs. A number outside its documented range throws
//! `RangeError`. Every other failure is an ordinary `Error` with
//! `name = "SshError"` and one stable own `code`:
//!
//! * `NEO_SSH_ARGUMENT` — an option value is structurally invalid.
//! * `NEO_SSH_HOST_KEY_REQUIRED` — not exactly one of `hostKey` / `knownHosts`.
//! * `NEO_SSH_HOST_KEY` — the pin blob is invalid or the host key is not
//!   trusted.
//! * `NEO_SSH_KNOWN_HOSTS` — the known-hosts text is malformed.
//! * `NEO_SSH_AUTH_METHOD` — not exactly one of `password` / `privateKey` /
//!   `agent: true`.
//! * `NEO_SSH_CREDENTIAL` — the private key or passphrase is unsupported.
//! * `NEO_SSH_AGENT` — the opted-in agent is unavailable, empty, oversized, or
//!   refused to sign.
//! * `NEO_SSH_CONNECT` — DNS or TCP connection failed.
//! * `NEO_SSH_TIMEOUT` — a documented deadline expired.
//! * `NEO_SSH_HANDSHAKE` — the SSH handshake or algorithm negotiation failed.
//! * `NEO_SSH_AUTH` — user authentication failed.
//! * `NEO_SSH_SESSION` — the server rejected a session request or the session
//!   protocol failed.
//! * `NEO_SSH_LIMIT` — an input or captured output exceeded a documented bound.
//! * `NEO_SSH_CLOSED` — the connection is closed or was already unusable.
//! * `NEO_SSH_IO` — the protected transport reported an I/O failure.
//! * `NEO_SSH_INTERNAL` — an unexpected host-side failure.
//!
//! Messages are fixed text, except that `NEO_SSH_CONNECT` and `NEO_SSH_IO`
//! include the operating-system error. They never include a password, a
//! passphrase, key bytes, command bytes, stdout bytes, or the peer's banner.

use std::collections::BTreeMap;
use std::io;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use deno_core::OpState;
use deno_core::op2;
use deno_core::v8;
use neotales_ssh::ClientAuthMethod;
use neotales_ssh::ClientHandshakeError;
use neotales_ssh::ClientTransport;
use neotales_ssh::Zeroizing;
use neotales_ssh::auth::AuthError;
use neotales_ssh::channel::ChannelError;
use neotales_ssh::client::ClientConfig;
use neotales_ssh::hostkey::Ed25519PublicKey;
use neotales_ssh::hostkey::HostKeyError;
use neotales_ssh::known_hosts::KnownHosts;
use neotales_ssh::private_key;
use neotales_ssh::session::ClientSessions;
use neotales_ssh::session::SessionError;
use neotales_ssh::session::SessionEvent;
use neotales_ssh::transport::TransportError;

/// Largest command accepted by `exec`, in bytes, before the request is sent.
pub(crate) const MAX_COMMAND_BYTES: usize = 64 * 1024;

/// Largest stdout captured from one `exec`, in bytes.
///
/// A command that produces more is `NEO_SSH_LIMIT`; the connection is then
/// closed rather than draining unbounded output. The same value is exported by
/// the JavaScript module, and the boundary test exercises both sides.
pub(crate) const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Default deadline for the TCP connect plus handshake and authentication.
pub(crate) const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 15_000;

/// Default deadline for one `exec`.
pub(crate) const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;

/// Largest accepted `connectTimeoutMs`.
const MAX_CONNECT_TIMEOUT_MS: u64 = 120_000;

/// Largest accepted `execTimeoutMs`.
const MAX_EXEC_TIMEOUT_MS: u64 = 600_000;

/// Largest accepted host name, in bytes.
const MAX_HOST_BYTES: usize = 1024;

/// Largest accepted user name, in bytes.
const MAX_USERNAME_BYTES: usize = 256;

/// Largest accepted password, in bytes.
const MAX_PASSWORD_BYTES: usize = 4096;

/// Largest accepted passphrase, in bytes.
const MAX_PASSPHRASE_BYTES: usize = 1024;

/// Largest accepted `knownHosts` document, in bytes.
const MAX_KNOWN_HOSTS_BYTES: usize = 1024 * 1024;

/// Largest number of agent identities this slice tries.
///
/// The agent enumerates at most this many; more is a fixed `NEO_SSH_AGENT`
/// error rather than an unbounded authentication attempt set.
#[cfg_attr(not(unix), allow(dead_code))]
const MAX_AGENT_IDENTITIES: usize = 16;

/// Largest accepted host-key pin blob, in bytes.
///
/// An Ed25519 blob is 51 bytes. The allowance covers the complete SSH string
/// encoding without accepting an arbitrarily large buffer.
const MAX_HOST_KEY_BYTES: usize = 1024;

/// A documented failure, carried to the JavaScript wrapper as data.
#[derive(Debug)]
struct SshError {
    code: ErrorCode,
    message: String,
}

impl SshError {
    fn new(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.message().to_string(),
        }
    }

    fn connect(source: &io::Error) -> Self {
        Self {
            code: ErrorCode::Connect,
            message: format!("cannot connect to the SSH server: {source}"),
        }
    }

    fn io(source: &io::Error) -> Self {
        Self {
            code: ErrorCode::Io,
            message: format!("the SSH transport failed: {source}"),
        }
    }

    fn agent(message: &'static str) -> Self {
        Self {
            code: ErrorCode::Agent,
            message: message.to_string(),
        }
    }
}

/// Stable failure kinds produced by the host side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorCode {
    /// The value is not an accepted JavaScript type, reported as `TypeError`.
    Type,
    /// A number is outside the accepted range, reported as `RangeError`.
    Range,
    Argument,
    HostKeyRequired,
    HostKey,
    KnownHosts,
    AuthMethod,
    Credential,
    Agent,
    Connect,
    Timeout,
    Handshake,
    Auth,
    Session,
    Limit,
    Closed,
    Io,
    Internal,
}

impl ErrorCode {
    fn js_class(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Range => "RangeError",
            _ => "SshError",
        }
    }

    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Type | Self::Range => None,
            Self::Argument => Some("NEO_SSH_ARGUMENT"),
            Self::HostKeyRequired => Some("NEO_SSH_HOST_KEY_REQUIRED"),
            Self::HostKey => Some("NEO_SSH_HOST_KEY"),
            Self::KnownHosts => Some("NEO_SSH_KNOWN_HOSTS"),
            Self::AuthMethod => Some("NEO_SSH_AUTH_METHOD"),
            Self::Credential => Some("NEO_SSH_CREDENTIAL"),
            Self::Agent => Some("NEO_SSH_AGENT"),
            Self::Connect => Some("NEO_SSH_CONNECT"),
            Self::Timeout => Some("NEO_SSH_TIMEOUT"),
            Self::Handshake => Some("NEO_SSH_HANDSHAKE"),
            Self::Auth => Some("NEO_SSH_AUTH"),
            Self::Session => Some("NEO_SSH_SESSION"),
            Self::Limit => Some("NEO_SSH_LIMIT"),
            Self::Closed => Some("NEO_SSH_CLOSED"),
            Self::Io => Some("NEO_SSH_IO"),
            Self::Internal => Some("NEO_SSH_INTERNAL"),
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Type => "an SSH option has the wrong type",
            Self::Range => "an SSH option is outside the documented range",
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

fn fixed(code: ErrorCode) -> SshError {
    SshError::new(code)
}

fn type_error() -> SshError {
    fixed(ErrorCode::Type)
}

fn range_error() -> SshError {
    fixed(ErrorCode::Range)
}

fn timeout_error() -> SshError {
    fixed(ErrorCode::Timeout)
}

fn closed_error() -> SshError {
    fixed(ErrorCode::Closed)
}

fn limit_error() -> SshError {
    fixed(ErrorCode::Limit)
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

/// One authenticated connection and the stream used to adjust its deadlines.
///
/// The stream is a duplicate of the descriptor the transport owns. Socket
/// timeouts are properties of the underlying socket, so setting one through
/// either handle changes the same socket for the next blocking operation.
struct Connection {
    /// `None` once the connection has failed and been taken out of the state.
    sessions: Option<ClientSessions<ClientTransport<TcpStream>>>,
    stream: TcpStream,
    exec_timeout: Duration,
}

impl Connection {
    /// Run one command and close its session channel before returning.
    ///
    /// Any error removes the caller's ability to reuse the connection: the op
    /// removes it from the state, which drops the transport and the socket.
    fn exec(&mut self, command: &[u8]) -> Result<(Vec<u8>, u32), SshError> {
        let deadline = Instant::now() + self.exec_timeout;
        let Self {
            sessions,
            stream,
            exec_timeout: _,
        } = self;
        let sessions = sessions.as_mut().ok_or_else(closed_error)?;

        let session = open_session(sessions, stream, deadline)?;
        set_socket_deadline(stream, deadline)?;
        sessions
            .request_exec(session, command, true)
            .map_err(|error| map_session_error(error, deadline))?;

        let mut output = Vec::new();
        let mut status = None;
        let mut sent_close = false;
        loop {
            set_socket_deadline(stream, deadline)?;
            match sessions
                .dispatch_one()
                .map_err(|error| map_session_error(error, deadline))?
            {
                SessionEvent::Opened { .. }
                | SessionEvent::RequestSucceeded { .. }
                | SessionEvent::WindowAdjusted { .. } => {}
                SessionEvent::Data { data, .. } => {
                    // Data may arrive after the exit status. Keep collecting
                    // until the channel closes and enforce the bound across the
                    // whole response, not just the portion before status.
                    if data.len() > MAX_OUTPUT_BYTES.saturating_sub(output.len()) {
                        return Err(limit_error());
                    }
                    return_credit(sessions, session, data.len(), deadline)?;
                    output.extend_from_slice(&data);
                }
                SessionEvent::ExtendedData { data, .. } => {
                    // stderr is discarded, but its channel credit is returned.
                    return_credit(sessions, session, data.len(), deadline)?;
                }
                SessionEvent::ExitStatus {
                    status: received, ..
                } => {
                    if status.replace(received).is_some() {
                        return Err(fixed(ErrorCode::Session));
                    }
                }
                SessionEvent::Eof { .. } => {
                    // The peer will send no more data. Half-close now so a
                    // server that waits for our CLOSE can finish.
                    if !sent_close {
                        set_socket_deadline(stream, deadline)?;
                        sessions
                            .send_close(session)
                            .map_err(|error| map_session_error(error, deadline))?;
                        sent_close = true;
                    }
                }
                SessionEvent::Close { .. } => {
                    // Complete the half-close if the peer closed before EOF.
                    // A failed reply means the protected transport is broken;
                    // propagate it so the caller removes the connection.
                    set_socket_deadline(stream, deadline)?;
                    finish_peer_close(sessions, session, deadline, sent_close)?;
                    break;
                }
                SessionEvent::RequestFailed { .. } | SessionEvent::OpenRejected { .. } => {
                    return Err(fixed(ErrorCode::Session));
                }
                _ => return Err(fixed(ErrorCode::Session)),
            }
        }

        let status = status.ok_or_else(|| fixed(ErrorCode::Session))?;
        Ok((output, status))
    }
}

/// Open a session and wait until the server confirms it.
///
/// A channel request cannot be sent until the peer's channel-open confirmation
/// has been dispatched, because the remote channel identifier is not known
/// before then.
fn open_session<T: neotales_ssh::channel::ChannelTransport>(
    sessions: &mut ClientSessions<T>,
    stream: &TcpStream,
    deadline: Instant,
) -> Result<u32, SshError> {
    let session = sessions
        .open_session()
        .map_err(|error| map_session_error(error, deadline))?;
    set_socket_deadline(stream, deadline)?;
    match sessions
        .dispatch_one()
        .map_err(|error| map_session_error(error, deadline))?
    {
        SessionEvent::Opened { session: opened } if opened == session => Ok(session),
        _ => Err(fixed(ErrorCode::Session)),
    }
}

/// Send the CLOSE reply when the peer closed before our EOF/CLOSE.
///
/// If this side already sent CLOSE in response to EOF, the peer's CLOSE is the
/// final one and no reply is needed. Otherwise the reply is mandatory: a write
/// failure is a broken transport, not a harmless late close, and is returned so
/// the caller can drop the connection.
fn finish_peer_close<T: neotales_ssh::channel::ChannelTransport>(
    sessions: &mut ClientSessions<T>,
    session: u32,
    deadline: Instant,
    sent_close: bool,
) -> Result<(), SshError> {
    if sent_close {
        return Ok(());
    }
    #[cfg(feature = "test-support")]
    if FAIL_NEXT_PEER_CLOSE_REPLY.swap(false, std::sync::atomic::Ordering::SeqCst) {
        // Injected at the transport boundary for the caller-facing regression.
        // The feature is off in normal builds.
        return Err(fixed(ErrorCode::Io));
    }
    sessions
        .send_close(session)
        .map_err(|error| map_session_error(error, deadline))
}

/// Test-only switch for the next peer-close reply. See `lib.rs`
/// `test_support`.
#[cfg(feature = "test-support")]
pub(crate) static FAIL_NEXT_PEER_CLOSE_REPLY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn return_credit<T: neotales_ssh::channel::ChannelTransport>(
    sessions: &mut ClientSessions<T>,
    session: u32,
    bytes: usize,
    deadline: Instant,
) -> Result<(), SshError> {
    if bytes == 0 {
        return Ok(());
    }
    let credit = u32::try_from(bytes).map_err(|_| limit_error())?;
    sessions
        .increase_local_window(session, credit)
        .map_err(|error| map_session_error(error, deadline))?;
    Ok(())
}

fn set_socket_deadline(stream: &TcpStream, deadline: Instant) -> Result<(), SshError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(timeout_error());
    }
    // Rust rejects a zero timeout. One millisecond keeps the socket blocking
    // while still failing almost immediately at the deadline.
    let remaining = (deadline - now).max(Duration::from_millis(1));
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| SshError::io(&error))?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| SshError::io(&error))?;
    Ok(())
}

fn map_session_error(error: SessionError, deadline: Instant) -> SshError {
    match error {
        SessionError::Channel(ChannelError::Transport) => {
            if Instant::now() >= deadline {
                timeout_error()
            } else {
                fixed(ErrorCode::Io)
            }
        }
        _ => fixed(ErrorCode::Session),
    }
}

fn map_handshake_error(error: ClientHandshakeError) -> SshError {
    match error {
        ClientHandshakeError::HostKey(
            HostKeyError::TrustRejected | HostKeyError::VerifierPanicked,
        ) => fixed(ErrorCode::HostKey),
        ClientHandshakeError::Transport(TransportError::Io(error)) => {
            if is_timeout(&error) {
                timeout_error()
            } else {
                SshError::io(&error)
            }
        }
        ClientHandshakeError::EntropyFailure => fixed(ErrorCode::Internal),
        _ => fixed(ErrorCode::Handshake),
    }
}

/// The two explicit trust choices. There is deliberately no third variant.
enum Trust {
    Pin(Ed25519PublicKey),
    KnownHosts(KnownHosts),
}

enum Auth {
    /// The SSH crate stores this value in a zeroizing wrapper; keeping the
    /// parsed copy in the same wrapper avoids one unwiped intermediate.
    Password(Zeroizing<String>),
    PrivateKey {
        bytes: Zeroizing<Vec<u8>>,
        passphrase: Zeroizing<Vec<u8>>,
    },
    /// Explicit opt-in. The agent is not touched until the host key has been
    /// verified, and no method is created before that point.
    Agent,
}

struct ConnectOptions {
    host: String,
    port: u16,
    username: Vec<u8>,
    auth: Auth,
    trust: Trust,
    connect_timeout: Duration,
    exec_timeout: Duration,
}

/// Validate, connect, authenticate, and register one connection.
fn connect(options: ConnectOptions) -> Result<Connection, SshError> {
    // Build a password or key credential before dialing: a malformed key or a
    // passphrase supplied for a format that cannot use one is a local error and
    // must not open a socket. Agent methods are deliberately deferred until
    // after host-key verification, so a rejected host never accesses the agent.
    let agent_auth = matches!(&options.auth, Auth::Agent);
    let method = match options.auth {
        Auth::Password(password) => Some(ClientAuthMethod::Password(password)),
        Auth::PrivateKey {
            bytes,
            mut passphrase,
        } => Some(import_identity(&bytes, &mut passphrase)?),
        Auth::Agent => None,
    };

    let stream = dial(&options.host, options.port, options.connect_timeout)?;
    let io = stream
        .try_clone()
        .map_err(|error| SshError::connect(&error))?;

    // The handshake and authentication share one deadline. DNS resolution was
    // part of `dial` and is documented as outside this budget.
    let deadline = Instant::now() + options.connect_timeout;
    set_socket_deadline(&io, deadline)?;

    let config = ClientConfig::new();
    let transport = match options.trust {
        Trust::Pin(pin) => config.connect(
            stream,
            move |key| {
                if *key == pin { Ok(()) } else { Err(()) }
            },
        ),
        Trust::KnownHosts(hosts) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| fixed(ErrorCode::Internal))?
                .as_secs();
            config.connect_with_known_hosts(stream, &hosts, &options.host, options.port, now)
        }
    }
    .map_err(map_handshake_error)?;

    if Instant::now() >= deadline {
        return Err(timeout_error());
    }
    set_socket_deadline(&io, deadline)?;

    let mut transport = transport;
    let methods = match method {
        Some(method) => vec![method],
        None => agent_methods()?,
    };
    // Agent connect, enumeration, and signing are outside the connect budget:
    // UnixStream::connect may block, and the backend's 30-second socket
    // timeout is inactivity-based. Re-check and refresh the SSH socket
    // deadline before user authentication.
    if Instant::now() >= deadline {
        return Err(timeout_error());
    }
    set_socket_deadline(&io, deadline)?;
    let attempts = methods.len().saturating_mul(2).max(1);
    neotales_ssh::authenticate_client(&mut transport, &options.username, &methods, attempts)
        .map_err(|error| map_auth_error(error, deadline, agent_auth))?;

    if Instant::now() >= deadline {
        return Err(timeout_error());
    }

    Ok(Connection {
        sessions: Some(ClientSessions::new(transport)),
        stream: io,
        exec_timeout: options.exec_timeout,
    })
}

fn map_auth_error(error: AuthError, deadline: Instant, agent_auth: bool) -> SshError {
    match error {
        AuthError::Transport => {
            if Instant::now() >= deadline {
                timeout_error()
            } else {
                fixed(ErrorCode::Auth)
            }
        }
        AuthError::AgentUnavailable | AuthError::InvalidProof if agent_auth => {
            SshError::agent("the SSH agent did not produce a usable signature")
        }
        _ => fixed(ErrorCode::Auth),
    }
}

/// Enumerate an explicitly opted-in Unix agent and build its methods.
///
/// This is called only after the server host key has been verified. It reads
/// `SSH_AUTH_SOCK` only here, never in another authentication mode.
/// `UnixStream::connect` may block and is not bounded by `connectTimeoutMs`.
/// After it returns, the agent crate applies a 30-second read/write timeout;
/// that is an inactivity timeout, not a hard wall clock, so a trickling peer
/// can prolong identity enumeration or a signing request. Windows named pipes
/// are rejected instead of being used with unbounded blocking I/O.
#[cfg(unix)]
fn agent_methods() -> Result<Vec<ClientAuthMethod>, SshError> {
    use neotales_ssh::AgentKeySigner;
    use neotales_ssh::agent::AgentClient;

    let path = std::env::var_os("SSH_AUTH_SOCK")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SshError::agent("SSH_AUTH_SOCK is not set"))?;
    let mut client = neotales_ssh::agent::connect_unix(path)
        .map_err(|_| SshError::agent("the SSH agent socket is unavailable"))?;
    let identities = client
        .identities()
        .map_err(|_| SshError::agent("the SSH agent did not list identities"))?;
    if identities.is_empty() {
        return Err(SshError::agent("the SSH agent has no identities"));
    }
    if identities.len() > MAX_AGENT_IDENTITIES {
        return Err(SshError::agent(
            "the SSH agent has more identities than this slice tries",
        ));
    }
    let stream = client.into_inner();
    let mut methods = Vec::new();
    for identity in identities {
        let clone = stream
            .try_clone()
            .map_err(|_| SshError::agent("the SSH agent socket could not be duplicated"))?;
        let client = AgentClient::new(clone);
        if let Ok(signer) = AgentKeySigner::new(client, identity.key_blob()) {
            methods.push(ClientAuthMethod::agent(signer));
        }
    }
    if methods.is_empty() {
        return Err(SshError::agent("the SSH agent has no supported identities"));
    }
    Ok(methods)
}

#[cfg(not(unix))]
fn agent_methods() -> Result<Vec<ClientAuthMethod>, SshError> {
    Err(SshError::agent(
        "the SSH agent is unsupported on this platform",
    ))
}

/// Resolve and dial the host, applying the connect deadline to each address.
///
/// `ToSocketAddrs` performs the platform's name resolution and has no portable
/// timeout. That is why the module documentation excludes DNS from the bound.
fn dial(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, SshError> {
    // Resolve first: the documented bound starts after name resolution and
    // covers only the TCP connect attempts. `to_socket_addrs` is a blocking
    // platform call and has no portable timeout.
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| SshError::connect(&error))?
        .collect::<Vec<_>>();
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    for address in addresses {
        let now = Instant::now();
        if now >= deadline {
            return Err(timeout_error());
        }
        let remaining = deadline - now;
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if is_timeout(&error) {
                    return Err(timeout_error());
                }
                last_error = Some(error);
            }
        }
    }
    Err(match last_error {
        Some(error) => SshError::connect(&error),
        None => fixed(ErrorCode::Connect),
    })
}

/// Import one in-memory private key without reading a file.
///
/// PKCS#8 DER and binary OpenSSH containers are passed to the SSH crate's
/// importers. PEM text is decoded first, with a bound, because the crate's
/// importers take the binary form. A passphrase is only used by the OpenSSH
/// importer; encrypted PKCS#8 is not supported.
fn import_identity(bytes: &[u8], passphrase: &mut [u8]) -> Result<ClientAuthMethod, SshError> {
    const OPENSSH_PEM: &[u8] = b"-----BEGIN OPENSSH PRIVATE KEY-----";
    const PKCS8_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----";
    const OPENSSH_BINARY: &[u8] = b"openssh-key-v1\0";

    if bytes.starts_with(OPENSSH_PEM) {
        let decoded = decode_pem(bytes, "OPENSSH PRIVATE KEY")?;
        let signer = private_key::import_openssh(&decoded, passphrase)
            .map_err(|_| fixed(ErrorCode::Credential))?;
        return Ok(ClientAuthMethod::public_key(signer));
    }
    if bytes.starts_with(PKCS8_PEM) {
        require_no_passphrase(passphrase)?;
        let decoded = decode_pem(bytes, "PRIVATE KEY")?;
        let signer =
            private_key::import_pkcs8_der(&decoded).map_err(|_| fixed(ErrorCode::Credential))?;
        return Ok(ClientAuthMethod::public_key(signer));
    }
    if bytes.starts_with(OPENSSH_BINARY) {
        let signer = private_key::import_openssh(bytes, passphrase)
            .map_err(|_| fixed(ErrorCode::Credential))?;
        return Ok(ClientAuthMethod::public_key(signer));
    }
    require_no_passphrase(passphrase)?;
    let signer = private_key::import_pkcs8_der(bytes).map_err(|_| fixed(ErrorCode::Credential))?;
    Ok(ClientAuthMethod::public_key(signer))
}

/// Refuse a passphrase for a format that cannot use one in this slice.
///
/// Only the OpenSSH container parser takes a passphrase. Silently ignoring a
/// supplied passphrase for PKCS#8 could hide a caller mistake, so it is
/// reported as a credential error instead.
fn require_no_passphrase(passphrase: &[u8]) -> Result<(), SshError> {
    if passphrase.is_empty() {
        Ok(())
    } else {
        Err(fixed(ErrorCode::Credential))
    }
}

/// Decode one PEM block whose label is `label` into DER bytes.
///
/// The input is already bounded by the caller. This routine only removes ASCII
/// whitespace and decodes standard base64; it never interprets the key.
fn decode_pem(bytes: &[u8], label: &str) -> Result<Zeroizing<Vec<u8>>, SshError> {
    let text = std::str::from_utf8(bytes).map_err(|_| fixed(ErrorCode::Credential))?;
    let header = format!("-----BEGIN {label}-----");
    let footer = format!("-----END {label}-----");
    let Some(after_header) = text.strip_prefix(&header) else {
        return Err(fixed(ErrorCode::Credential));
    };
    let Some(body) = after_header.split_once(&footer).map(|(body, _)| body) else {
        return Err(fixed(ErrorCode::Credential));
    };
    let encoded = Zeroizing::new(
        body.chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect::<String>(),
    );
    let decoded = Zeroizing::new(
        neotales_ssh::known_hosts::decode_base64(&encoded)
            .map_err(|_| fixed(ErrorCode::Credential))?,
    );
    if decoded.len() > private_key::MAX_PRIVATE_KEY_DER {
        return Err(limit_error());
    }
    Ok(decoded)
}

#[derive(Default)]
struct SshState {
    next: u32,
    connections: BTreeMap<u32, Connection>,
}

impl SshState {
    fn insert(&mut self, connection: Connection) -> u32 {
        loop {
            self.next = self.next.wrapping_add(1);
            if self.next == 0 {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(slot) =
                self.connections.entry(self.next)
            {
                slot.insert(connection);
                return self.next;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// JavaScript value handling
// ---------------------------------------------------------------------------

fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: SshError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    if let Some(name) = v8::String::new(scope, error.code.js_class()) {
        set(scope, object, "name", name.into());
    }
    if let Some(code) = error.code.as_str()
        && let Some(code) = v8::String::new(scope, code)
    {
        set(scope, object, "code", code.into());
    }
    if let Some(message) = v8::String::new(scope, &error.message) {
        set(scope, object, "message", message.into());
    }
}

fn failure<'s>(scope: &mut v8::PinScope<'s, '_>, error: SshError) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set_failure(scope, object, error);
    object.into()
}

fn failure_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: SshError,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set_failure(scope, object, error);
    set(
        scope,
        object,
        "closed",
        v8::Boolean::new(scope, true).into(),
    );
    object.into()
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        return v8::Uint8Array::new(scope, buffer, 0, 0);
    }
    let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

/// Read an optional string and copy it, bounding it before the copy.
fn optional_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<String>, SshError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return Err(type_error());
    };
    Ok(Some(copy_string(scope, string, max)?))
}

/// Encode one V8 string as UTF-8 without silently replacing a lone surrogate.
///
/// `to_rust_string_lossy` substitutes U+FFFD for an unpaired surrogate, which
/// would quietly change a password or an exact command. V8's UTF-8 writer is
/// asked for the original code units instead; the result is decoded as strict
/// UTF-8, so an unpaired surrogate is an argument error rather than a different
/// value.
fn copy_string(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    max: usize,
) -> Result<String, SshError> {
    let length = string.utf8_length(scope);
    if length > max {
        return Err(limit_error());
    }
    // The extra room covers any difference between V8's length count and the
    // exact encoding, so a write is never silently truncated.
    let capacity = length.checked_add(4).ok_or_else(limit_error)?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| limit_error())?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    if written > max {
        return Err(limit_error());
    }
    buffer.truncate(written);
    String::from_utf8(buffer).map_err(|_| fixed(ErrorCode::Argument))
}

/// Copy an optional secret string into a zeroizing buffer.
///
/// The intermediate UTF-8 bytes and the returned string are wiped on drop.
/// Every validation return after this call therefore wipes the parsed copy.
fn optional_secret_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<Zeroizing<String>>, SshError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return Err(type_error());
    };
    let length = string.utf8_length(scope);
    if length > max {
        return Err(limit_error());
    }
    let capacity = length.checked_add(4).ok_or_else(limit_error)?;
    let mut buffer = Zeroizing::new(Vec::new());
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| limit_error())?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    if written > max {
        return Err(limit_error());
    }
    buffer.truncate(written);
    let text = std::str::from_utf8(&buffer).map_err(|_| fixed(ErrorCode::Argument))?;
    // `to_owned` allocates a second buffer; the first is wiped when `buffer`
    // drops at the end of this function, and the second is wiped by its
    // `Zeroizing` wrapper.
    Ok(Some(Zeroizing::new(text.to_owned())))
}

/// Copy an optional secret `Uint8Array` into a zeroizing buffer.
fn optional_secret_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, SshError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(type_error());
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(fixed(ErrorCode::Argument));
    };
    if buffer.was_detached() {
        return Err(fixed(ErrorCode::Argument));
    }
    let length = view.byte_length();
    if length > max {
        return Err(limit_error());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    bytes.try_reserve_exact(length).map_err(|_| limit_error())?;
    bytes.resize(length, 0);
    if view.copy_contents(&mut bytes) != length {
        return Err(fixed(ErrorCode::Argument));
    }
    Ok(Some(bytes))
}

/// Copy an optional `Uint8Array`, bounding it before the copy.
fn optional_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<Vec<u8>>, SshError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(type_error());
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(fixed(ErrorCode::Argument));
    };
    if buffer.was_detached() {
        return Err(fixed(ErrorCode::Argument));
    }
    let length = view.byte_length();
    if length > max {
        return Err(limit_error());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| limit_error())?;
    bytes.resize(length, 0);
    if view.copy_contents(&mut bytes) != length {
        return Err(fixed(ErrorCode::Argument));
    }
    Ok(Some(bytes))
}

fn optional_number(
    value: v8::Local<'_, v8::Value>,
    default: u64,
    max: u64,
) -> Result<u64, SshError> {
    if value.is_undefined() {
        return Ok(default);
    }
    let Ok(number) = v8::Local::<v8::Number>::try_from(value) else {
        return Err(type_error());
    };
    let value = number.value();
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(range_error());
    }
    if value < 1.0 || value > max as f64 {
        return Err(range_error());
    }
    Ok(value as u64)
}

fn optional_port(value: v8::Local<'_, v8::Value>) -> Result<u16, SshError> {
    if value.is_undefined() {
        return Ok(22);
    }
    let Ok(number) = v8::Local::<v8::Number>::try_from(value) else {
        return Err(type_error());
    };
    let value = number.value();
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(range_error());
    }
    if value < 1.0 || value > 65535.0 {
        return Err(range_error());
    }
    Ok(value as u16)
}

fn required_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<String, SshError> {
    optional_string(scope, value, max)?.ok_or_else(type_error)
}

fn validate_host(host: &str) -> Result<(), SshError> {
    if host.is_empty()
        || host.len() > MAX_HOST_BYTES
        || host
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(fixed(ErrorCode::Argument));
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<(), SshError> {
    if username.is_empty()
        || username.len() > MAX_USERNAME_BYTES
        || username.chars().any(char::is_control)
    {
        return Err(fixed(ErrorCode::Argument));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Ops
// ---------------------------------------------------------------------------

#[op2]
#[allow(clippy::too_many_arguments)]
fn op_neo_ssh_connect<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    op_state: &mut OpState,
    host: v8::Local<'s, v8::Value>,
    port: v8::Local<'s, v8::Value>,
    username: v8::Local<'s, v8::Value>,
    password: v8::Local<'s, v8::Value>,
    private_key: v8::Local<'s, v8::Value>,
    agent: v8::Local<'s, v8::Value>,
    passphrase: v8::Local<'s, v8::Value>,
    host_key: v8::Local<'s, v8::Value>,
    known_hosts: v8::Local<'s, v8::Value>,
    connect_timeout_ms: v8::Local<'s, v8::Value>,
    exec_timeout_ms: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let parsed = (|| {
        let host = required_string(scope, host, MAX_HOST_BYTES)?;
        validate_host(&host)?;
        let port = optional_port(port)?;
        let username = required_string(scope, username, MAX_USERNAME_BYTES)?;
        validate_username(&username)?;

        let password = optional_secret_string(scope, password, MAX_PASSWORD_BYTES)?;
        let private_key =
            optional_secret_bytes(scope, private_key, private_key::MAX_PRIVATE_KEY_DER)?;
        let agent = if agent.is_undefined() {
            false
        } else {
            let Ok(flag) = v8::Local::<v8::Boolean>::try_from(agent) else {
                return Err(type_error());
            };
            flag.is_true()
        };
        let passphrase = optional_secret_string(scope, passphrase, MAX_PASSPHRASE_BYTES)?;

        let auth_count = usize::from(password.is_some())
            + usize::from(private_key.is_some())
            + usize::from(agent);
        if auth_count != 1 {
            return Err(fixed(ErrorCode::AuthMethod));
        }
        if agent && passphrase.is_some() {
            return Err(fixed(ErrorCode::Argument));
        }
        let auth = if agent {
            Auth::Agent
        } else {
            match (password, private_key) {
                (Some(password), None) => {
                    if passphrase.is_some() {
                        return Err(fixed(ErrorCode::Argument));
                    }
                    Auth::Password(password)
                }
                (None, Some(bytes)) => {
                    if bytes.is_empty() {
                        return Err(fixed(ErrorCode::Credential));
                    }
                    let passphrase =
                        passphrase.map_or_else(Vec::new, |text| text.as_bytes().to_vec());
                    Auth::PrivateKey {
                        bytes,
                        passphrase: Zeroizing::new(passphrase),
                    }
                }
                _ => return Err(fixed(ErrorCode::AuthMethod)),
            }
        };

        let host_key = optional_bytes(scope, host_key, MAX_HOST_KEY_BYTES)?;
        let known_hosts = optional_string(scope, known_hosts, MAX_KNOWN_HOSTS_BYTES)?;
        let trust = match (host_key, known_hosts) {
            (Some(blob), None) => {
                let pin =
                    Ed25519PublicKey::from_blob(&blob).map_err(|_| fixed(ErrorCode::HostKey))?;
                Trust::Pin(pin)
            }
            (None, Some(text)) => {
                let hosts = KnownHosts::parse(&text).map_err(|_| fixed(ErrorCode::KnownHosts))?;
                Trust::KnownHosts(hosts)
            }
            _ => return Err(fixed(ErrorCode::HostKeyRequired)),
        };

        let connect_timeout = Duration::from_millis(optional_number(
            connect_timeout_ms,
            DEFAULT_CONNECT_TIMEOUT_MS,
            MAX_CONNECT_TIMEOUT_MS,
        )?);
        let exec_timeout = Duration::from_millis(optional_number(
            exec_timeout_ms,
            DEFAULT_EXEC_TIMEOUT_MS,
            MAX_EXEC_TIMEOUT_MS,
        )?);

        Ok(ConnectOptions {
            host,
            port,
            username: username.into_bytes(),
            auth,
            trust,
            connect_timeout,
            exec_timeout,
        })
    })();

    let connection = match parsed {
        Ok(options) => match connect(options) {
            Ok(connection) => connection,
            Err(error) => return failure(scope, error),
        },
        Err(error) => return failure(scope, error),
    };

    let Some(state) = op_state.try_borrow_mut::<SshState>() else {
        return failure(scope, fixed(ErrorCode::Internal));
    };
    let id = state.insert(connection);
    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    set(
        scope,
        object,
        "id",
        v8::Number::new(scope, f64::from(id)).into(),
    );
    object.into()
}

#[op2]
fn op_neo_ssh_exec<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    op_state: &mut OpState,
    #[smi] id: u32,
    command: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let command = match read_command(scope, command) {
        Ok(command) => command,
        Err(error) => return failure(scope, error),
    };

    let Some(state) = op_state.try_borrow_mut::<SshState>() else {
        return failure_closed(scope, fixed(ErrorCode::Internal));
    };
    let result = match state.connections.get_mut(&id) {
        Some(connection) => connection.exec(&command),
        None => return failure_closed(scope, closed_error()),
    };

    match result {
        Ok((stdout, status)) => {
            let object = response(scope);
            let Some(stdout) = bytes_to_v8(scope, stdout) else {
                state.connections.remove(&id);
                return failure_closed(scope, limit_error());
            };
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(scope, object, "stdout", stdout.into());
            set(
                scope,
                object,
                "status",
                v8::Number::new(scope, f64::from(status)).into(),
            );
            object.into()
        }
        Err(error) => {
            // Any failure after the connection was borrowed abandons it. The
            // session may be mid-packet, so reusing it could mix responses.
            state.connections.remove(&id);
            failure_closed(scope, error)
        }
    }
}

fn read_command(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Vec<u8>, SshError> {
    if let Ok(string) = v8::Local::<v8::String>::try_from(value) {
        return Ok(copy_string(scope, string, MAX_COMMAND_BYTES)?.into_bytes());
    }
    optional_bytes(scope, value, MAX_COMMAND_BYTES)?.ok_or_else(type_error)
}

#[op2]
fn op_neo_ssh_close<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    op_state: &mut OpState,
    #[smi] id: u32,
) -> v8::Local<'s, v8::Value> {
    let Some(state) = op_state.try_borrow_mut::<SshState>() else {
        return failure(scope, fixed(ErrorCode::Internal));
    };
    let removed = state.connections.remove(&id).is_some();
    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    set(
        scope,
        object,
        "value",
        v8::Boolean::new(scope, removed).into(),
    );
    object.into()
}

deno_core::extension!(
    neojs_ssh,
    ops = [op_neo_ssh_connect, op_neo_ssh_exec, op_neo_ssh_close],
    state = |state| state.put(SshState::default()),
    docs = "Synchronous SSH command execution behind `Neo.getBuiltInModule(\"neo:ssh\")`."
);

#[cfg(test)]
mod tests {
    use super::*;
    use neotales_ssh::Writer;
    use neotales_ssh::channel::ChannelTransport;
    use std::collections::VecDeque;

    /// A deterministic transport that fails the CLOSE reply write.
    struct FailingCloseTransport {
        reads: VecDeque<Vec<u8>>,
        writes: usize,
        fail_at: usize,
    }

    impl ChannelTransport for FailingCloseTransport {
        fn write_channel_payload(&mut self, _payload: &[u8]) -> Result<(), ()> {
            self.writes += 1;
            if self.writes == self.fail_at {
                Err(())
            } else {
                Ok(())
            }
        }

        fn read_channel_payload(&mut self) -> Result<Vec<u8>, ()> {
            self.reads.pop_front().ok_or(())
        }
    }

    fn open_confirmation(local: u32) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_byte(91);
        writer.write_uint32(local);
        writer.write_uint32(99);
        writer.write_uint32(1 << 20);
        writer.write_uint32(32 * 1024);
        writer.into_bytes()
    }

    fn channel_close(local: u32) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_byte(97);
        writer.write_uint32(local);
        writer.into_bytes()
    }

    #[test]
    fn peer_close_before_eof_fails_closed_when_the_reply_write_fails() {
        // A real TCP peer close would race the client's reply write. Failing
        // the write at the ChannelTransport boundary exercises the branch
        // deterministically, with no timing assumption.
        let transport = FailingCloseTransport {
            reads: VecDeque::from(vec![open_confirmation(7), channel_close(7)]),
            writes: 0,
            fail_at: 2,
        };
        let mut sessions = ClientSessions::new(transport);
        sessions.open_session_with_id(7).expect("open session");
        assert!(matches!(
            sessions.dispatch_one(),
            Ok(SessionEvent::Opened { session: 7 })
        ));
        assert!(matches!(
            sessions.dispatch_one(),
            Ok(SessionEvent::Close { session: 7 })
        ));
        let deadline = Instant::now() + Duration::from_secs(60);
        let error = finish_peer_close(&mut sessions, 7, deadline, false)
            .expect_err("a failed CLOSE reply must not be ignored");
        assert_eq!(error.code, ErrorCode::Io);
    }

    #[test]
    fn peer_close_after_our_close_needs_no_reply() {
        let transport = FailingCloseTransport {
            reads: VecDeque::from(vec![open_confirmation(7), channel_close(7)]),
            writes: 0,
            fail_at: 2,
        };
        let mut sessions = ClientSessions::new(transport);
        sessions.open_session_with_id(7).expect("open session");
        assert!(matches!(
            sessions.dispatch_one(),
            Ok(SessionEvent::Opened { session: 7 })
        ));
        assert!(matches!(
            sessions.dispatch_one(),
            Ok(SessionEvent::Close { session: 7 })
        ));
        let deadline = Instant::now() + Duration::from_secs(60);
        finish_peer_close(&mut sessions, 7, deadline, true)
            .expect("no reply is needed after our CLOSE");
    }

    #[test]
    fn timeouts_and_limits_are_the_documented_values() {
        assert_eq!(MAX_COMMAND_BYTES, 64 * 1024);
        assert_eq!(MAX_OUTPUT_BYTES, 8 * 1024 * 1024);
        const {
            assert!(DEFAULT_CONNECT_TIMEOUT_MS <= MAX_CONNECT_TIMEOUT_MS);
            assert!(DEFAULT_EXEC_TIMEOUT_MS <= MAX_EXEC_TIMEOUT_MS);
        }
    }

    #[test]
    fn pem_decoder_ignores_whitespace_and_rejects_other_labels() {
        let pem = b"-----BEGIN PRIVATE KEY-----\nAAEC\nAw==\n-----END PRIVATE KEY-----\n";
        assert_eq!(&*decode_pem(pem, "PRIVATE KEY").unwrap(), &[0, 1, 2, 3]);
        assert!(decode_pem(pem, "OPENSSH PRIVATE KEY").is_err());
        assert!(decode_pem(b"not a key", "PRIVATE KEY").is_err());
    }

    #[test]
    fn handle_counter_skips_zero_after_wrapping() {
        let mut next = u32::MAX;
        next = next.wrapping_add(1);
        assert_eq!(next, 0);
        // The insert loop turns the wrapped zero into one before probing.
        next = next.wrapping_add(1);
        assert_eq!(next, 1);
    }
}
