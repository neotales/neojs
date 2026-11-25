//! Integration tests for `vaeljs-ssh-core` over loopback SSH servers.
//!
//! Uses deterministic loopback servers built with `neotales-ssh`'s public
//! server API. Host keys are derived from fixed seeds and passwords/keys are
//! synthetic test fixtures.

use std::io::Read;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

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
use vaeljs_ssh_core::{
    AuthConfig, ConnectOptions, HostTrust, MAX_COMMAND_BYTES, MAX_OUTPUT_BYTES, SshErrorCode,
    SshState, Zeroizing, connect, validate_host, validate_username,
};

const TEST_PASSWORD: &str = "fixture-password";

fn host_key_blob(seed: u8) -> Vec<u8> {
    let mut bytes = [seed; 32];
    Ed25519Signer::from_seed(&mut bytes)
        .public_key_blob()
        .expect("host key blob")
}

enum Reply {
    Fixed(&'static [u8]),
    Split(&'static [u8], &'static [u8]),
    CloseBeforeEof(&'static [u8]),
    BytesAfterStatus(usize),
    Silent,
}

struct Fixture {
    port: u16,
    observed: mpsc::Receiver<Vec<u8>>,
    attempts: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl Fixture {
    fn finish(self) -> (Vec<Vec<u8>>, usize) {
        self.handle.join().expect("server thread");
        (
            self.observed.try_iter().collect(),
            self.attempts.load(Ordering::SeqCst),
        )
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
                        let _ = sessions.connection_mut().send_close(session);
                    } else {
                        let _ = sessions.send_eof(session);
                        let _ = sessions.send_close(session);
                    }
                }
            }
            Ok(SessionEvent::Close { .. }) => {}
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

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
fn trust_and_auth_validation_enforces_exact_codes_and_redacts_debug() {
    assert_eq!(
        HostTrust::from_parts(None, None).unwrap_err().code(),
        SshErrorCode::HostKeyRequired
    );
    let blob = host_key_blob(7);
    assert_eq!(
        HostTrust::from_parts(Some(&blob), Some("127.0.0.1 ssh-ed25519 AAAA"))
            .unwrap_err()
            .code(),
        SshErrorCode::HostKeyRequired
    );
    assert_eq!(
        HostTrust::host_key(&[1, 2, 3]).unwrap_err().code(),
        SshErrorCode::HostKey
    );
    assert_eq!(
        HostTrust::known_hosts("not a known-hosts line")
            .unwrap_err()
            .code(),
        SshErrorCode::KnownHosts
    );

    let secret_pw = "UNIQUE-PASSWORD-7f3a9c";
    let secret_phrase = "UNIQUE-PASSPHRASE-2b8d41";
    let secret_key = vec![0xde, 0xad, 0xbe, 0xef];

    let auth = AuthConfig::from_parts(
        Some(Zeroizing::new(secret_pw.to_owned())),
        None,
        false,
        None,
    )
    .expect("password auth");
    let debug_pw = format!("{auth:?}");
    assert!(!debug_pw.contains(secret_pw));

    let auth_pk = AuthConfig::from_parts(
        None,
        Some(Zeroizing::new(secret_key.clone())),
        false,
        Some(Zeroizing::new(secret_phrase.to_owned())),
    )
    .expect("private key auth");
    let debug_pk = format!("{auth_pk:?}");
    assert!(!debug_pk.contains(secret_phrase));
    assert!(!debug_pk.contains("deadbeef"));

    assert_eq!(
        AuthConfig::from_parts(None, None, false, None)
            .unwrap_err()
            .code(),
        SshErrorCode::AuthMethod
    );
    assert_eq!(
        AuthConfig::from_parts(Some(Zeroizing::new(secret_pw.to_owned())), None, true, None,)
            .unwrap_err()
            .code(),
        SshErrorCode::AuthMethod
    );
    assert_eq!(
        AuthConfig::from_parts(
            Some(Zeroizing::new(secret_pw.to_owned())),
            None,
            false,
            Some(Zeroizing::new(secret_phrase.to_owned())),
        )
        .unwrap_err()
        .code(),
        SshErrorCode::Argument
    );
    assert_eq!(
        AuthConfig::from_parts(None, Some(Zeroizing::new(Vec::new())), false, None)
            .unwrap_err()
            .code(),
        SshErrorCode::Credential
    );
}

#[test]
fn wrong_host_key_and_changed_known_hosts_refuse_before_auth() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"never\n"));
    let options = ConnectOptions::new(
        "127.0.0.1",
        fixture.port,
        "fixture",
        AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
        HostTrust::host_key(&host_key_blob(8)).expect("valid pin blob"),
    )
    .with_connect_timeout(Duration::from_secs(5));
    let err = connect(options).err().expect("wrong host key must fail");
    assert_eq!(err.code(), SshErrorCode::HostKey);
    let (commands, attempts) = fixture.finish();
    assert!(commands.is_empty());
    assert_eq!(attempts, 0);

    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"never\n"));
    let wrong_blob = host_key_blob(8);
    let token = format_host_token("127.0.0.1", fixture.port);
    let entry = AppendEntry::plain(&token, &wrong_blob).expect("entry");
    let kh = format_append_entry(&entry).expect("known_hosts");
    let options = ConnectOptions::new(
        "127.0.0.1",
        fixture.port,
        "fixture",
        AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
        HostTrust::known_hosts(&kh).expect("valid known_hosts"),
    )
    .with_connect_timeout(Duration::from_secs(5));
    let err = connect(options)
        .err()
        .expect("changed known_hosts must fail");
    assert_eq!(err.code(), SshErrorCode::HostKey);
    let (commands, attempts) = fixture.finish();
    assert!(commands.is_empty());
    assert_eq!(attempts, 0);
}

