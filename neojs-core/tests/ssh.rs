//! Integration tests for the `ssh` built-in through the public `Runtime`.
//!
//! Every connection test uses a deterministic loopback SSH server built with
//! `neotales-ssh`'s public server API. The host key is derived from a fixed
//! seed, the password verifier accepts a fixed test value, and the server never
//! contacts an external host. Host-key failures are exercised by pinning a
//! different valid key, and the server is asserted to have observed no command.
//! No real credential file is read.

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use neojs_core::Runtime;
use neotales_ssh::ServerAuthPolicy;
use neotales_ssh::ServerConfig;
use neotales_ssh::ServerSessions;
use neotales_ssh::authenticate_server;
use neotales_ssh::channel::ChannelTransport;
use neotales_ssh::hostkey::Ed25519Signer;
use neotales_ssh::known_hosts::AppendEntry;
use neotales_ssh::known_hosts::format_append_entry;
use neotales_ssh::known_hosts::format_host_token;
use neotales_ssh::session::SessionDecision;
use neotales_ssh::session::SessionEvent;
use neotales_ssh::session::SessionRequest;

/// Writes a module into a fresh temporary directory and runs it.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

/// A test password. It is not a credential and is used only on loopback.
const TEST_PASSWORD: &str = "fixture-password";

/// A non-ASCII test password, used to check UTF-8 encoding on the wire.
const UNICODE_PASSWORD: &str = "pässwörd-🔑";

/// Build the complete Ed25519 host-key blob for a fixed seed.
fn host_key_blob(seed: u8) -> Vec<u8> {
    let mut bytes = [seed; 32];
    Ed25519Signer::from_seed(&mut bytes)
        .public_key_blob()
        .expect("host key blob")
}

/// A JavaScript expression for a `Uint8Array` containing `bytes`.
fn js_bytes(bytes: &[u8]) -> String {
    let items = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("Uint8Array.from([{items}])")
}

/// A JavaScript string literal for `value`, with JSON escaping.
fn js_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

