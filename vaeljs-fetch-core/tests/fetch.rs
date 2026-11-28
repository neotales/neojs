use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::tempdir;
use vaeljs_fetch_core::{
    DEFAULT_USER_AGENT, DownloadOptions, DownloadProgress, FetchBody, FetchError, FetchErrorCode,
    FetchOptions, FetchRequest, Zeroizing, download_sync, download_sync_cancellable,
    fetch_request_sync, fetch_sync,
};

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct ScriptedReply {
    status: u16,
    reason: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Duration,
    omit_content_length: bool,
}

impl ScriptedReply {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            reason: "OK",
            headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
            body: body.into(),
            delay: Duration::ZERO,
            omit_content_length: false,
        }
    }

    fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            reason: "OK",
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.into(),
            delay: Duration::ZERO,
            omit_content_length: false,
        }
    }

    fn redirect(status: u16, location: impl Into<String>) -> Self {
        Self {
            status,
            reason: "Redirect",
            headers: vec![("Location".to_owned(), location.into())],
            body: Vec::new(),
            delay: Duration::ZERO,
            omit_content_length: false,
        }
    }
}

struct TestServer {
    addr: SocketAddr,
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn spawn<F>(max_requests: usize, handler: F) -> Self
    where
        F: Fn(usize, &CapturedRequest) -> ScriptedReply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_bg = Arc::clone(&captured);

        let handle = thread::spawn(move || {
            for idx in 0..max_requests {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                let req = read_http_request(&mut stream);
                let reply = handler(idx, &req);
                captured_bg.lock().unwrap().push(req);
                if !reply.delay.is_zero() {
                    thread::sleep(reply.delay);
                }
                let _ = write_http_reply(&mut stream, &reply);
            }
        });

        Self {
            addr,
            captured,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.captured.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // Wake blocked accept if fewer requests were sent than `max_requests`.
            let _ = TcpStream::connect(self.addr);
            let _ = handle.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    let _ = reader.read_line(&mut request_line);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();

    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).unwrap_or(0);
        if bytes == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let key = name.trim().to_ascii_lowercase();
            let val = value.trim().to_owned();
            if key == "content-length" {
                content_length = val.parse().unwrap_or(0);
            }
            headers.insert(key, val);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    CapturedRequest {
        method,
        target,
        headers,
        body,
    }
}

