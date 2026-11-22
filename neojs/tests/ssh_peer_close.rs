//! Caller-facing regression for a peer that sends CLOSE before EOF.
//!
//! The failure is injected at the transport boundary by the off-by-default
//! `test-support` feature, so the test reaches the real `Connection::exec`,
//! host op removal, and JavaScript `closed` state without relying on a racy
//! TCP reset. The SSH server and credentials are local synthetic fixtures.

use std::fs;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use neojs_core::Runtime;
use neotales_ssh::ServerAuthPolicy;
use neotales_ssh::ServerConfig;
use neotales_ssh::ServerSessions;
use neotales_ssh::authenticate_server;
use neotales_ssh::hostkey::Ed25519Signer;
use neotales_ssh::session::SessionDecision;
use neotales_ssh::session::SessionEvent;
use neotales_ssh::session::SessionRequest;

const PASSWORD: &str = "peer-close-password";

struct Fixture {
    port: u16,
    host_key: Vec<u8>,
    attempts: Arc<AtomicUsize>,
    observed: mpsc::Receiver<Vec<u8>>,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let port = listener.local_addr().expect("address").port();
        let mut seed = [41; 32];
        let host_key = Ed25519Signer::from_seed(&mut seed)
            .public_key_blob()
            .expect("host key blob");
        let (observed, observed_rx) = mpsc::channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_auth = Arc::clone(&attempts);
        let handle = thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
            let mut seed = [41; 32];
            let config = ServerConfig::new(Ed25519Signer::from_seed(&mut seed));
            let Ok(mut transport) = config.accept(stream) else {
                return;
            };
            let Ok(policy) = ServerAuthPolicy::new(4).map(|policy| {
                policy.with_password_verifier(move |candidate: &str| {
                    attempts_for_auth.fetch_add(1, Ordering::SeqCst);
                    candidate == PASSWORD
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
            loop {
                match sessions.dispatch_one() {
                    Ok(SessionEvent::Request {
                        session,
                        accepted: true,
                        ..
                    }) => {
                        if sessions.send_data(session, b"peer-close\n").is_ok()
                            && sessions.send_exit_status(session).is_ok()
                        {
                            // CLOSE without EOF: the client must send the
                            // required reply, and the injected transport
                            // failure makes that reply fail.
                            let _ = sessions.connection_mut().send_close(session);
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            host_key,
            attempts,
            observed: observed_rx,
            handle,
        }
    }

    fn finish(self) -> (usize, Vec<Vec<u8>>) {
        self.handle.join().expect("server thread");
        (
            self.attempts.load(Ordering::SeqCst),
            self.observed.try_iter().collect(),
        )
    }
}

fn js_bytes(bytes: &[u8]) -> String {
    let items = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("Uint8Array.from([{items}])")
}

#[test]
fn peer_close_reply_failure_closes_the_caller_visible_connection() {
    neojs_core::test_support::fail_next_peer_close_reply();
    let fixture = Fixture::start();
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    let source = r#"
import { connect } from "neo:ssh";

const connection = connect({
    host: "127.0.0.1",
    port: __PORT__,
    username: "fixture",
    password: __PASSWORD__,
    hostKey: __HOST_KEY__,
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
});

let first = null;
try {
    connection.exec("peer-close-injected");
} catch (error) {
    first = error;
}
if (first === null) throw Error("exec succeeded despite the reply failure");
if (first.name !== "SshError" || first.code !== "NEO_SSH_IO") {
    throw Error("unexpected first code: " + first.code);
}
if (!connection.closed) throw Error("the connection was not marked closed");

let second = null;
try {
    connection.exec("again");
} catch (error) {
    second = error;
}
if (second === null) throw Error("exec succeeded after the connection closed");
if (second.name !== "SshError" || second.code !== "NEO_SSH_CLOSED") {
    throw Error("unexpected second code: " + second.code);
}
console.log("peer-close injected regression ok");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__PASSWORD__", &format!("\"{PASSWORD}\""))
    .replace("__HOST_KEY__", &js_bytes(&fixture.host_key));
    fs::write(&file, source).expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");

    let (attempts, commands) = fixture.finish();
    assert_eq!(attempts, 1);
    assert_eq!(commands, vec![b"peer-close-injected".to_vec()]);
}