/// How much stdout the fixture sends after an accepted `exec`.
enum Reply {
    Fixed(&'static [u8]),
    /// Send stdout, then the exit status, then more stdout, then EOF/CLOSE.
    Split(&'static [u8], &'static [u8]),
    /// Send stdout and the exit status, then CLOSE without a preceding EOF.
    CloseBeforeEof(&'static [u8]),
    /// Send the exit status before all stdout, so the client must keep
    /// collecting and enforce the limit after status.
    BytesAfterStatus(usize),
    /// Accept the request and send nothing, so the client must hit its
    /// `execTimeoutMs` deadline.
    Silent,
}

/// What the server observed after the client closed the connection.
struct Finished {
    /// Every `exec` command the server accepted, in order.
    commands: Vec<Vec<u8>>,
    /// Authentication proofs the server evaluated. A host-key rejection must
    /// leave this at zero.
    attempts: usize,
}

struct Fixture {
    port: u16,
    observed: mpsc::Receiver<Vec<u8>>,
    attempts: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl Fixture {
    /// Stop the server and return what it observed.
    fn finish(self) -> Finished {
        self.handle.join().expect("server thread");
        Finished {
            commands: self.observed.try_iter().collect(),
            attempts: self.attempts.load(Ordering::SeqCst),
        }
    }
}

/// Run the deterministic password-authenticated loopback server.
fn password_server(seed: u8, password: &'static str, reply: Reply) -> Fixture {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let port = listener.local_addr().expect("local address").port();
    let (observed, observed_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_auth = Arc::clone(&attempts);
    let handle = thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        // Keep a broken client or a test panic from hanging the suite.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
        let mut key_seed = [seed; 32];
        let config = ServerConfig::new(Ed25519Signer::from_seed(&mut key_seed));
        let Ok(mut transport) = config.accept(stream) else {
            return;
        };
        let Ok(policy) = ServerAuthPolicy::new(4).map(|policy| {
            policy.with_password_verifier(move |candidate: &str| {
                attempts_for_auth.fetch_add(1, Ordering::SeqCst);
                candidate == password
            })
        }) else {
            return;
        };
        if authenticate_server(&mut transport, &policy).is_err() {
            return;
        }
        let mut sessions = ServerSessions::with_policy(transport, move |request| {
            if let SessionRequest::Exec { command } = request {
                let _ = observed.send(command.to_vec());
                SessionDecision::accept(0)
            } else {
                SessionDecision::reject()
            }
        });
        serve(&mut sessions, reply);
    });
    Fixture {
        port,
        observed: observed_rx,
        attempts,
        handle,
    }
}

/// Serve one connection's session events until the peer closes.
fn serve<T: ChannelTransport>(sessions: &mut ServerSessions<T>, reply: Reply) {
    loop {
        match sessions.dispatch_one() {
            Ok(SessionEvent::Request {
                session,
                accepted: true,
                ..
            }) => {
                let sent = match reply {
                    Reply::Fixed(bytes) => {
                        sessions.send_data(session, bytes).is_ok()
                            && sessions.send_exit_status(session).is_ok()
                    }
                    Reply::Split(before, after) => {
                        sessions.send_data(session, before).is_ok()
                            && sessions.send_exit_status(session).is_ok()
                            && sessions.connection_mut().write_data(session, after).is_ok()
                    }
                    Reply::CloseBeforeEof(bytes) => {
                        sessions.send_data(session, bytes).is_ok()
                            && sessions.send_exit_status(session).is_ok()
                    }
                    Reply::BytesAfterStatus(total) => {
                        sessions.send_exit_status(session).is_ok()
                            && send_bytes_after_status(sessions, session, total)
                    }
                    Reply::Silent => false,
                };
                if sent {
                    if matches!(reply, Reply::CloseBeforeEof(_)) {
                        // The session API requires EOF before CLOSE; the
                        // channel writer lets the fixture exercise a peer that
                        // closes first instead.
                        let _ = sessions.connection_mut().send_close(session);
                    } else {
                        let _ = sessions.send_eof(session);
                        let _ = sessions.send_close(session);
                    }
                }
            }
            // A channel close ends one command, not the connection: the client
            // may open another session on the same authenticated transport.
            Ok(SessionEvent::Close { .. }) => {}
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

/// Send `total` bytes through the lower-level channel writer, bypassing the
/// session API's rule that no data may follow `exit-status`.
///
/// The SSH channel layer and the receiving client both allow this ordering;
/// the server convenience wrapper forbids it, so a peer-emulation test needs
/// the channel writer directly.
fn send_bytes_after_status<T: ChannelTransport>(
    sessions: &mut ServerSessions<T>,
    session: u32,
    total: usize,
) -> bool {
    use neotales_ssh::channel::ChannelError;
    use neotales_ssh::channel::WindowError;

    let chunk = vec![b'x'; 32 * 1024];
    let mut remaining = total;
    let mut limit = chunk.len();
    while remaining > 0 {
        let length = remaining.min(limit);
        match sessions
            .connection_mut()
            .write_data(session, &chunk[..length])
        {
            Ok(written) => {
                remaining -= written;
                limit = chunk.len();
            }
            Err(ChannelError::Window(WindowError::Insufficient { available, .. }))
                if available > 0 =>
            {
                limit = usize::try_from(available).unwrap_or(limit).min(chunk.len());
            }
            Err(ChannelError::Window(WindowError::Insufficient { .. })) => {
                match sessions.dispatch_one() {
                    Ok(SessionEvent::WindowAdjusted { .. }) => limit = chunk.len(),
                    _ => return false,
                }
            }
            Err(_) => return false,
        }
    }
    true
}

#[test]
fn module_identity_constants_and_no_network_capability_leak() {
    run_js(
        r#"
import { connect } from "ssh";
import * as aliased from "neo:ssh";

const registry = Neo.getBuiltInModule("ssh");
if (typeof registry !== "object" || registry === null) throw Error("missing registry module");
if (Neo.getBuiltInModule("ssh") !== registry) throw Error("registry lookup is not cached");
if (!Object.isFrozen(registry)) throw Error("registry module is not frozen");
if (registry.connect !== connect) throw Error("bare import is not the registry function");
if (aliased.connect !== connect) throw Error("neo: import is not the registry function");
const registryNames = Object.keys(registry).sort().join(",");
const moduleNames = Object.keys(aliased).sort().join(",");
if (registryNames !== moduleNames) throw Error("export list drifted: " + registryNames + " / " + moduleNames);
if (registry.MAX_COMMAND_BYTES !== 65536) throw Error("MAX_COMMAND_BYTES");
if (registry.MAX_OUTPUT_BYTES !== 8388608) throw Error("MAX_OUTPUT_BYTES");
if (registry.DEFAULT_CONNECT_TIMEOUT_MS !== 15000) throw Error("DEFAULT_CONNECT_TIMEOUT_MS");
if (registry.DEFAULT_EXEC_TIMEOUT_MS !== 30000) throw Error("DEFAULT_EXEC_TIMEOUT_MS");
if (typeof globalThis.connect === "function") throw Error("connect leaked as a global");
if (typeof globalThis.fs !== "undefined" || typeof globalThis.env !== "undefined") {
    throw Error("filesystem or environment capability leaked");
}
"#,
    );
}

#[test]
fn trust_is_mandatory_and_never_defaults() {
    let blob = js_bytes(&host_key_blob(7));
    let known_hosts = js_string("[127.0.0.1]:22 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample");
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";

function expectCode(code, fn) {{
    try {{
        fn();
    }} catch (error) {{
        if (error.name !== "SshError") throw error;
        if (error.code !== code) throw Error(`expected ${{code}}, got ${{error.code}}`);
        return;
    }}
    throw Error("expected " + code);
}}

expectCode("NEO_SSH_HOST_KEY_REQUIRED", () => connect({{
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: "fixture",
}}));
expectCode("NEO_SSH_HOST_KEY_REQUIRED", () => connect({{
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: "fixture",
    hostKey: {blob},
    knownHosts: {known_hosts},
}}));
"#
    ));
}

#[test]
fn argument_validation_is_strict_and_does_not_connect() {
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
const base = {{
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: "fixture",
    hostKey: {blob},
}};

function expectType(fn, label) {{
    try {{ fn(); }} catch (error) {{
        if (!(error instanceof TypeError)) throw error;
        if ("code" in error) throw Error(label + ": unexpected code");
        return;
    }}
    throw Error(label + ": expected TypeError");
}}
function expectRange(fn, label) {{
    try {{ fn(); }} catch (error) {{
        if (!(error instanceof RangeError)) throw error;
        return;
    }}
    throw Error(label + ": expected RangeError");
}}
function expectCode(code, fn, label) {{
    try {{ fn(); }} catch (error) {{
        if (error.name !== "SshError" || error.code !== code) throw error;
        return;
    }}
    throw Error(label + ": expected " + code);
}}

expectType(() => connect(), "no argument");
expectType(() => connect(base, base), "two arguments");
expectType(() => connect(null), "null options");
expectType(() => connect([]), "array options");
expectType(() => connect({{ ...base, unknownOption: true }}), "unknown option");
expectType(() => connect({{ ...base, host: 1 }}), "host type");
expectType(() => connect({{ ...base, username: null }}), "username type");
expectType(() => connect({{ ...base, password: 1 }}), "password type");
expectType(() => connect({{ ...base, privateKey: "text" }}), "privateKey type");
expectType(() => connect({{ ...base, hostKey: "text" }}), "hostKey type");
expectRange(() => connect({{ ...base, port: 0 }}), "port zero");
expectRange(() => connect({{ ...base, port: 65536 }}), "port high");
expectRange(() => connect({{ ...base, port: 1.5 }}), "port fraction");
expectRange(() => connect({{ ...base, connectTimeoutMs: 0 }}), "connect timeout zero");
expectRange(() => connect({{ ...base, execTimeoutMs: 600001 }}), "exec timeout high");

expectCode("NEO_SSH_AUTH_METHOD", () => connect({{ ...base, password: undefined }}), "no auth");
expectCode("NEO_SSH_AUTH_METHOD", () => connect({{
    ...base,
    privateKey: Uint8Array.from([1, 2, 3]),
}}), "two auth methods");
expectCode("NEO_SSH_ARGUMENT", () => connect({{ ...base, passphrase: "x" }}), "passphrase without key");
expectCode("NEO_SSH_CREDENTIAL", () => connect({{
    ...base,
    password: undefined,
    privateKey: new Uint8Array(0),
}}), "empty key");
expectCode("NEO_SSH_ARGUMENT", () => connect({{ ...base, password: "\uD800" }}), "lone surrogate password");
expectType(() => connect({{ ...base, agent: 1 }}), "agent type");
expectCode("NEO_SSH_AUTH_METHOD", () => connect({{ ...base, agent: true }}), "two auth methods");
expectCode("NEO_SSH_ARGUMENT", () => connect({{
    ...base,
    password: undefined,
    agent: true,
    passphrase: "unused",
}}), "agent passphrase");
expectCode("NEO_SSH_AUTH_METHOD", () => connect({{ ...base, password: undefined, agent: false }}), "agent false alone");
expectCode("NEO_SSH_HOST_KEY", () => connect({{ ...base, hostKey: Uint8Array.from([1, 2, 3]) }}), "bad pin");
expectCode("NEO_SSH_KNOWN_HOSTS", () => connect({{
    ...base,
    hostKey: undefined,
    knownHosts: "not a known-hosts line",
}}), "bad known hosts");
"#
    ));
}

#[test]
fn errors_do_not_echo_passwords_passphrases_or_key_bytes() {
    run_js(
        r#"
import { connect } from "neo:ssh";

const password = "UNIQUE-PASSWORD-7f3a9c";
const passphrase = "UNIQUE-PASSPHRASE-2b8d41";
const key = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x11, 0x22]);
const needles = [password, passphrase, "222,173,190,239", "deadbeef"];

function noSecret(fn, code) {
    try {
        fn();
    } catch (error) {
        if (error.name !== "SshError" || error.code !== code) throw error;
        const text = [String(error), String(error.message), String(error.stack)].join("\n");
        for (const needle of needles) {
            if (text.includes(needle)) throw Error("error echoed a secret");
        }
        return;
    }
    throw Error("expected " + code);
}

noSecret(() => connect({
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password,
}), "NEO_SSH_HOST_KEY_REQUIRED");

noSecret(() => connect({
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: undefined,
    privateKey: key,
    passphrase,
    knownHosts: "not a known-hosts line",
}), "NEO_SSH_KNOWN_HOSTS");

noSecret(() => connect({
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password,
    privateKey: key,
}), "NEO_SSH_AUTH_METHOD");
"#,
    );
}

