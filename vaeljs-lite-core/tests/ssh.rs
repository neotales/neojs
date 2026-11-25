//! Integration tests for the `vael:ssh` built-in through the QuickJS [`Runtime`].
//!
//! Every connection test uses a deterministic loopback SSH server built with
//! `neotales-ssh`'s public server API. Host keys are derived from fixed seeds
//! and passwords/keys are synthetic test fixtures. No real credential file is
//! read and no external host is contacted.

use vaeljs_lite_core::Runtime;

fn run_js(source: &str) {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source("ssh-test.js", source)
        .expect("module assertions");
}

#[cfg(not(feature = "vael-ssh"))]
#[test]
fn feature_off_rejects_vael_ssh_import_and_registry() {
    run_js(
        r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:ssh") || list.includes("ssh")) {
    throw new Error(`vael:ssh leaked into module list: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:ssh") !== undefined) {
    throw new Error("Vael.getBuiltInModule('vael:ssh') must be undefined when disabled");
}
let imported = false;
try {
    await import("vael:ssh");
    imported = true;
} catch {
    // Expected: vael:ssh is not registered when vael-ssh is disabled.
}
if (imported) {
    throw new Error("import('vael:ssh') succeeded while vael-ssh feature was disabled");
}
"#,
    );
}

#[cfg(feature = "vael-ssh")]
mod enabled {
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use neotales_ssh::channel::{ChannelError, ChannelTransport, WindowError};
    use neotales_ssh::hostkey::Ed25519Signer;
    use neotales_ssh::known_hosts::{
        AppendEntry, decode_base64, encode_base64, format_append_entry, format_host_token,
    };
    use neotales_ssh::session::{SessionDecision, SessionEvent, SessionRequest};
    use neotales_ssh::{ServerAuthPolicy, ServerConfig, ServerSessions, authenticate_server};
    use vaeljs_lite_core::Runtime;

    use super::run_js;

    const TEST_PASSWORD: &str = "fixture-password";
    const UNICODE_PASSWORD: &str = "pässwörd-🔑";
    const ENCRYPTED_OPENSSH_PASSPHRASE: &str = "fixture-passphrase";
    const ENCRYPTED_OPENSSH_PEM: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----\n\
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABCFrgAgM1\n\
8C9lboHxiRCICyAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAICtF9/eVIVopc7o9\n\
uC/WrVSQPZgU0dtckG8HMkca1Bu+AAAAkBYbH3V2HvfWaGgAbf8nA9zfEhlk+4y+3Z8DrQ\n\
aLjGto5twZwvKE5BaBtohpN9wU8GZJv5ZoM05p9RkQ6eIgUxqseBmKwEkxjU4GQdev9k+4\n\
n3NzaNISGmYBstvbAAIDCPoLZAAMDwWZLbP2Z6b8z7mXv6izj/26ZxXbGPbHFRKZAQd+IW\n\
8jT/i3+CNkKRfGcA==\n\
-----END OPENSSH PRIVATE KEY-----\n";
    const ENCRYPTED_OPENSSH_PUB_B64: &str =
        "AAAAC3NzaC1lZDI1NTE5AAAAICtF9/eVIVopc7o9uC/WrVSQPZgU0dtckG8HMkca1Bu+";

    fn host_key_blob(seed: u8) -> Vec<u8> {
        let mut bytes = [seed; 32];
        Ed25519Signer::from_seed(&mut bytes)
            .public_key_blob()
            .expect("host key blob")
    }

    fn pkcs8_ed25519_pem(seed: u8) -> Vec<u8> {
        let mut der = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        der.extend_from_slice(&[seed; 32]);
        let encoded = encode_base64(&der);
        format!("-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n").into_bytes()
    }

    fn js_bytes(bytes: &[u8]) -> String {
        let items = bytes
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!("Uint8Array.from([{items}])")
    }

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

    enum Reply {
        Fixed(&'static [u8], u32),
        BytesAfterStatus(usize),
        Silent,
    }

    struct Finished {
        commands: Vec<Vec<u8>>,
        attempts: usize,
    }

    struct Fixture {
        port: u16,
        observed: mpsc::Receiver<Vec<u8>>,
        attempts: Arc<AtomicUsize>,
        handle: JoinHandle<()>,
    }

    impl Fixture {
        fn finish(self) -> Finished {
            self.handle.join().expect("server thread");
            Finished {
                commands: self.observed.try_iter().collect(),
                attempts: self.attempts.load(Ordering::SeqCst),
            }
        }
    }

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
            let exit_code = match &reply {
                Reply::Fixed(_, status) => *status,
                _ => 0,
            };
            let mut sessions = ServerSessions::with_policy(transport, move |request| {
                if let SessionRequest::Exec { command } = request {
                    let _ = observed.send(command.to_vec());
                    SessionDecision::accept(exit_code)
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
            let policy =
                policy.with_public_key_verifier(move |_algorithm, key, _signed, _proof| {
                    attempts_for_auth.fetch_add(1, Ordering::SeqCst);
                    key == authorized_for_verify.as_slice()
                });
            if authenticate_server(&mut transport, &policy).is_err() {
                return;
            }
            let exit_code = match &reply {
                Reply::Fixed(_, status) => *status,
                _ => 0,
            };
            let mut sessions = ServerSessions::with_policy(transport, move |request| {
                if let SessionRequest::Exec { command } = request {
                    let _ = observed.send(command.to_vec());
                    SessionDecision::accept(exit_code)
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

    fn serve<T: ChannelTransport>(sessions: &mut ServerSessions<T>, reply: Reply) {
        loop {
            match sessions.dispatch_one() {
                Ok(SessionEvent::Request {
                    session,
                    accepted: true,
                    ..
                }) => {
                    let sent = match reply {
                        Reply::Fixed(bytes, _) => {
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
                        let _ = sessions.send_eof(session);
                        let _ = sessions.send_close(session);
                    }
                }
                Ok(SessionEvent::Close { .. }) | Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    fn send_bytes_after_status<T: ChannelTransport>(
        sessions: &mut ServerSessions<T>,
        session: u32,
        total: usize,
    ) -> bool {
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
    fn module_identity_constants_and_no_capability_leak() {
        run_js(
            r#"
import { connect } from "vael:ssh";
import * as aliased from "vael:ssh";

const registry = Vael.getBuiltInModule("vael:ssh");
if (typeof registry !== "object" || registry === null) throw Error("missing registry module");
if (Vael.getBuiltInModule("vael:ssh") !== registry) throw Error("registry lookup is not cached");
if (!Object.isFrozen(registry)) throw Error("registry module is not frozen");
if (registry.connect !== connect) throw Error("import is not the registry function");
if (aliased.connect !== connect) throw Error("vael: import is not the registry function");
const registryNames = Object.keys(registry).sort().join(",");
const moduleNames = Object.keys(aliased).sort().join(",");
if (registryNames !== moduleNames) throw Error("export list drifted: " + registryNames + " / " + moduleNames);
if (registry.MAX_COMMAND_BYTES !== 65536) throw Error("MAX_COMMAND_BYTES");
if (registry.MAX_OUTPUT_BYTES !== 8388608) throw Error("MAX_OUTPUT_BYTES");
if (registry.DEFAULT_CONNECT_TIMEOUT_MS !== 15000) throw Error("DEFAULT_CONNECT_TIMEOUT_MS");
if (registry.DEFAULT_EXEC_TIMEOUT_MS !== 30000) throw Error("DEFAULT_EXEC_TIMEOUT_MS");
if (typeof globalThis.connect === "function") throw Error("connect leaked as a global");
if (typeof globalThis.env !== "undefined") throw Error("environment capability leaked");
"#,
        );
    }

    #[test]
    fn trust_is_mandatory_and_never_defaults() {
        let blob = js_bytes(&host_key_blob(7));
        let known_hosts = js_string("[127.0.0.1]:22 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample");
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";

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

expectCode("VAEL_SSH_HOST_KEY_REQUIRED", () => connect({{
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: "fixture",
}}));
expectCode("VAEL_SSH_HOST_KEY_REQUIRED", () => connect({{
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: "fixture",
    hostKey: {blob},
    knownHosts: {known_hosts},
}}));
"#,
        ));
    }

    #[test]
    fn argument_validation_is_strict_and_does_not_connect() {
        let blob = js_bytes(&host_key_blob(7));
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";
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
expectType(() => connect(undefined), "undefined options");
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

expectCode("VAEL_SSH_ARGUMENT", () => connect({{ ...base, host: "" }}), "empty host");
expectCode("VAEL_SSH_ARGUMENT", () => connect({{ ...base, host: "bad host" }}), "whitespace host");
expectCode("VAEL_SSH_ARGUMENT", () => connect({{ ...base, username: "" }}), "empty username");
expectCode("VAEL_SSH_AUTH_METHOD", () => connect({{ ...base, password: undefined }}), "no auth");
expectCode("VAEL_SSH_AUTH_METHOD", () => connect({{
    ...base,
    privateKey: Uint8Array.from([1, 2, 3]),
}}), "two auth methods");
expectCode("VAEL_SSH_ARGUMENT", () => connect({{ ...base, passphrase: "x" }}), "passphrase without key");
expectCode("VAEL_SSH_CREDENTIAL", () => connect({{
    ...base,
    password: undefined,
    privateKey: new Uint8Array(0),
}}), "empty key");
expectCode("VAEL_SSH_ARGUMENT", () => connect({{ ...base, password: "\uD800" }}), "lone surrogate password");
expectType(() => connect({{ ...base, agent: 1 }}), "agent type");
expectCode("VAEL_SSH_AUTH_METHOD", () => connect({{ ...base, agent: true }}), "two auth methods");
expectCode("VAEL_SSH_ARGUMENT", () => connect({{
    ...base,
    password: undefined,
    agent: true,
    passphrase: "unused",
}}), "agent passphrase");
expectCode("VAEL_SSH_AUTH_METHOD", () => connect({{ ...base, password: undefined, agent: false }}), "agent false alone");
expectCode("VAEL_SSH_HOST_KEY", () => connect({{ ...base, hostKey: Uint8Array.from([1, 2, 3]) }}), "bad pin");
expectCode("VAEL_SSH_KNOWN_HOSTS", () => connect({{
    ...base,
    hostKey: undefined,
    knownHosts: "not a known-hosts line",
}}), "bad known hosts");
"#,
        ));
    }

    #[test]
    fn errors_do_not_echo_passwords_passphrases_or_key_bytes() {
        run_js(
            r#"
import { connect } from "vael:ssh";

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
}), "VAEL_SSH_HOST_KEY_REQUIRED");

noSecret(() => connect({
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password: undefined,
    privateKey: key,
    passphrase,
    knownHosts: "not a known-hosts line",
}), "VAEL_SSH_KNOWN_HOSTS");

noSecret(() => connect({
    host: "127.0.0.1",
    port: 1,
    username: "fixture",
    password,
    privateKey: key,
}), "VAEL_SSH_AUTH_METHOD");
"#,
        );
    }

    #[test]
    fn host_key_mismatch_and_changed_known_hosts_refuse_before_auth() {
        let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"must not run\n", 0));
        let wrong = js_bytes(&host_key_blob(8));
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";
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
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_HOST_KEY") throw error;
    failed = true;
}}
if (!failed) throw Error("connect accepted an untrusted host key");
"#,
            port = fixture.port,
            password = js_string(TEST_PASSWORD),
            wrong = wrong,
        ));
        let finished = fixture.finish();
        assert!(finished.commands.is_empty());
        assert_eq!(finished.attempts, 0);

        let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"must not run\n", 0));
        let wrong_blob = host_key_blob(8);
        let token = format_host_token("127.0.0.1", fixture.port);
        let entry = AppendEntry::plain(&token, &wrong_blob).expect("known-hosts entry");
        let known_hosts = format_append_entry(&entry).expect("known-hosts text");
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";
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
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_HOST_KEY") throw error;
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
        assert_eq!(finished.attempts, 0);
    }

    #[test]
    fn password_auth_exec_stdout_status_and_close_idempotency() {
        let fixture = password_server(7, UNICODE_PASSWORD, Reply::Fixed(b"fixture stdout\n", 42));
        let blob = js_bytes(&host_key_blob(7));
        run_js(&format!(
            r#"
import {{ connect, MAX_COMMAND_BYTES, MAX_OUTPUT_BYTES }} from "vael:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
if (!Object.isFrozen(connection)) throw Error("connection must be frozen");
if (connection.closed !== false) throw Error("new connection is closed");
const closedDesc = Object.getOwnPropertyDescriptor(connection, "closed");
if (!closedDesc || typeof closedDesc.get !== "function") throw Error("closed must be a getter");
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
if (first.status !== 42) throw Error("unexpected status: " + first.status);

const second = connection.exec(Uint8Array.from([0x69, 0x64]));
if (text(second.stdout) !== "fixture stdout\n") throw Error("second stdout");
if (second.status !== 42) throw Error("second status");

// Invalid exec arity or type does not close an open connection.
let badArity = false;
try {{ connection.exec(); }} catch (e) {{ badArity = e instanceof TypeError; }}
if (!badArity || connection.closed) throw Error("bad arity handling");

let badCmdType = false;
try {{ connection.exec(123); }} catch (e) {{ badCmdType = e instanceof TypeError; }}
if (!badCmdType || connection.closed) throw Error("bad command type handling");

connection.close();
connection.close();
if (!connection.closed) throw Error("close did not mark the connection closed");
try {{
    connection.exec("echo again");
    throw Error("exec after close succeeded");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_CLOSED") throw error;
}}
"#,
            port = fixture.port,
            password = js_string(UNICODE_PASSWORD),
            blob = blob,
        ));
        let finished = fixture.finish();
        assert_eq!(
            finished.commands,
            vec![b"echo hello".to_vec(), b"id".to_vec()]
        );
        assert_eq!(finished.attempts, 1);
    }

    #[test]
    fn explicit_known_hosts_text_is_accepted() {
        let fixture = password_server(11, TEST_PASSWORD, Reply::Fixed(b"known-hosts\n", 0));
        let blob = host_key_blob(11);
        let token = format_host_token("127.0.0.1", fixture.port);
        let entry = AppendEntry::plain(&token, &blob).expect("known-hosts entry");
        let known_hosts = format_append_entry(&entry).expect("known-hosts text");
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";
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
    fn ed25519_private_key_and_passphrase_auth() {
        let authorized = decode_base64(ENCRYPTED_OPENSSH_PUB_B64).expect("decode pub blob");
        let fixture = public_key_server(13, authorized, Reply::Fixed(b"openssh-key\n", 0));
        let pem = js_bytes(ENCRYPTED_OPENSSH_PEM.as_bytes());
        let host_key = js_bytes(&host_key_blob(13));
        let pkcs8 = js_bytes(&pkcs8_ed25519_pem(17));

        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";

// Wrong passphrase for encrypted OpenSSH key fails locally before dialing.
let wrongPass = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: 1,
        username: "fixture",
        privateKey: {pem},
        passphrase: "wrong-passphrase",
        hostKey: {host_key},
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_CREDENTIAL") throw error;
    wrongPass = true;
}}
if (!wrongPass) throw Error("wrong passphrase accepted");

// PKCS#8 key with a passphrase fails locally before dialing.
let pkcs8Pass = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: 1,
        username: "fixture",
        privateKey: {pkcs8},
        passphrase: "unused",
        hostKey: {host_key},
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_CREDENTIAL") throw error;
    pkcs8Pass = true;
}}
if (!pkcs8Pass) throw Error("pkcs8 with passphrase accepted");

// Encrypted OpenSSH Ed25519 key + correct passphrase connects and executes.
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    privateKey: {pem},
    passphrase: {passphrase},
    hostKey: {host_key},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const result = connection.exec("whoami");
if (result.status !== 0) throw Error("status");
if (result.stdout.length !== 12) throw Error("stdout length");
connection.close();
"#,
            port = fixture.port,
            pem = pem,
            pkcs8 = pkcs8,
            host_key = host_key,
            passphrase = js_string(ENCRYPTED_OPENSSH_PASSPHRASE),
        ));
        let finished = fixture.finish();
        assert_eq!(finished.commands, vec![b"whoami".to_vec()]);
        assert_eq!(finished.attempts, 1);
    }

    #[test]
    fn rab_uint8_array_keys_and_commands_handle_resize_and_oob() {
        let authorized = decode_base64(ENCRYPTED_OPENSSH_PUB_B64).expect("decode pub blob");
        let fixture = public_key_server(13, authorized, Reply::Fixed(b"rab-ok\n", 0));
        let key_bytes = js_bytes(ENCRYPTED_OPENSSH_PEM.as_bytes());
        let host_bytes = js_bytes(&host_key_blob(13));

        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";

const hostBytes = {host_bytes};
const hostRab = new ArrayBuffer(16, {{ maxByteLength: 128 }});
const hostView = new Uint8Array(hostRab);
hostRab.resize(hostBytes.length);
hostView.set(hostBytes);

const keyBytes = {key_bytes};
const keyRab = new ArrayBuffer(16, {{ maxByteLength: 1024 }});
const keyView = new Uint8Array(keyRab);
keyRab.resize(keyBytes.length);
keyView.set(keyBytes);

const cmdRab = new ArrayBuffer(16, {{ maxByteLength: 64 }});
const cmdView = new Uint8Array(cmdRab);
cmdRab.resize(6);
cmdView.set([0x75, 0x6e, 0x61, 0x6d, 0x65, 0x0a]);
// Shrink RAB to verify live length is respected.
cmdRab.resize(5);

// Out-of-bounds RAB view must throw TypeError without dialing.
const oobRab = new ArrayBuffer(8, {{ maxByteLength: 16 }});
const oobView = new Uint8Array(oobRab, 4, 4);
oobRab.resize(2);
let oobThrew = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: 1,
        username: "fixture",
        password: "fixture",
        hostKey: oobView,
    }});
}} catch (error) {{
    if (!(error instanceof TypeError)) throw error;
    oobThrew = true;
}}
if (!oobThrew) throw Error("OOB hostKey RAB view was accepted");

const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    privateKey: keyView,
    passphrase: {passphrase},
    hostKey: hostView,
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const result = connection.exec(cmdView);
if (result.status !== 0 || result.stdout.length !== 7) throw Error("rab exec failed");
connection.close();
"#,
            port = fixture.port,
            host_bytes = host_bytes,
            key_bytes = key_bytes,
            passphrase = js_string(ENCRYPTED_OPENSSH_PASSPHRASE),
        ));
        let finished = fixture.finish();
        assert_eq!(finished.commands, vec![b"uname".to_vec()]);
        assert_eq!(finished.attempts, 1);
    }

    #[test]
    fn oversized_command_and_oversized_output_follow_close_contract() {
        let fixture = password_server(
            7,
            TEST_PASSWORD,
            Reply::BytesAfterStatus(8 * 1024 * 1024 + 1),
        );
        let blob = js_bytes(&host_key_blob(7));
        run_js(&format!(
            r#"
import {{ connect, MAX_COMMAND_BYTES, MAX_OUTPUT_BYTES }} from "vael:ssh";
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
    connection.exec("x".repeat(MAX_COMMAND_BYTES + 1));
    throw Error("oversized command was accepted");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_LIMIT") throw error;
}}
if (connection.closed) throw Error("oversized command closed the connection");

try {{
    connection.exec("flood");
    throw Error("oversized output was accepted");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_LIMIT") throw error;
}}
if (!connection.closed) throw Error("oversized output did not close the connection");
try {{
    connection.exec("after-limit");
    throw Error("exec after fatal limit succeeded");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_CLOSED") throw error;
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
    fn hostile_error_and_object_prototypes_cannot_hijack_ssh_errors() {
        let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"ok\n", 0));
        let blob = js_bytes(&host_key_blob(7));
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";

let setterCalls = 0;
Object.defineProperty(Error.prototype, "message", {{
    configurable: true,
    get() {{ return ""; }},
    set() {{
        setterCalls += 1;
        throw new Error("hostile Error.prototype.message setter");
    }},
}});
Object.defineProperty(Object.prototype, "code", {{
    configurable: true,
    set() {{
        setterCalls += 1;
        throw new Error("hostile Object.prototype.code setter");
    }},
}});
Object.defineProperty(Object.prototype, "host", {{
    configurable: true,
    get() {{
        setterCalls += 1;
        return "evil.example";
    }},
}});
const origFreeze = Object.freeze;
Object.freeze = () => {{
    setterCalls += 1;
    throw new Error("hijacked Object.freeze");
}};
origFreeze(Error.prototype);

let caught;
try {{
    connect({{
        host: "127.0.0.1",
        port: 1,
        username: "fixture",
        password: "fixture",
    }});
}} catch (error) {{
    caught = error;
}}
if (setterCalls !== 0) throw Error(`prototype setter ran ${{setterCalls}} times`);
if (!(caught instanceof Error)) throw Error("expected Error instance");
if (caught.name !== "SshError" || caught.code !== "VAEL_SSH_HOST_KEY_REQUIRED") {{
    throw Error(`unexpected error: ${{caught.name}} / ${{caught.code}}`);
}}
if (Object.getOwnPropertyDescriptor(caught, "code")?.value !== "VAEL_SSH_HOST_KEY_REQUIRED") {{
    throw Error("missing own code descriptor");
}}

const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
if (!Object.isFrozen(connection)) throw Error("connection not frozen under hostile Object.freeze");
if (setterCalls !== 0) throw Error(`hostile hooks fired: ${{setterCalls}}`);
connection.close();
"#,
            port = fixture.port,
            password = js_string(TEST_PASSWORD),
            blob = blob,
        ));
        let finished = fixture.finish();
        assert_eq!(finished.attempts, 1);
    }

    #[test]
    fn dropping_runtime_closes_open_ssh_connections() {
        let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"ok\n", 0));
        let blob = js_bytes(&host_key_blob(7));
        let js = format!(
            r#"
import {{ connect }} from "vael:ssh";
const connection = connect({{
    host: "127.0.0.1",
    port: {port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
}});
const res = connection.exec("keep-open");
if (res.status !== 0) throw Error("status");
// Deliberately omit connection.close() so Runtime::drop must drop SshState.
"#,
            port = fixture.port,
            password = js_string(TEST_PASSWORD),
            blob = blob,
        );
        {
            let mut runtime = Runtime::new().expect("runtime");
            runtime.run_source("leak-check.js", &js).expect("run");
        }
        let start = Instant::now();
        let finished = fixture.finish();
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "dropping Runtime did not close open SSH connection promptly"
        );
        assert_eq!(finished.commands, vec![b"keep-open".to_vec()]);
        assert_eq!(finished.attempts, 1);
    }

    #[test]
    fn connect_and_exec_timeouts_and_closed_port_errors() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let closed_port = listener.local_addr().expect("addr").port();
        drop(listener);

        let stall_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind stall");
        let stall_port = stall_listener.local_addr().expect("addr").port();
        let stall_handle = thread::spawn(move || {
            let Ok((mut stream, _)) = stall_listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0_u8; 256];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
            }
        });

        let fixture = password_server(7, TEST_PASSWORD, Reply::Silent);
        let blob = js_bytes(&host_key_blob(7));
        run_js(&format!(
            r#"
import {{ connect }} from "vael:ssh";

let closedFailed = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {closed_port},
        username: "fixture",
        password: "fixture",
        hostKey: {blob},
        connectTimeoutMs: 2000,
    }});
}} catch (error) {{
    if (error.name !== "SshError") throw error;
    if (error.code !== "VAEL_SSH_CONNECT" && error.code !== "VAEL_SSH_TIMEOUT") throw error;
    closedFailed = true;
}}
if (!closedFailed) throw Error("closed port connect succeeded");

let connectTimedOut = false;
try {{
    connect({{
        host: "127.0.0.1",
        port: {stall_port},
        username: "fixture",
        password: "fixture",
        hostKey: {blob},
        connectTimeoutMs: 200,
    }});
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_TIMEOUT") throw error;
    connectTimedOut = true;
}}
if (!connectTimedOut) throw Error("stalled handshake did not time out");

const connection = connect({{
    host: "127.0.0.1",
    port: {exec_port},
    username: "fixture",
    password: {password},
    hostKey: {blob},
    connectTimeoutMs: 5000,
    execTimeoutMs: 200,
}});
let execTimedOut = false;
try {{
    connection.exec("stall");
}} catch (error) {{
    if (error.name !== "SshError" || error.code !== "VAEL_SSH_TIMEOUT") throw error;
    execTimedOut = true;
}}
if (!execTimedOut || !connection.closed) throw Error("silent exec did not time out and close");
"#,
            closed_port = closed_port,
            stall_port = stall_port,
            exec_port = fixture.port,
            password = js_string(TEST_PASSWORD),
            blob = blob,
        ));
        stall_handle.join().expect("stall thread");
        let finished = fixture.finish();
        assert_eq!(finished.commands, vec![b"stall".to_vec()]);
        assert_eq!(finished.attempts, 1);
    }
}