fn write_http_reply(stream: &mut TcpStream, reply: &ScriptedReply) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reply.reason);
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !reply.omit_content_length {
        head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[test]
fn ssrf_and_url_policy_reject_private_targets_and_credentials() {
    let opts = FetchOptions::default();

    let ssrf_urls = [
        "http://127.0.0.1/secret",
        "http://[::1]/secret",
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.1/internal",
        "http://100.64.0.1/cgnat",
        "http://[::ffff:127.0.0.1]/mapped",
        "http://[64:ff9b::7f00:1]/nat64",
        "http://localhost:8080/admin",
        "http://0.0.0.0/",
        "http://172.16.0.5/",
        "http://192.168.1.10/",
        "http://[fe80::1]/",
        "http://[fc00::1]/",
        "http://255.255.255.255/",
    ];
    for url in ssrf_urls {
        let err = fetch_sync(url, &opts).unwrap_err();
        assert_eq!(
            err.code(),
            FetchErrorCode::SsrfBlocked,
            "expected SsrfBlocked for {url}, got {err:?}"
        );
    }

    let invalid_urls = [
        "file:///etc/passwd",
        "ftp://example.com/resource",
        "https://alice:s3cr3t-pw@example.com/private?secret=xyz",
        "not-a-url",
    ];
    for url in invalid_urls {
        let err = fetch_sync(url, &opts).unwrap_err();
        assert_eq!(
            err.code(),
            FetchErrorCode::InvalidUrl,
            "expected InvalidUrl for {url}, got {err:?}"
        );
        let msg = format!("{err} {err:?}");
        assert!(!msg.contains("alice"));
        assert!(!msg.contains("s3cr3t-pw"));
        assert!(!msg.contains("secret=xyz"));
    }
}

#[test]
fn fetch_get_query_headers_and_bearer_redaction() {
    let server = TestServer::spawn(1, |_, _| {
        let mut reply = ScriptedReply::ok(b"hello vael");
        reply.headers.push(("X-Multi".to_owned(), "one".to_owned()));
        reply.headers.push(("X-Multi".to_owned(), "two".to_owned()));
        reply
    });

    let opts = FetchOptions {
        allow_private_ips: true,
        query: vec![
            ("q".to_owned(), "hello world".to_owned()),
            ("api_key".to_owned(), "top-secret-query".to_owned()),
        ],
        headers: vec![
            ("X-Trace".to_owned(), "trace-42".to_owned()),
            (
                "Cookie".to_owned(),
                "session=super-secret-cookie".to_owned(),
            ),
        ],
        bearer_token: Some(Zeroizing::new("bearer-secret-999".to_owned())),
        ..Default::default()
    };

    let debug_repr = format!("{opts:?}");
    assert!(!debug_repr.contains("bearer-secret-999"));
    assert!(!debug_repr.contains("super-secret-cookie"));
    assert!(!debug_repr.contains("top-secret-query"));
    assert!(debug_repr.contains("[REDACTED]"));

    let resp = fetch_sync(&server.url("/items"), &opts).unwrap();
    assert_eq!(resp.status, 200);
    assert!(resp.ok);
    assert!(!resp.redirected);
    assert_eq!(resp.text().unwrap(), "hello vael");
    assert_eq!(resp.header("content-type"), Some("text/plain"));
    assert_eq!(resp.header_values("x-multi"), vec!["one", "two"]);

    let captured = server.requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, "GET");
    assert_eq!(
        captured[0].target,
        "/items?q=hello+world&api_key=top-secret-query"
    );
    assert_eq!(
        captured[0].headers.get("authorization").map(String::as_str),
        Some("Bearer bearer-secret-999")
    );
    assert_eq!(
        captured[0].headers.get("user-agent").map(String::as_str),
        Some("vaeljs/0.0.0-alpha.1")
    );
    assert_eq!(
        captured[0].headers.get("x-trace").map(String::as_str),
        Some("trace-42")
    );
}

#[test]
fn explicit_authorization_header_takes_precedence_over_bearer_token() {
    let server = TestServer::spawn(1, |_, _| ScriptedReply::ok(b"ok"));
    let opts = FetchOptions {
        allow_private_ips: true,
        headers: vec![("authorization".to_owned(), "Basic dXNlcjpwYXNz".to_owned())],
        bearer_token: Some(Zeroizing::new("ignored-token".to_owned())),
        ..Default::default()
    };

    let resp = fetch_sync(&server.url("/auth"), &opts).unwrap();
    assert_eq!(resp.status, 200);
    let captured = server.requests();
    assert_eq!(
        captured[0].headers.get("authorization").map(String::as_str),
        Some("Basic dXNlcjpwYXNz")
    );
}

#[test]
fn post_json_sets_content_type_and_parses_json_response() {
    let server = TestServer::spawn(1, |_, req| {
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.headers.get("content-type").map(String::as_str),
            Some("application/json; charset=utf-8")
        );
        let parsed: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(parsed["hello"], "world");
        ScriptedReply::json(br#"{"accepted":true,"count":3}"#)
    });

    let opts = FetchOptions {
        method: "post".to_owned(),
        allow_private_ips: true,
        body: FetchBody::Json(serde_json::json!({ "hello": "world" })),
        ..Default::default()
    };

    let resp = fetch_request_sync(&FetchRequest::new(server.url("/submit"), opts)).unwrap();
    assert_eq!(resp.status, 200);
    let val = resp.json_value().unwrap();
    assert_eq!(val["accepted"], true);
    assert_eq!(val["count"], 3);
}