#[test]
fn a_wrong_host_key_is_refused_before_any_command() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"must not run\n"));
    let wrong = js_bytes(&host_key_blob(8));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
let failed = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {port},
        username: "fixture",
        password: {password},
        hostKey: {wrong},
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_HOST_KEY") throw error;
    failed = true;
}}
if (!failed) throw Error("connect accepted an untrusted host key");
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        wrong = wrong,
    ));
    // The module must not leave the server a session or command to observe,
    // and it must not have reached authentication.
    let finished = fixture.finish();
    assert!(
        finished.commands.is_empty(),
        "the server accepted a command despite the host-key mismatch"
    );
    assert_eq!(
        finished.attempts, 0,
        "authentication ran before host-key verification"
    );
}

#[test]
fn a_changed_known_hosts_entry_is_refused_before_any_command() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"must not run\n"));
    let wrong_blob = host_key_blob(8);
    let token = format_host_token("127.0.0.1", fixture.port);
    let entry = AppendEntry::plain(&token, &wrong_blob).expect("known-hosts entry");
    let known_hosts = format_append_entry(&entry).expect("known-hosts text");
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
let failed = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {port},
        username: "fixture",
        password: {password},
        knownHosts: {known_hosts},
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_HOST_KEY") throw error;
    failed = true;
}}
if (!failed) throw Error("connect accepted a changed known-hosts key");
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        known_hosts = js_string(&known_hosts),
    ));
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(
        finished.attempts, 0,
        "authentication ran before known-hosts verification"
    );
}

