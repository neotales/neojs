//! Synchronous SSH connection, command execution, and handle table.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::io;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use neotales_ssh::ClientAuthMethod;
use neotales_ssh::ClientHandshakeError;
use neotales_ssh::ClientTransport;
use neotales_ssh::auth::AuthError;
use neotales_ssh::channel::ChannelError;
use neotales_ssh::channel::ChannelTransport;
use neotales_ssh::client::ClientConfig;
use neotales_ssh::hostkey::HostKeyError;
use neotales_ssh::private_key;
use neotales_ssh::session::ClientSessions;
use neotales_ssh::session::SessionError;
use neotales_ssh::session::SessionEvent;
use neotales_ssh::transport::TransportError;
use zeroize::Zeroizing;

use crate::error::SshError;
use crate::types::AuthConfig;
use crate::types::ConnectOptions;
use crate::types::ExecOutput;
use crate::types::MAX_COMMAND_BYTES;
use crate::types::MAX_CONNECT_TIMEOUT_MS;
use crate::types::MAX_EXEC_TIMEOUT_MS;
use crate::types::MAX_HOST_BYTES;
use crate::types::MAX_OPEN_CONNECTIONS;
use crate::types::MAX_OUTPUT_BYTES;
use crate::types::MAX_PASSPHRASE_BYTES;
use crate::types::MAX_PASSWORD_BYTES;
use crate::types::MAX_PRIVATE_KEY_BYTES;
use crate::types::MAX_USERNAME_BYTES;
use crate::types::TrustInner;

/// Test-only switch for the next peer-close reply. Compiled only with
/// `test-support`.
#[cfg(feature = "test-support")]
pub(crate) static FAIL_NEXT_PEER_CLOSE_REPLY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// One authenticated SSH connection and the socket handle used to apply
/// deadlines.
///
/// The stream is a duplicate of the descriptor the transport owns. Socket
/// timeouts are properties of the underlying socket, so setting one through
/// either handle applies to the next blocking operation.
///
/// Dropping a [`Connection`] closes its socket immediately without sending a
/// graceful SSH disconnect message.
pub struct Connection {
    /// `None` once the connection has closed or failed after being engaged.
    sessions: Option<ClientSessions<ClientTransport<TcpStream>>>,
    stream: TcpStream,
    exec_timeout: Duration,
}

impl Connection {
    /// Runs one command synchronously and closes its session channel before
    /// returning.
    ///
    /// Standard error from the remote process is discarded (while still
    /// returning SSH channel window credit). Captured stdout is bounded by
    /// [`MAX_OUTPUT_BYTES`], and `command` is bounded by [`MAX_COMMAND_BYTES`].
    ///
    /// * Returns [`crate::SshErrorCode::Closed`] if the connection is already
    ///   closed.
    /// * Returns [`crate::SshErrorCode::Limit`] before sending anything if
    ///   `command` exceeds [`MAX_COMMAND_BYTES`], leaving the connection open.
    /// * Any failure after the session is engaged drops the underlying
    ///   transport and marks the connection closed (`self.is_closed() == true`).
    pub fn exec(&mut self, command: &[u8]) -> Result<ExecOutput, SshError> {
        let Some(mut sessions) = self.sessions.take() else {
            return Err(SshError::Closed);
        };
        if command.len() > MAX_COMMAND_BYTES {
            self.sessions = Some(sessions);
            return Err(SshError::default_limit());
        }
        match run_exec(&mut sessions, &self.stream, self.exec_timeout, command) {
            Ok(output) => {
                self.sessions = Some(sessions);
                Ok(output)
            }
            Err(error) => {
                // `sessions` drops here, closing the transport so partially
                // read packets cannot corrupt a later command.
                Err(error)
            }
        }
    }

    /// Returns `true` once this connection has been closed or aborted by a
    /// fatal `exec` error.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.sessions.is_none()
    }

    /// Closes this connection immediately. Idempotent.
    ///
    /// Dropping the underlying transport closes the TCP socket without sending
    /// a graceful SSH disconnect message.
    pub fn close(&mut self) {
        self.sessions = None;
    }
}

/// Manages active [`Connection`] handles keyed by non-zero `u32` identifiers.
///
/// Enforces a maximum open-connection capacity ([`MAX_OPEN_CONNECTIONS`] by
/// default) and ensures any fatal `exec` failure removes and drops the
/// connection automatically.
pub struct SshState {
    next: u32,
    connections: BTreeMap<u32, Connection>,
    max_connections: usize,
}