#[test]
fn cross_origin_redirect_strips_authorization_proxy_auth_and_cookie() {
    let target_server = TestServer::spawn(1, |_, req| {
        assert!(!req.headers.contains_key("authorization"));
        assert!(!req.headers.contains_key("proxy-authorization"));
        assert!(!req.headers.contains_key("cookie"));
        assert_eq!(
            req.headers.get("x-keep").map(String::as_str),
            Some("still-here")
        );
        ScriptedReply::ok(b"final-destination")
    });

    let target_url = target_server.url("/final");
    let origin_server = TestServer::spawn(1, move |_, req| {
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer cross-origin-secret")
        );
        assert_eq!(
            req.headers.get("proxy-authorization").map(String::as_str),
            Some("Basic cHJveHk=")
        );
        assert_eq!(
            req.headers.get("cookie").map(String::as_str),
            Some("sid=secret-cookie")
        );
        ScriptedReply::redirect(302, target_url.clone())
    });

    let opts = FetchOptions {
        allow_private_ips: true,
        bearer_token: Some(Zeroizing::new("cross-origin-secret".to_owned())),
        headers: vec![
            (
                "Proxy-Authorization".to_owned(),
                "Basic cHJveHk=".to_owned(),
            ),
            ("Cookie".to_owned(), "sid=secret-cookie".to_owned()),
            ("X-Keep".to_owned(), "still-here".to_owned()),
        ],
        ..Default::default()
    };

    let resp = fetch_sync(&origin_server.url("/start"), &opts).unwrap();
    assert!(resp.redirected);
    assert_eq!(resp.url, target_server.url("/final"));
    assert_eq!(resp.text().unwrap(), "final-destination");
}

#[test]
fn redirect_method_and_body_rewrites_follow_fetch_semantics() {
    // 303 rewrites POST -> GET and drops body + Content-Type + Content-Length.
    let server_303 = TestServer::spawn(2, |idx, req| match idx {
        0 => {
            assert_eq!(req.method, "POST");
            assert_eq!(req.body, br#"{"a":1}"#);
            ScriptedReply::redirect(303, "/after-303")
        }
        _ => {
            assert_eq!(req.method, "GET");
            assert!(req.body.is_empty());
            assert!(!req.headers.contains_key("content-type"));
            assert!(!req.headers.contains_key("content-length"));
            ScriptedReply::ok(b"rewritten")
        }
    });

    let opts_303 = FetchOptions {
        method: "POST".to_owned(),
        allow_private_ips: true,
        body: FetchBody::Json(serde_json::json!({"a": 1})),
        ..Default::default()
    };
    let resp_303 = fetch_sync(&server_303.url("/post"), &opts_303).unwrap();
    assert!(resp_303.redirected);
    assert_eq!(resp_303.text().unwrap(), "rewritten");

    // 307 preserves POST, body, and Content-Type.
    let server_307 = TestServer::spawn(2, |idx, req| match idx {
        0 => ScriptedReply::redirect(307, "/after-307"),
        _ => {
            assert_eq!(req.method, "POST");
            assert_eq!(req.body, b"preserve-me");
            assert_eq!(
                req.headers.get("content-type").map(String::as_str),
                Some("text/plain")
            );
            ScriptedReply::ok(b"preserved")
        }
    });

    let opts_307 = FetchOptions {
        method: "POST".to_owned(),
        allow_private_ips: true,
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: FetchBody::Text("preserve-me".to_owned()),
        ..Default::default()
    };
    let resp_307 = fetch_sync(&server_307.url("/post"), &opts_307).unwrap();
    assert!(resp_307.redirected);
    assert_eq!(resp_307.text().unwrap(), "preserved");
}

#[test]
fn redirect_limit_enforced_as_too_many_redirects() {
    let server = TestServer::spawn(3, |idx, _| {
        ScriptedReply::redirect(302, format!("/hop/{}", idx + 1))
    });

    let opts = FetchOptions {
        allow_private_ips: true,
        max_redirects: 2,
        query: vec![("token".to_owned(), "secret-val".to_owned())],
        ..Default::default()
    };

    let err = fetch_sync(&server.url("/hop/0"), &opts).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::TooManyRedirects);
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains("secret-val"));
}

#[test]
fn max_response_bytes_enforced_for_declared_and_streamed_bodies() {
    let declared_server = TestServer::spawn(1, |_, _| ScriptedReply::ok(vec![b'x'; 256]));
    let opts = FetchOptions {
        allow_private_ips: true,
        max_response_bytes: 128,
        ..Default::default()
    };
    let err = fetch_sync(&declared_server.url("/big"), &opts).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::BodyTooLarge);

    let streamed_server = TestServer::spawn(1, |_, _| {
        let mut reply = ScriptedReply::ok(vec![b'y'; 256]);
        reply.omit_content_length = true;
        reply
    });
    let err = fetch_sync(&streamed_server.url("/stream"), &opts).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::BodyTooLarge);
}