#[test]
fn a_wrong_password_is_an_authentication_failure() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"must not run\n"));
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
let failed = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {port},
        username: "fixture",
        password: "wrong",
        hostKey: {blob},
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_AUTH") throw error;
    failed = true;
}}
if (!failed) throw Error("connect accepted the wrong password");
"#,
        port = fixture.port,
        blob = blob,
    ));
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(finished.attempts, 1, "the wrong password was not evaluated");
}

#[test]
fn exec_roundtrips_bytes_reuses_the_connection_and_closes() {
    let fixture = password_server(7, UNICODE_PASSWORD, Reply::Fixed(b"fixture stdout\n"));
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect, MAX_COMMAND_BYTES, MAX_OUTPUT_BYTES }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
if (connection.closed) throw Error("new connection is closed");
if (MAX_COMMAND_BYTES !== 65536 || MAX_OUTPUT_BYTES !== 8388608) throw Error("limits");

function text(bytes) {{
    let result = "";
    for (let index = 0; index < bytes.length; index += 1) {{
        result += String.fromCharCode(bytes[index]);
    }}
    return result;
}}

const first = connection.exec("echo hello");
if (!(first.stdout instanceof Uint8Array)) throw Error("stdout type");
if (text(first.stdout) !== "fixture stdout\n") throw Error("stdout bytes");
if (first.status !== 0) throw Error("status");