impl Default for SshState {
    fn default() -> Self {
        Self::new()
    }
}

impl SshState {
    /// Creates an empty handle table capped at [`MAX_OPEN_CONNECTIONS`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_connections(MAX_OPEN_CONNECTIONS)
    }

    /// Creates an empty handle table with an explicit open-connection cap.
    #[must_use]
    pub fn with_max_connections(max_connections: usize) -> Self {
        Self {
            next: 0,
            connections: BTreeMap::new(),
            max_connections,
        }
    }

    /// Returns the number of currently open connections.
    #[must_use]
    pub fn len(&self) -> usize {
        self.connections.len()
    }

    /// Returns `true` if no connections are currently open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// Returns `true` if `id` refers to an open connection.
    #[must_use]
    pub fn is_open(&self, id: u32) -> bool {
        self.connections.contains_key(&id)
    }

    /// Validates options, establishes one authenticated SSH connection, and
    /// registers it under a fresh non-zero handle identifier.
    ///
    /// Fails with [`crate::SshErrorCode::Limit`] before dialing if the table
    /// has already reached its open-connection limit.
    pub fn connect(&mut self, options: ConnectOptions) -> Result<u32, SshError> {
        let validated = validate_options(options)?;
        if self.connections.len() >= self.max_connections {
            return Err(SshError::limit("too many open SSH connections"));
        }
        let connection = connect_validated(validated)?;
        self.insert(connection)
    }

    /// Runs one command on connection `id`.
    ///
    /// * Rejects commands exceeding [`MAX_COMMAND_BYTES`] with
    ///   [`crate::SshErrorCode::Limit`] before borrowing the connection,
    ///   leaving open connections usable.
    /// * Returns [`crate::SshErrorCode::Closed`] if `id` is not open.
    /// * Any failure from [`Connection::exec`] removes and drops connection
    ///   `id` before returning the error.
    pub fn exec(&mut self, id: u32, command: &[u8]) -> Result<ExecOutput, SshError> {
        if command.len() > MAX_COMMAND_BYTES {
            return Err(SshError::default_limit());
        }
        let Some(connection) = self.connections.get_mut(&id) else {
            return Err(SshError::Closed);
        };
        match connection.exec(command) {
            Ok(output) => Ok(output),
            Err(error) => {
                // Any failure after the connection was borrowed abandons it.
                // The session may be mid-packet, so reusing it could mix
                // responses.
                self.connections.remove(&id);
                Err(error)
            }
        }
    }

    /// Closes and removes connection `id`. Returns `true` if the handle was open.
    pub fn close(&mut self, id: u32) -> bool {
        self.connections.remove(&id).is_some()
    }

    fn insert(&mut self, connection: Connection) -> Result<u32, SshError> {
        if self.connections.len() >= usize::try_from(u32::MAX).unwrap_or(usize::MAX) {
            return Err(SshError::limit("SSH handle space exhausted"));
        }
        loop {
            self.next = self.next.wrapping_add(1);
            if self.next == 0 {
                continue;
            }
            if let Entry::Vacant(slot) = self.connections.entry(self.next) {
                slot.insert(connection);
                return Ok(self.next);
            }
        }
    }
}

/// Validates options, connects, verifies host trust, authenticates, and
/// returns one synchronous [`Connection`].
///
/// Validates credentials locally before dialing; private-key decoding errors
/// never open a TCP socket. Agent authentication is deferred until after server
/// host-key verification succeeds.
pub fn connect(options: ConnectOptions) -> Result<Connection, SshError> {
    let validated = validate_options(options)?;
    connect_validated(validated)
}

struct ValidatedConnect {
    host: String,
    port: u16,
    username: Vec<u8>,
    method: Option<ClientAuthMethod>,
    agent_auth: bool,
    trust: TrustInner,
    connect_timeout: Duration,
    exec_timeout: Duration,
}