#[test]
fn validation_rejects_crlf_headers_invalid_methods_and_bad_bounds() {
    let bad_header_name = FetchOptions {
        allow_private_ips: true,
        headers: vec![("Bad Header".to_owned(), "ok".to_owned())],
        ..Default::default()
    };
    assert_eq!(
        fetch_sync("http://127.0.0.1:1/", &bad_header_name)
            .unwrap_err()
            .code(),
        FetchErrorCode::InvalidOption
    );

    let crlf_header_val = FetchOptions {
        allow_private_ips: true,
        headers: vec![("X-Test".to_owned(), "val\r\nEvil: injected".to_owned())],
        ..Default::default()
    };
    assert_eq!(
        fetch_sync("http://127.0.0.1:1/", &crlf_header_val)
            .unwrap_err()
            .code(),
        FetchErrorCode::InvalidOption
    );

    let nul_bearer = FetchOptions {
        allow_private_ips: true,
        bearer_token: Some(Zeroizing::new("abc\0def".to_owned())),
        ..Default::default()
    };
    assert_eq!(
        fetch_sync("http://127.0.0.1:1/", &nul_bearer)
            .unwrap_err()
            .code(),
        FetchErrorCode::InvalidOption
    );

    let bad_method = FetchOptions {
        method: "TRACE".to_owned(),
        allow_private_ips: true,
        ..Default::default()
    };
    assert_eq!(
        fetch_sync("http://127.0.0.1:1/", &bad_method)
            .unwrap_err()
            .code(),
        FetchErrorCode::InvalidOption
    );

    let get_with_body = FetchOptions {
        method: "GET".to_owned(),
        body: FetchBody::Text("nope".to_owned()),
        allow_private_ips: true,
        ..Default::default()
    };
    assert_eq!(
        fetch_sync("http://127.0.0.1:1/", &get_with_body)
            .unwrap_err()
            .code(),
        FetchErrorCode::InvalidOption
    );

    let zero_timeout = FetchOptions {
        timeout: Duration::ZERO,
        allow_private_ips: true,
        ..Default::default()
    };
    assert_eq!(
        fetch_sync("http://127.0.0.1:1/", &zero_timeout)
            .unwrap_err()
            .code(),
        FetchErrorCode::InvalidOption
    );
}