const second = connection.exec("echo héllo");
if (text(second.stdout) !== "fixture stdout\n") throw Error("second stdout");
if (second.status !== 0) throw Error("second status");

connection.close();
connection.close();
if (!connection.closed) throw Error("close did not mark the connection closed");
try {{
    connection.exec("echo again");
    throw Error("exec after close succeeded");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_CLOSED") throw error;
}}
"#,
        port = fixture.port,
        password = js_string(UNICODE_PASSWORD),
        blob = blob,
    ));
    let finished = fixture.finish();
    assert_eq!(
        finished.commands,
        vec![b"echo hello".to_vec(), "echo héllo".as_bytes().to_vec()],
        "the server did not observe the exact command bytes"
    );
    assert_eq!(finished.attempts, 1);
}

#[test]
fn explicit_known_hosts_text_is_accepted() {
    let fixture = password_server(11, TEST_PASSWORD, Reply::Fixed(b"known-hosts\n"));
    let blob = host_key_blob(11);
    let token = format_host_token("127.0.0.1", fixture.port);
    let entry = AppendEntry::plain(&token, &blob).expect("known-hosts entry");
    let known_hosts = format_append_entry(&entry).expect("known-hosts text");
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    knownHosts: {known_hosts},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const result = connection.exec("hostname");
if (result.status !== 0) throw Error("status");
if (result.stdout.length !== 12) throw Error("output length");
connection.close();
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        known_hosts = js_string(&known_hosts),
    ));
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"hostname".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

#[test]
fn oversized_command_is_refused_without_a_round_trip() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"must not run\n"));
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect, MAX_COMMAND_BYTES }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
try {{
    connection.exec("x".repeat(MAX_COMMAND_BYTES + 1));
    throw Error("oversized command was accepted");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_LIMIT") throw error;
}}
if (connection.closed) throw Error("an argument error closed the connection");
connection.close();
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        blob = blob,
    ));
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(finished.attempts, 1, "the connection was not authenticated");
}