#[test]
fn state_roundtrips_exec_reuses_handle_rejects_oversized_command_and_closes() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Split(b"before\n", b"after\n"));
    let mut state = SshState::new();
    let id = state
        .connect(
            ConnectOptions::new(
                "127.0.0.1",
                fixture.port,
                "fixture",
                AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
                HostTrust::host_key(&host_key_blob(7)).expect("pin"),
            )
            .with_connect_timeout(Duration::from_secs(5))
            .with_exec_timeout(Duration::from_secs(5)),
        )
        .expect("connect");
    assert!(state.is_open(id));

    let oversized = vec![b'x'; MAX_COMMAND_BYTES + 1];
    let err = state
        .exec(id, &oversized)
        .expect_err("oversized command refused");
    assert_eq!(err.code(), SshErrorCode::Limit);
    assert!(
        state.is_open(id),
        "oversized command must leave handle open"
    );

    let out1 = state.exec(id, b"first").expect("first exec");
    assert_eq!(out1.status, 0);
    assert_eq!(out1.stdout, b"before\nafter\n");

    let out2 = state.exec(id, b"second").expect("second exec");
    assert_eq!(out2.status, 0);
    assert_eq!(out2.stdout, b"before\nafter\n");

    assert!(state.close(id));
    assert!(!state.close(id));
    assert!(!state.is_open(id));
    assert_eq!(
        state.exec(id, b"after-close").unwrap_err().code(),
        SshErrorCode::Closed
    );

    let (commands, attempts) = fixture.finish();
    assert_eq!(commands, vec![b"first".to_vec(), b"second".to_vec()]);
    assert_eq!(attempts, 1);
}

#[test]
fn peer_close_before_eof_and_output_over_limit() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::CloseBeforeEof(b"close-first\n"));
    let mut conn = connect(ConnectOptions::new(
        "127.0.0.1",
        fixture.port,
        "fixture",
        AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
        HostTrust::host_key(&host_key_blob(7)).expect("pin"),
    ))
    .expect("connect");
    let out = conn.exec(b"close-first").expect("exec");
    assert_eq!(out.stdout, b"close-first\n");
    let close_start = Instant::now();
    conn.close();
    let (commands, attempts) = fixture.finish();
    assert!(
        close_start.elapsed() < Duration::from_secs(5),
        "conn.close() must close the underlying TCP fd before Connection drops"
    );
    assert!(conn.is_closed());
    assert_eq!(commands, vec![b"close-first".to_vec()]);
    assert_eq!(attempts, 1);

    let fixture = password_server(
        7,
        TEST_PASSWORD,
        Reply::BytesAfterStatus(MAX_OUTPUT_BYTES + 1),
    );
    let mut state = SshState::new();
    let id = state
        .connect(ConnectOptions::new(
            "127.0.0.1",
            fixture.port,
            "fixture",
            AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
            HostTrust::host_key(&host_key_blob(7)).expect("pin"),
        ))
        .expect("connect");
    let err = state.exec(id, b"flood").expect_err("oversized output");
    assert_eq!(err.code(), SshErrorCode::Limit);
    assert!(!state.is_open(id));
    let (commands, attempts) = fixture.finish();
    assert_eq!(commands, vec![b"flood".to_vec()]);
    assert_eq!(attempts, 1);
}