#[test]
fn download_sync_streams_verifies_sha256_and_reports_progress() {
    // 200 KiB payload (> 3 chunks of 64 KiB).
    let payload: Vec<u8> = (0..(200 * 1024)).map(|i| (i % 251) as u8).collect();
    let expected_hex = sha256_hex(&payload);

    let payload_clone = payload.clone();
    let server = TestServer::spawn(1, move |_, _| {
        let mut reply = ScriptedReply::ok(payload_clone.clone());
        reply.headers = vec![
            (
                "Content-Type".to_owned(),
                "application/octet-stream".to_owned(),
            ),
            ("ETag".to_owned(), "\"abc123\"".to_owned()),
        ];
        reply
    });

    let dir = tempdir().unwrap();
    let dest = dir.path().join("artifact.bin");
    let opts = DownloadOptions {
        allow_private_ips: true,
        expected_sha256: Some(expected_hex.to_ascii_uppercase()),
        ..Default::default()
    };

    let mut progress_events: Vec<DownloadProgress> = Vec::new();
    let mut cb = |p: DownloadProgress| progress_events.push(p);
    let result = download_sync(&server.url("/artifact.bin"), &dest, &opts, Some(&mut cb)).unwrap();

    assert_eq!(result.path, dest);
    assert_eq!(result.status, 200);
    assert_eq!(result.bytes_written, payload.len() as u64);
    assert_eq!(result.sha256, expected_hex);
    assert_eq!(
        result.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(result.etag.as_deref(), Some("\"abc123\""));
    assert_eq!(std::fs::read(&dest).unwrap(), payload);

    assert!(progress_events.len() >= 2);
    assert_eq!(
        progress_events.last().unwrap().bytes_written,
        payload.len() as u64
    );
    assert_eq!(
        progress_events.last().unwrap().total_bytes,
        Some(payload.len() as u64)
    );

    // No leftover temp files.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["artifact.bin".to_owned()]);
}

#[test]
fn download_sync_cleans_up_temp_on_sha256_mismatch_max_bytes_and_http_error() {
    let payload = b"actual-server-bytes".to_vec();
    let wrong_sha = "0".repeat(64);

    // 1. Checksum mismatch deletes temp file and does not create dest.
    let p1 = payload.clone();
    let server1 = TestServer::spawn(1, move |_, _| ScriptedReply::ok(p1.clone()));
    let dir = tempdir().unwrap();
    let dest1 = dir.path().join("bad-sha.bin");
    let opts1 = DownloadOptions {
        allow_private_ips: true,
        expected_sha256: Some(wrong_sha),
        ..Default::default()
    };
    let err1 = download_sync(&server1.url("/file"), &dest1, &opts1, None).unwrap_err();
    assert_eq!(err1.code(), FetchErrorCode::ChecksumMismatch);
    assert!(!dest1.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // 2. Streaming max_bytes overflow deletes temp file and does not create dest.
    let server2 = TestServer::spawn(1, |_, _| {
        let mut reply = ScriptedReply::ok(vec![b'z'; 1024]);
        reply.omit_content_length = true;
        reply
    });
    let dest2 = dir.path().join("too-big.bin");
    let opts2 = DownloadOptions {
        allow_private_ips: true,
        max_bytes: 512,
        ..Default::default()
    };
    let err2 = download_sync(&server2.url("/big"), &dest2, &opts2, None).unwrap_err();
    assert_eq!(err2.code(), FetchErrorCode::BodyTooLarge);
    assert!(!dest2.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // 3. Non-2xx HTTP status returns FetchError::HttpStatus with redacted query.
    let server3 = TestServer::spawn(1, |_, _| ScriptedReply {
        status: 404,
        reason: "Not Found",
        headers: Vec::new(),
        body: b"missing".to_vec(),
        delay: Duration::ZERO,
        omit_content_length: false,
    });
    let dest3 = dir.path().join("missing.bin");
    let opts3 = DownloadOptions {
        allow_private_ips: true,
        query: vec![("sig".to_owned(), "secret-signature".to_owned())],
        ..Default::default()
    };
    let err3 = download_sync(&server3.url("/missing"), &dest3, &opts3, None).unwrap_err();
    assert_eq!(err3.code(), FetchErrorCode::HttpStatus);
    assert!(matches!(err3, FetchError::HttpStatus { status: 404, .. }));
    let err3_str = format!("{err3} {err3:?}");
    assert!(!err3_str.contains("secret-signature"));
    assert!(!dest3.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn download_sync_respects_overwrite_flag_and_validates_sha256_upfront() {
    let dir = tempdir().unwrap();
    let dest = dir.path().join("existing.txt");
    std::fs::write(&dest, b"original").unwrap();

    // overwrite == false fails without touching existing file or hitting network.
    let opts_no_overwrite = DownloadOptions {
        allow_private_ips: true,
        overwrite: false,
        ..Default::default()
    };
    let err =
        download_sync("http://127.0.0.1:1/never", &dest, &opts_no_overwrite, None).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::AlreadyExists);
    assert_eq!(std::fs::read(&dest).unwrap(), b"original");

    // Invalid expected_sha256 fails up front before hitting network.
    let opts_bad_sha = DownloadOptions {
        allow_private_ips: true,
        expected_sha256: Some("not-hex".to_owned()),
        overwrite: true,
        ..Default::default()
    };
    let err = download_sync("http://127.0.0.1:1/never", &dest, &opts_bad_sha, None).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::InvalidOption);

    // overwrite == true atomically replaces existing file.
    let server = TestServer::spawn(1, |_, _| ScriptedReply::ok(b"replaced-content"));
    let opts_overwrite = DownloadOptions {
        allow_private_ips: true,
        overwrite: true,
        ..Default::default()
    };
    let res = download_sync(&server.url("/new"), &dest, &opts_overwrite, None).unwrap();
    assert_eq!(res.bytes_written, b"replaced-content".len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), b"replaced-content");
}

#[cfg(unix)]
#[test]
fn download_sync_rejects_symlink_destination_even_with_overwrite() {
    let dir = tempdir().unwrap();
    let real_target = dir.path().join("target.txt");
    std::fs::write(&real_target, b"do-not-clobber").unwrap();
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real_target, &link).unwrap();

    let opts = DownloadOptions {
        allow_private_ips: true,
        overwrite: true,
        ..Default::default()
    };
    let err = download_sync("http://127.0.0.1:1/never", &link, &opts, None).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::InvalidOption);
    assert_eq!(std::fs::read(&real_target).unwrap(), b"do-not-clobber");
}