#[test]
fn stdout_after_the_exit_status_is_collected() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Split(b"before\n", b"after\n"));
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const result = connection.exec("split");
if (result.status !== 0) throw Error("status");
let text = "";
for (let index = 0; index < result.stdout.length; index += 1) {{
    text += String.fromCharCode(result.stdout[index]);
}}
if (text !== "before\nafter\n") throw Error("stdout truncated: " + JSON.stringify(text));
connection.close();
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        blob = blob,
    ));
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"split".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

#[test]
fn peer_close_before_eof_returns_the_captured_result() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::CloseBeforeEof(b"close-first\n"));
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const result = connection.exec("close-first");
if (result.status !== 0) throw Error("status");
let text = "";
for (let index = 0; index < result.stdout.length; index += 1) {{
    text += String.fromCharCode(result.stdout[index]);
}}
if (text !== "close-first\n") throw Error("stdout " + JSON.stringify(text));
connection.close();
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        blob = blob,
    ));
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"close-first".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

#[test]
fn output_over_the_limit_after_status_closes_the_connection() {
    let fixture = password_server(
        7,
        TEST_PASSWORD,
        Reply::BytesAfterStatus(8 * 1024 * 1024 + 1),
    );
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect, MAX_OUTPUT_BYTES }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 30000,
}});
try {{
    connection.exec("flood");
    throw Error("oversized output was accepted");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_LIMIT") throw error;
}}
if (!connection.closed) throw Error("the connection stayed usable after a fatal limit");
try {{
    connection.exec("echo again");
    throw Error("exec after a fatal limit succeeded");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_CLOSED") throw error;
}}
if (MAX_OUTPUT_BYTES !== 8388608) throw Error("limit constant");
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        blob = blob,
    ));
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"flood".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

#[test]
fn a_server_that_never_speaks_ssh_hits_the_connect_deadline() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let port = listener.local_addr().expect("address").port();
    let handle = thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        // The client sends an SSH identification and waits for ours. Keep
        // reading until the client gives up and closes its side; returning
        // early would drop the socket and turn the deadline into a reset.
        let mut buffer = [0_u8; 256];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
let failed = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {port},
        username: "fixture",
        password: "fixture",
        hostKey: {blob},
        connectTimeoutMs: 200,
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_TIMEOUT") throw error;
    failed = true;
}}
if (!failed) throw Error("the stalled server did not hit the connect deadline");
"#,
        port = port,
        blob = blob,
    ));
    handle.join().expect("stalled server thread");
}

#[test]
fn an_exec_that_never_answers_hits_the_exec_deadline() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Silent);
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 200,
}});
let failed = false;
try {{
    connection.exec("stall");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_TIMEOUT") throw error;
    failed = true;
}}
if (!failed) throw Error("the silent server did not hit the exec deadline");
if (!connection.closed) throw Error("a timed-out exec left the connection usable");
try {{
    connection.exec("again");
    throw Error("exec after the deadline succeeded");
}} catch (error) {{
    if (error.code !== "NEO_SSH_CLOSED") throw error;
}}
"#,
        port = fixture.port,
        password = js_string(TEST_PASSWORD),
        blob = blob,
    ));
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"stall".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

#[test]
fn a_tcp_connect_failure_is_reported() {
    // Bind and release a loopback port, then connect to it with a valid pin.
    // The validation of host, auth, and trust happens before the dial.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
let failed = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {port},
        username: "fixture",
        password: "fixture",
        hostKey: {blob},
        connectTimeoutMs: 2000,
    }});
}} catch (error) {{
    if (error.name !== "SshError") throw error;
    if (error.code !== "NEO_SSH_CONNECT" && error.code !== "NEO_SSH_TIMEOUT") {{
        throw Error("unexpected code: " + error.code);
    }}
    failed = true;
}}
if (!failed) throw Error("connect succeeded on a closed port");
"#,
        port = port,
        blob = blob,
    ));
}