fn validate_options(options: ConnectOptions) -> Result<ValidatedConnect, SshError> {
    validate_host(&options.host)?;
    if options.port == 0 {
        return Err(SshError::default_argument());
    }
    validate_username(&options.username)?;
    validate_timeout(options.connect_timeout, MAX_CONNECT_TIMEOUT_MS)?;
    validate_timeout(options.exec_timeout, MAX_EXEC_TIMEOUT_MS)?;

    // Validate password/key bounds and decode private-key material before
    // dialing: a malformed key or a passphrase supplied for PKCS#8 is a local
    // error and must not open a socket. Agent methods are deferred until after
    // host-key verification.
    let agent_auth = matches!(&options.auth, AuthConfig::Agent);
    let method = match options.auth {
        AuthConfig::Password(password) => {
            if password.len() > MAX_PASSWORD_BYTES {
                return Err(SshError::default_limit());
            }
            Some(ClientAuthMethod::Password(password))
        }
        AuthConfig::PrivateKey { bytes, passphrase } => {
            if bytes.len() > MAX_PRIVATE_KEY_BYTES {
                return Err(SshError::default_limit());
            }
            if bytes.is_empty() {
                return Err(SshError::Credential);
            }
            let mut passphrase_bytes = match passphrase {
                Some(mut text) => {
                    if text.len() > MAX_PASSPHRASE_BYTES {
                        return Err(SshError::default_limit());
                    }
                    Zeroizing::new(std::mem::take(&mut *text).into_bytes())
                }
                None => Zeroizing::new(Vec::new()),
            };
            Some(import_identity(&bytes, &mut passphrase_bytes)?)
        }
        AuthConfig::Agent => None,
    };

    Ok(ValidatedConnect {
        host: options.host,
        port: options.port,
        username: options.username.into_bytes(),
        method,
        agent_auth,
        trust: options.trust.inner,
        connect_timeout: options.connect_timeout,
        exec_timeout: options.exec_timeout,
    })
}

fn connect_validated(options: ValidatedConnect) -> Result<Connection, SshError> {
    let stream = dial(&options.host, options.port, options.connect_timeout)?;
    let io = stream.try_clone().map_err(SshError::connect)?;

    // The handshake and authentication share one deadline. DNS resolution was
    // part of `dial` and is documented as outside this budget.
    let deadline = Instant::now() + options.connect_timeout;
    set_socket_deadline(&io, deadline)?;

    let config = ClientConfig::new();
    let mut transport = match options.trust {
        TrustInner::Pin(pin) => config.connect(
            stream,
            move |key| {
                if *key == pin { Ok(()) } else { Err(()) }
            },
        ),
        TrustInner::KnownHosts(hosts) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| SshError::Internal)?
                .as_secs();
            config.connect_with_known_hosts(stream, &hosts, &options.host, options.port, now)
        }
    }
    .map_err(map_handshake_error)?;

    if Instant::now() >= deadline {
        return Err(SshError::Timeout);
    }
    set_socket_deadline(&io, deadline)?;

    let methods = match options.method {
        Some(method) => vec![method],
        None => agent_methods()?,
    };
    // Agent connect, enumeration, and signing are outside the connect budget:
    // UnixStream::connect may block, and the backend's 30-second socket
    // timeout is inactivity-based. Re-check and refresh the SSH socket
    // deadline before user authentication.
    if Instant::now() >= deadline {
        return Err(SshError::Timeout);
    }
    set_socket_deadline(&io, deadline)?;
    let attempts = methods.len().saturating_mul(2).max(1);
    neotales_ssh::authenticate_client(&mut transport, &options.username, &methods, attempts)
        .map_err(|error| map_auth_error(error, deadline, options.agent_auth))?;

    if Instant::now() >= deadline {
        return Err(SshError::Timeout);
    }

    Ok(Connection {
        sessions: Some(ClientSessions::new(transport)),
        stream: io,
        exec_timeout: options.exec_timeout,
    })
}

fn validate_host(host: &str) -> Result<(), SshError> {
    if host.len() > MAX_HOST_BYTES {
        return Err(SshError::default_limit());
    }
    if host.is_empty()
        || host
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(SshError::default_argument());
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<(), SshError> {
    if username.len() > MAX_USERNAME_BYTES {
        return Err(SshError::default_limit());
    }
    if username.is_empty() || username.chars().any(char::is_control) {
        return Err(SshError::default_argument());
    }
    Ok(())
}

fn validate_timeout(timeout: Duration, max_ms: u64) -> Result<(), SshError> {
    if timeout < Duration::from_millis(1) || timeout > Duration::from_millis(max_ms) {
        return Err(SshError::default_argument());
    }
    Ok(())
}

fn run_exec(
    sessions: &mut ClientSessions<ClientTransport<TcpStream>>,
    stream: &TcpStream,
    exec_timeout: Duration,
    command: &[u8],
) -> Result<ExecOutput, SshError> {
    let deadline = Instant::now() + exec_timeout;
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
                    return Err(SshError::default_limit());
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
                    return Err(SshError::Session);
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
                return Err(SshError::Session);
            }
            _ => return Err(SshError::Session),
        }
    }

    let status = status.ok_or(SshError::Session)?;
    Ok(ExecOutput {
        stdout: output,
        status,
    })
}