#[test]
fn request_timeout_returns_timeout_error_without_query_leak() {
    let server = TestServer::spawn(1, |_, _| ScriptedReply {
        status: 200,
        reason: "OK",
        headers: Vec::new(),
        body: b"too-late".to_vec(),
        delay: Duration::from_millis(350),
        omit_content_length: false,
    });

    let opts = FetchOptions {
        allow_private_ips: true,
        timeout: Duration::from_millis(80),
        query: vec![("secret".to_owned(), "do-not-leak".to_owned())],
        ..Default::default()
    };
    let err = fetch_sync(&server.url("/slow"), &opts).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::Timeout);
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains("do-not-leak"));
}

#[test]
fn user_agent_none_omits_header_and_zero_byte_download_emits_progress() {
    let server = TestServer::spawn(3, |_, _| ScriptedReply::ok(b""));
    let default_resp = fetch_sync(
        &server.url("/default-ua"),
        &FetchOptions {
            allow_private_ips: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(default_resp.status, 200);

    let no_ua_resp = fetch_sync(
        &server.url("/no-ua"),
        &FetchOptions {
            allow_private_ips: true,
            user_agent: None,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(no_ua_resp.status, 200);

    let dir = tempdir().unwrap();
    let empty_dest = dir.path().join("empty.bin");
    let mut events = Vec::new();
    let mut cb = |p: DownloadProgress| events.push(p);
    let dl = download_sync(
        &server.url("/empty"),
        &empty_dest,
        &DownloadOptions {
            allow_private_ips: true,
            user_agent: None,
            ..Default::default()
        },
        Some(&mut cb),
    )
    .unwrap();
    assert_eq!(dl.bytes_written, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].bytes_written, 0);
    assert_eq!(events[0].total_bytes, Some(0));

    let reqs = server.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        reqs[0].headers.get("user-agent").map(String::as_str),
        Some(DEFAULT_USER_AGENT)
    );
    assert!(
        reqs[1]
            .headers
            .get("user-agent")
            .is_none_or(String::is_empty),
        "expected no non-empty user-agent when FetchOptions::user_agent is None, got {:?}",
        reqs[1].headers
    );
    assert!(
        reqs[2]
            .headers
            .get("user-agent")
            .is_none_or(String::is_empty),
        "expected no non-empty user-agent when DownloadOptions::user_agent is None, got {:?}",
        reqs[2].headers
    );
}

#[test]
fn download_sync_cancellable_aborts_before_commit_on_callback_error_or_cancel_flag() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = TestServer::spawn(2, |_, _| ScriptedReply::ok(b"should-not-commit"));
    let dir = tempdir().unwrap();

    // 1. Callback error prevents committing dest_path and cleans temp file.
    let dest1 = dir.path().join("aborted.bin");
    let mut abort_cb = |_p: DownloadProgress| -> Result<(), FetchError> {
        Err(FetchError::Io {
            reason: "aborted by progress".to_owned(),
        })
    };
    let err1 = download_sync_cancellable(
        &server.url("/abort"),
        &dest1,
        &DownloadOptions {
            allow_private_ips: true,
            ..Default::default()
        },
        None,
        Some(&mut abort_cb),
    )
    .unwrap_err();
    assert_eq!(err1.code(), FetchErrorCode::Io);
    assert!(!dest1.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // 2. Cancel flag flipped inside progress callback prevents commit.
    let dest2 = dir.path().join("cancelled.bin");
    let cancel = AtomicBool::new(false);
    let mut cancel_cb = |_p: DownloadProgress| -> Result<(), FetchError> {
        cancel.store(true, Ordering::Release);
        Ok(())
    };
    let err2 = download_sync_cancellable(
        &server.url("/cancel"),
        &dest2,
        &DownloadOptions {
            allow_private_ips: true,
            ..Default::default()
        },
        Some(&cancel),
        Some(&mut cancel_cb),
    )
    .unwrap_err();
    assert_eq!(err2.code(), FetchErrorCode::Io);
    assert!(!dest2.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