#[test]
fn a_pkcs8_passphrase_is_refused_before_dialing() {
    use ed25519_dalek::pkcs8::EncodePrivateKey as _;

    let signing = ed25519_dalek::SigningKey::from_bytes(&[17; 32]);
    let der = signing.to_pkcs8_der().expect("pkcs8 der");
    let encoded = neotales_ssh::known_hosts::encode_base64(der.as_bytes());
    let pem =
        format!("-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n").into_bytes();
    let blob = js_bytes(&host_key_blob(7));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
let failed = false;
try {{
    // Port 1 would fail to connect if the key were not validated first.
    connect({{
        host: "127.0.0.1",
        port: 1,
        username: "fixture",
        password: undefined,
        privateKey: {private_key},
        passphrase: "unused",
        hostKey: {blob},
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "NEO_SSH_CREDENTIAL") throw error;
    failed = true;
}}
if (!failed) throw Error("a PKCS#8 key with a passphrase was accepted");
"#,
        private_key = js_bytes(&pem),
        blob = blob,
    ));
}

#[test]
fn a_private_key_pem_authenticates_and_runs_one_command() {
    use ed25519_dalek::pkcs8::EncodePrivateKey as _;

    let signing = ed25519_dalek::SigningKey::from_bytes(&[13; 32]);
    let public = signing.verifying_key().to_bytes();
    // The authorized-key blob is the SSH string encoding of ssh-ed25519.
    let mut key_blob = Vec::new();
    key_blob.extend_from_slice(&(11_u32).to_be_bytes());
    key_blob.extend_from_slice(b"ssh-ed25519");
    key_blob.extend_from_slice(&(32_u32).to_be_bytes());
    key_blob.extend_from_slice(&public);
    let der = signing.to_pkcs8_der().expect("pkcs8 der");
    let encoded = neotales_ssh::known_hosts::encode_base64(der.as_bytes());
    let pem =
        format!("-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n").into_bytes();

    let fixture = public_key_server(13, key_blob, Reply::Fixed(b"private key\n"));
    run_js(&format!(
        r#"
import {{ connect }} from "neo:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: undefined,
    privateKey: {private_key},
    hostKey: {host_key},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const result = connection.exec("whoami");
if (result.status !== 0) throw Error("status");
if (result.stdout.length !== 12) throw Error("output length");
connection.close();
"#,
        port = fixture.port,
        private_key = js_bytes(&pem),
        host_key = js_bytes(&host_key_blob(13)),
    ));
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"whoami".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

/// A loopback server that authorizes one Ed25519 public-key blob.
fn public_key_server(seed: u8, authorized: Vec<u8>, reply: Reply) -> Fixture {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let port = listener.local_addr().expect("local address").port();
    let (observed, observed_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_auth = Arc::clone(&attempts);
    let handle = thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
        let mut key_seed = [seed; 32];
        let config = ServerConfig::new(Ed25519Signer::from_seed(&mut key_seed));
        let Ok(mut transport) = config.accept(stream) else {
            return;
        };
        let authorized_for_verify = authorized.clone();
        let Ok(policy) =
            ServerAuthPolicy::new(4).and_then(|policy| policy.with_authorized_key(&authorized))
        else {
            return;
        };
        let policy = policy.with_public_key_verifier(move |_algorithm, key, _signed, _proof| {
            attempts_for_auth.fetch_add(1, Ordering::SeqCst);
            key == authorized_for_verify.as_slice()
        });
        if authenticate_server(&mut transport, &policy).is_err() {
            return;
        }
        let mut sessions = ServerSessions::with_policy(transport, move |request| {
            if let SessionRequest::Exec { command } = request {
                let _ = observed.send(command.to_vec());
                SessionDecision::accept(0)
            } else {
                SessionDecision::reject()
            }
        });
        serve(&mut sessions, reply);
    });
    Fixture {
        port,
        observed: observed_rx,
        attempts,
        handle,
    }
}