#[test]
fn connect_and_exec_deadlines_expire_with_timeout() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
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
    let err = connect(
        ConnectOptions::new(
            "127.0.0.1",
            port,
            "fixture",
            AuthConfig::Password(Zeroizing::new("fixture".to_owned())),
            HostTrust::host_key(&host_key_blob(7)).expect("pin"),
        )
        .with_connect_timeout(Duration::from_millis(200)),
    )
    .err()
    .expect("stalled handshake must time out");
    assert_eq!(err.code(), SshErrorCode::Timeout);
    handle.join().expect("stalled thread");

    let fixture = password_server(7, TEST_PASSWORD, Reply::Silent);
    let mut conn = connect(
        ConnectOptions::new(
            "127.0.0.1",
            fixture.port,
            "fixture",
            AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
            HostTrust::host_key(&host_key_blob(7)).expect("pin"),
        )
        .with_exec_timeout(Duration::from_millis(200)),
    )
    .expect("connect");
    let err = conn.exec(b"stall").expect_err("silent exec must time out");
    assert_eq!(err.code(), SshErrorCode::Timeout);
    let finish_start = Instant::now();
    let (commands, _) = fixture.finish();
    assert!(
        finish_start.elapsed() < Duration::from_secs(5),
        "failed conn.exec() must close the underlying TCP fd before Connection drops"
    );
    assert!(conn.is_closed());
    assert_eq!(commands, vec![b"stall".to_vec()]);
}

#[test]
fn pkcs8_pem_auth_and_passphrase_refusal() {
    use ed25519_dalek::pkcs8::EncodePrivateKey as _;

    let signing = ed25519_dalek::SigningKey::from_bytes(&[13; 32]);
    let public = signing.verifying_key().to_bytes();
    let mut key_blob = Vec::new();
    key_blob.extend_from_slice(&(11_u32).to_be_bytes());
    key_blob.extend_from_slice(b"ssh-ed25519");
    key_blob.extend_from_slice(&(32_u32).to_be_bytes());
    key_blob.extend_from_slice(&public);
    let der = signing.to_pkcs8_der().expect("pkcs8 der");
    let encoded = neotales_ssh::known_hosts::encode_base64(der.as_bytes());
    let pem =
        format!("-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n").into_bytes();

    let err = connect(ConnectOptions::new(
        "127.0.0.1",
        1,
        "fixture",
        AuthConfig::PrivateKey {
            bytes: Zeroizing::new(pem.clone()),
            passphrase: Some(Zeroizing::new("unused".to_owned())),
        },
        HostTrust::host_key(&host_key_blob(13)).expect("pin"),
    ))
    .err()
    .expect("pkcs8 with passphrase must fail before dialing");
    assert_eq!(err.code(), SshErrorCode::Credential);

    let fixture = public_key_server(13, key_blob, Reply::Fixed(b"private key\n"));
    let mut conn = connect(ConnectOptions::new(
        "127.0.0.1",
        fixture.port,
        "fixture",
        AuthConfig::PrivateKey {
            bytes: Zeroizing::new(pem),
            passphrase: None,
        },
        HostTrust::host_key(&host_key_blob(13)).expect("pin"),
    ))
    .expect("connect");
    let out = conn.exec(b"whoami").expect("exec");
    assert_eq!(out.status, 0);
    assert_eq!(out.stdout, b"private key\n");
    conn.close();
    let (commands, attempts) = fixture.finish();
    assert_eq!(commands, vec![b"whoami".to_vec()]);
    assert_eq!(attempts, 1);
}

#[test]
fn open_connection_cap_is_enforced_before_dialing() {
    let fixture = password_server(7, TEST_PASSWORD, Reply::Fixed(b"ok\n"));
    let mut state = SshState::with_max_connections(1);
    let id = state
        .connect(ConnectOptions::new(
            "127.0.0.1",
            fixture.port,
            "fixture",
            AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
            HostTrust::host_key(&host_key_blob(7)).expect("pin"),
        ))
        .expect("first connect");
    assert_eq!(state.len(), 1);
    assert!(!state.is_empty());

    let err = state
        .connect(ConnectOptions::new(
            "127.0.0.1",
            1,
            "fixture",
            AuthConfig::Password(Zeroizing::new(TEST_PASSWORD.to_owned())),
            HostTrust::host_key(&host_key_blob(7)).expect("pin"),
        ))
        .expect_err("second connect over cap must fail before dialing");
    assert_eq!(err.code(), SshErrorCode::Limit);
    state.close(id);
    assert!(state.is_empty());
    let _ = fixture.finish();
}

#[test]
fn host_and_username_validators_enforce_non_empty_and_character_rules() {
    assert_eq!(
        validate_host("").unwrap_err().code(),
        SshErrorCode::Argument
    );
    assert_eq!(
        validate_host("bad host").unwrap_err().code(),
        SshErrorCode::Argument
    );
    assert_eq!(
        validate_username("").unwrap_err().code(),
        SshErrorCode::Argument
    );
    assert_eq!(
        validate_username("bad\nuser").unwrap_err().code(),
        SshErrorCode::Argument
    );
    validate_host("127.0.0.1").expect("valid host");
    validate_username("fixture").expect("valid username");
}