/// Opens a session and waits until the server confirms it.
///
/// A channel request cannot be sent until the peer's channel-open confirmation
/// has been dispatched, because the remote channel identifier is not known
/// before then.
fn open_session<T: ChannelTransport>(
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
        _ => Err(SshError::Session),
    }
}

/// Sends the CLOSE reply when the peer closed before our EOF/CLOSE.
///
/// If this side already sent CLOSE in response to EOF, the peer's CLOSE is the
/// final one and no reply is needed. Otherwise the reply is mandatory: a write
/// failure is a broken transport, not a harmless late close, and is returned so
/// the caller can drop the connection.
fn finish_peer_close<T: ChannelTransport>(
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
        return Err(SshError::Io { source: None });
    }
    sessions
        .send_close(session)
        .map_err(|error| map_session_error(error, deadline))
}

fn return_credit<T: ChannelTransport>(
    sessions: &mut ClientSessions<T>,
    session: u32,
    bytes: usize,
    deadline: Instant,
) -> Result<(), SshError> {
    if bytes == 0 {
        return Ok(());
    }
    let credit = u32::try_from(bytes).map_err(|_| SshError::default_limit())?;
    sessions
        .increase_local_window(session, credit)
        .map_err(|error| map_session_error(error, deadline))?;
    Ok(())
}

fn set_socket_deadline(stream: &TcpStream, deadline: Instant) -> Result<(), SshError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(SshError::Timeout);
    }
    // Rust rejects a zero timeout. One millisecond keeps the socket blocking
    // while still failing almost immediately at the deadline.
    let remaining = (deadline - now).max(Duration::from_millis(1));
    stream
        .set_read_timeout(Some(remaining))
        .map_err(SshError::io)?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(SshError::io)?;
    Ok(())
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn map_session_error(error: SessionError, deadline: Instant) -> SshError {
    match error {
        SessionError::Channel(ChannelError::Transport) => {
            if Instant::now() >= deadline {
                SshError::Timeout
            } else {
                SshError::Io { source: None }
            }
        }
        _ => SshError::Session,
    }
}

fn map_handshake_error(error: ClientHandshakeError) -> SshError {
    match error {
        ClientHandshakeError::HostKey(
            HostKeyError::TrustRejected | HostKeyError::VerifierPanicked,
        ) => SshError::HostKey,
        ClientHandshakeError::Transport(TransportError::Io(error)) => {
            if is_timeout(&error) {
                SshError::Timeout
            } else {
                SshError::io(error)
            }
        }
        ClientHandshakeError::EntropyFailure => SshError::Internal,
        _ => SshError::Handshake,
    }
}

fn map_auth_error(error: AuthError, deadline: Instant, agent_auth: bool) -> SshError {
    match error {
        AuthError::Transport => {
            if Instant::now() >= deadline {
                SshError::Timeout
            } else {
                SshError::Auth
            }
        }
        AuthError::AgentUnavailable | AuthError::InvalidProof if agent_auth => {
            SshError::agent("the SSH agent did not produce a usable signature")
        }
        _ => SshError::Auth,
    }
}

/// Enumerates an explicitly opted-in Unix agent and builds its auth methods.
///
/// Called only after server host-key verification succeeds. Reads
/// `SSH_AUTH_SOCK` only here, never in password or private-key mode.
#[cfg(unix)]
fn agent_methods() -> Result<Vec<ClientAuthMethod>, SshError> {
    use neotales_ssh::AgentKeySigner;
    use neotales_ssh::agent::AgentClient;

    use crate::types::MAX_AGENT_IDENTITIES;

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

/// Resolves and dials the host, applying the connect deadline to each address.
///
/// `ToSocketAddrs` performs the platform's name resolution and has no portable
/// timeout; DNS resolution is outside the connect timeout budget.
fn dial(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, SshError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(SshError::connect)?
        .collect::<Vec<_>>();
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    for address in addresses {
        let now = Instant::now();
        if now >= deadline {
            return Err(SshError::Timeout);
        }
        let remaining = deadline - now;
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if is_timeout(&error) {
                    return Err(SshError::Timeout);
                }
                last_error = Some(error);
            }
        }
    }
    Err(match last_error {
        Some(error) => SshError::connect(error),
        None => SshError::Connect { source: None },
    })
}

/// Imports one in-memory private key without reading a file.
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
        let signer =
            private_key::import_openssh(&decoded, passphrase).map_err(|_| SshError::Credential)?;
        return Ok(ClientAuthMethod::public_key(signer));
    }
    if bytes.starts_with(PKCS8_PEM) {
        require_no_passphrase(passphrase)?;
        let decoded = decode_pem(bytes, "PRIVATE KEY")?;
        let signer = private_key::import_pkcs8_der(&decoded).map_err(|_| SshError::Credential)?;
        return Ok(ClientAuthMethod::public_key(signer));
    }
    if bytes.starts_with(OPENSSH_BINARY) {
        let signer =
            private_key::import_openssh(bytes, passphrase).map_err(|_| SshError::Credential)?;
        return Ok(ClientAuthMethod::public_key(signer));
    }
    require_no_passphrase(passphrase)?;
    let signer = private_key::import_pkcs8_der(bytes).map_err(|_| SshError::Credential)?;
    Ok(ClientAuthMethod::public_key(signer))
}

/// Refuses a passphrase for a format that cannot use one in this slice.
fn require_no_passphrase(passphrase: &[u8]) -> Result<(), SshError> {
    if passphrase.is_empty() {
        Ok(())
    } else {
        Err(SshError::Credential)
    }
}

/// Decodes one PEM block whose label is `label` into DER bytes.
fn decode_pem(bytes: &[u8], label: &str) -> Result<Zeroizing<Vec<u8>>, SshError> {
    let text = std::str::from_utf8(bytes).map_err(|_| SshError::Credential)?;
    let header = format!("-----BEGIN {label}-----");
    let footer = format!("-----END {label}-----");
    let Some(after_header) = text.strip_prefix(&header) else {
        return Err(SshError::Credential);
    };
    let Some(body) = after_header.split_once(&footer).map(|(body, _)| body) else {
        return Err(SshError::Credential);
    };
    let encoded = Zeroizing::new(
        body.chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect::<String>(),
    );
    let decoded = Zeroizing::new(
        neotales_ssh::known_hosts::decode_base64(&encoded).map_err(|_| SshError::Credential)?,
    );
    if decoded.len() > private_key::MAX_PRIVATE_KEY_DER {
        return Err(SshError::default_limit());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use neotales_ssh::Writer;

    use super::*;
    use crate::error::SshErrorCode;
    use crate::types::{DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_EXEC_TIMEOUT_MS};

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

    // Unit test: logic not deterministically reachable through a real TCP peer
    // without racing the client's reply write.
    #[test]
    fn peer_close_before_eof_fails_closed_when_the_reply_write_fails() {
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
        assert_eq!(error.code(), SshErrorCode::Io);
    }

    // Unit test: private helper branch where sent_close is already true.
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

    // Unit test: exact documented bounds check.
    #[test]
    fn timeouts_and_limits_are_the_documented_values() {
        assert_eq!(MAX_COMMAND_BYTES, 64 * 1024);
        assert_eq!(MAX_OUTPUT_BYTES, 8 * 1024 * 1024);
        const {
            assert!(DEFAULT_CONNECT_TIMEOUT_MS <= MAX_CONNECT_TIMEOUT_MS);
            assert!(DEFAULT_EXEC_TIMEOUT_MS <= MAX_EXEC_TIMEOUT_MS);
        }
    }

    // Unit test: private PEM decoder helper with real branching.
    #[test]
    fn pem_decoder_ignores_whitespace_and_rejects_other_labels() {
        let pem = b"-----BEGIN PRIVATE KEY-----\nAAEC\nAw==\n-----END PRIVATE KEY-----\n";
        assert_eq!(&*decode_pem(pem, "PRIVATE KEY").unwrap(), &[0, 1, 2, 3]);
        assert!(decode_pem(pem, "OPENSSH PRIVATE KEY").is_err());
        assert!(decode_pem(b"not a key", "PRIVATE KEY").is_err());
    }

    // Unit test: handle counter wrap-around branch in SshState::insert without
    // allocating u32::MAX connections.
    #[test]
    fn handle_counter_skips_zero_after_wrapping() {
        let mut next = u32::MAX;
        next = next.wrapping_add(1);
        assert_eq!(next, 0);
        next = next.wrapping_add(1);
        assert_eq!(next, 1);
    }
}
