//! V8 integration tests for `vael:fetch`.
#![cfg(feature = "vael-fetch")]

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};
use vaeljs_core::Runtime;

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

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn module_contract_and_no_global_fetch() {
    run_js(
        r#"
import { FetchError, download, downloadSync, fetch, fetchSync } from "vael:fetch";
import * as ns from "vael:fetch";

if (globalThis.fetch !== undefined) {
    throw new Error("globalThis.fetch must never be installed");
}
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:fetch")) throw new Error("vael:fetch missing from list");
if (list.includes("fetch")) throw new Error("bare fetch in list");

const reg = Vael.getBuiltInModule("vael:fetch");
if (!reg || !Object.isFrozen(reg)) throw new Error("registry missing or unfrozen");
if (Vael.getBuiltInModule("fetch") !== undefined) throw new Error("bare name accepted");

const dyn = await import("vael:fetch");
if (dyn === reg) throw new Error("namespace must be distinct from registry");
if (Object.prototype.hasOwnProperty.call(ns, "default")) {
    throw new Error("vael:fetch must not have a default export");
}
const keys = Object.keys(ns).sort().join(",");
if (keys !== "FetchError,download,downloadSync,fetch,fetchSync") {
    throw new Error(`unexpected exports: ${keys}`);
}
if (Object.keys(reg).sort().join(",") !== keys) {
    throw new Error("registry keys drifted from namespace keys");
}
if (dyn.FetchError !== FetchError || reg.FetchError !== FetchError) throw new Error("FetchError identity");
if (dyn.fetch !== fetch || reg.fetch !== fetch) throw new Error("fetch identity");
if (dyn.fetchSync !== fetchSync || reg.fetchSync !== fetchSync) throw new Error("fetchSync identity");
if (dyn.download !== download || reg.download !== download) throw new Error("download identity");
if (dyn.downloadSync !== downloadSync || reg.downloadSync !== downloadSync) throw new Error("downloadSync identity");
if (import.meta.resolve("vael:fetch") !== "vael:fetch") throw new Error("resolve failed");
if (import.meta.resolve("VAEL:fetch") !== "vael:fetch") throw new Error("case-fold failed");
"#,
    );
}

#[test]
fn fetch_sync_and_async_get_post_json_rab_and_headers() {
    let server = TestServer::spawn(4, |idx, req| match idx {
        0 => {
            assert_eq!(req.method, "GET");
            assert_eq!(req.target, "/items?page=2&tag=vael+js");
            assert_eq!(
                req.headers.get("authorization").map(String::as_str),
                Some("Bearer secret-tok")
            );
            assert_eq!(
                req.headers.get("user-agent").map(String::as_str),
                Some("custom-agent/1.0")
            );
            let mut reply = ScriptedReply::ok(b"hello sync");
            reply.headers.push(("X-Custom".to_owned(), "v1".to_owned()));
            reply.headers.push(("X-Custom".to_owned(), "v2".to_owned()));
            reply
        }
        1 => {
            assert_eq!(req.method, "POST");
            assert_eq!(
                req.headers.get("content-type").map(String::as_str),
                Some("application/json; charset=utf-8")
            );
            let parsed: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(parsed["count"], 42);
            ScriptedReply::json(br#"{"ok":true,"echo":42}"#)
        }
        2 => {
            assert_eq!(req.method, "PUT");
            assert_eq!(req.body, &[0xca, 0xfe]);
            ScriptedReply {
                status: 404,
                reason: "Not Found",
                headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
                body: b"not-found-body".to_vec(),
                delay: Duration::ZERO,
                omit_content_length: false,
            }
        }
        _ => {
            // Invalid UTF-8 + invalid JSON test
            ScriptedReply {
                status: 200,
                reason: "OK",
                headers: Vec::new(),
                body: vec![0xff, 0xfe, 0x00],
                delay: Duration::ZERO,
                omit_content_length: false,
            }
        }
    });

    let url_items = serde_json::to_string(&server.url("/items")).unwrap();
    let url_submit = serde_json::to_string(&server.url("/submit")).unwrap();
    let url_put = serde_json::to_string(&server.url("/bin")).unwrap();
    let url_bad = serde_json::to_string(&server.url("/bad")).unwrap();

    run_js(&format!(
        r#"
import {{ FetchError, fetch, fetchSync }} from "vael:fetch";

const r1 = fetchSync({url_items}, {{
    allowPrivateIps: true,
    query: {{ page: "2", tag: "vael js" }},
    bearerToken: "secret-tok",
    userAgent: "custom-agent/1.0",
}});
if (r1.status !== 200 || !r1.ok || r1.redirected) throw new Error("r1 status/ok/redirected");
if (r1.text() !== "hello sync") throw new Error("r1 text mismatch");
if (r1.header("x-custom") !== "v1, v2" || r1.header("X-CUSTOM") !== "v1, v2") {{
    throw new Error(`r1 header mismatch: ${{r1.header("x-custom")}}`);
}}
if (r1.header("missing") !== undefined) throw new Error("missing header must be undefined");
const b1 = r1.body;
b1[0] = 0;
if (r1.text() !== "hello sync") throw new Error("mutating r1.body corrupted internal bytes");
if (r1.bytes().byteLength !== 10) throw new Error("r1.bytes length");

const r2 = await fetch({url_submit}, {{
    method: "post",
    allowPrivateIps: true,
    json: {{ count: 42, nested: [true, null, "ok"] }},
}});
if (r2.status !== 200 || !r2.ok) throw new Error("r2 status");
const j2 = r2.json();
if (j2.ok !== true || j2.echo !== 42) throw new Error("r2 json mismatch");

const rab = new ArrayBuffer(4, {{ maxByteLength: 16 }});
const view = new Uint8Array(rab, 1, 2);
view.set([0xca, 0xfe]);
rab.resize(8);
const r3 = await fetch({url_put}, {{
    method: "PUT",
    allowPrivateIps: true,
    body: view,
}});
if (r3.status !== 404 || r3.ok !== false) throw new Error("404 must return response with ok=false");
if (r3.text() !== "not-found-body") throw new Error("r3 text mismatch");

const r4 = await fetch({url_bad}, {{ allowPrivateIps: true }});
let utf8Err = null;
try {{ r4.text(); }} catch (e) {{ utf8Err = e; }}
if (!(utf8Err instanceof FetchError) || utf8Err.code !== "VAEL_FETCH_INVALID_UTF8") {{
    throw new Error(`expected VAEL_FETCH_INVALID_UTF8, got ${{utf8Err}}`);
}}
let utf8JsonErr = null;
try {{ r4.json(); }} catch (e) {{ utf8JsonErr = e; }}
if (!(utf8JsonErr instanceof FetchError) || utf8JsonErr.code !== "VAEL_FETCH_INVALID_UTF8") {{
    throw new Error(`expected VAEL_FETCH_INVALID_UTF8, got ${{utf8JsonErr}}`);
}}
let jsonErr = null;
try {{ r3.json(); }} catch (e) {{ jsonErr = e; }}
if (!(jsonErr instanceof FetchError) || jsonErr.code !== "VAEL_FETCH_INVALID_JSON") {{
    throw new Error(`expected VAEL_FETCH_INVALID_JSON, got ${{jsonErr}}`);
}}
"#,
    ));

    assert_eq!(server.requests().len(), 4);
}

#[test]
fn fetch_redirects_ssrf_prototype_safety_and_redaction() {
    let target = TestServer::spawn(1, |_, req| {
        assert!(!req.headers.contains_key("authorization"));
        assert!(!req.headers.contains_key("cookie"));
        assert_eq!(
            req.headers.get("x-keep").map(String::as_str),
            Some("preserved")
        );
        ScriptedReply::ok(b"final-hop")
    });
    let target_url = target.url("/dest");
    let origin = TestServer::spawn(1, move |_, req| {
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer cross-secret")
        );
        ScriptedReply::redirect(302, target_url.clone())
    });

    let origin_url = serde_json::to_string(&origin.url("/start")).unwrap();
    let expected_final = serde_json::to_string(&target.url("/dest")).unwrap();

    run_js(&format!(
        r#"
import {{ FetchError, fetch, fetchSync }} from "vael:fetch";

// Cross-origin redirect strips sensitive headers
const res = await fetch({origin_url}, {{
    allowPrivateIps: true,
    bearerToken: "cross-secret",
    headers: {{ "Cookie": "sid=secret", "X-Keep": "preserved" }},
}});
if (!res.redirected || res.url !== {expected_final} || res.text() !== "final-hop") {{
    throw new Error("redirect mismatch");
}}

// SSRF protection active by default AND immune to Object.prototype pollution
Object.prototype.allowPrivateIps = true;
try {{
    for (const bad of ["http://127.0.0.1/a", "http://[::1]/a", "http://169.254.169.254/latest"]) {{
        let err = null;
        try {{
            await fetch(bad);
        }} catch (e) {{
            err = e;
        }}
        if (!(err instanceof FetchError) || err.code !== "VAEL_FETCH_SSRF_BLOCKED") {{
            throw new Error(`expected SSRF block for ${{bad}}, got ${{err && err.code}}`);
        }}
    }}
}} finally {{
    delete Object.prototype.allowPrivateIps;
}}

// Mutually exclusive body and json -> VAEL_FETCH_INVALID_OPTION
let mutexErr = null;
try {{
    fetchSync("http://127.0.0.1:1/", {{ allowPrivateIps: true, method: "POST", body: "a", json: {{ a: 1 }} }});
}} catch (e) {{
    mutexErr = e;
}}
if (!(mutexErr instanceof FetchError) || mutexErr.code !== "VAEL_FETCH_INVALID_OPTION") {{
    throw new Error(`expected VAEL_FETCH_INVALID_OPTION, got ${{mutexErr && mutexErr.code}}`);
}}

// Invalid URL redacts userinfo and query
let urlErr = null;
try {{
    fetchSync("https://alice:topsecretpw@example.com/p?token=supersecret");
}} catch (e) {{
    urlErr = e;
}}
if (!(urlErr instanceof FetchError) || urlErr.code !== "VAEL_FETCH_INVALID_URL") {{
    throw new Error("expected VAEL_FETCH_INVALID_URL");
}}
if (urlErr.message.includes("topsecretpw") || urlErr.message.includes("supersecret")) {{
    throw new Error(`URL error leaked secrets: ${{urlErr.message}}`);
}}

// Proxy and getter objects rejected with TypeError without running traps
let trapRan = false;
const proxyOpts = new Proxy({{ allowPrivateIps: true }}, {{
    get() {{ trapRan = true; return true; }},
    getOwnPropertyDescriptor() {{ trapRan = true; return undefined; }},
}});
let proxyThrew = false;
try {{ fetchSync("http://127.0.0.1:1/", proxyOpts); }} catch (e) {{ proxyThrew = e instanceof TypeError; }}
if (!proxyThrew || trapRan) throw new Error("Proxy was not rejected safely");

const getterOpts = {{ get allowPrivateIps() {{ trapRan = true; return true; }} }};
let getterThrew = false;
try {{ fetchSync("http://127.0.0.1:1/", getterOpts); }} catch (e) {{ getterThrew = e instanceof TypeError; }}
if (!getterThrew || trapRan) throw new Error("Getter was not rejected safely");
"#,
    ));
}

#[test]
fn download_sync_and_async_with_progress_sha256_and_errors() {
    let payload: Vec<u8> = (0..(192 * 1024)).map(|i| (i % 251) as u8).collect();
    let expected_sha = sha256_hex(&payload);
    let p1 = payload.clone();
    let p2 = payload.clone();

    let server = TestServer::spawn(4, move |idx, _| match idx {
        0 => {
            let mut r = ScriptedReply::ok(p1.clone());
            r.headers = vec![
                (
                    "Content-Type".to_owned(),
                    "application/octet-stream".to_owned(),
                ),
                ("ETag".to_owned(), "\"v1\"".to_owned()),
            ];
            r
        }
        1 => {
            let mut r = ScriptedReply::ok(p2.clone());
            r.headers = vec![
                (
                    "Content-Type".to_owned(),
                    "application/octet-stream".to_owned(),
                ),
                ("ETag".to_owned(), "\"v2\"".to_owned()),
            ];
            r
        }
        2 => ScriptedReply::ok(b"bad-checksum-bytes"),
        _ => ScriptedReply {
            status: 404,
            reason: "Not Found",
            headers: Vec::new(),
            body: b"nope".to_vec(),
            delay: Duration::ZERO,
            omit_content_length: false,
        },
    });

    let dir = tempfile::tempdir().expect("temp dir");
    let dest_sync = dir.path().join("sync.bin");
    let dest_async = dir.path().join("async.bin");
    let dest_bad_sha = dir.path().join("bad-sha.bin");
    let dest_404 = dir.path().join("missing.bin");

    let url_file = serde_json::to_string(&server.url("/file")).unwrap();
    let url_bad_sha = serde_json::to_string(&server.url("/badsha")).unwrap();
    let url_404 = serde_json::to_string(&server.url("/missing")).unwrap();
    let dest_sync_json = serde_json::to_string(dest_sync.to_str().unwrap()).unwrap();
    let dest_async_json = serde_json::to_string(dest_async.to_str().unwrap()).unwrap();
    let dest_bad_sha_json = serde_json::to_string(dest_bad_sha.to_str().unwrap()).unwrap();
    let dest_404_json = serde_json::to_string(dest_404.to_str().unwrap()).unwrap();
    let expected_sha_json = serde_json::to_string(&expected_sha).unwrap();
    let expected_len = payload.len();

    run_js(&format!(
        r#"
import {{ FetchError, download, downloadSync }} from "vael:fetch";

const syncProgress = [];
const resSync = downloadSync({url_file}, {dest_sync_json}, {{
    allowPrivateIps: true,
    expectedSha256: {expected_sha_json},
    onProgress(p) {{
        syncProgress.push(p);
    }},
}});
if (resSync.status !== 200 || resSync.bytesWritten !== {expected_len} || resSync.sha256 !== {expected_sha_json}) {{
    throw new Error("downloadSync result mismatch");
}}
if (resSync.contentType !== "application/octet-stream" || resSync.etag !== "\"v1\"") {{
    throw new Error("downloadSync metadata mismatch");
}}
if (syncProgress.length < 2 || syncProgress[syncProgress.length - 1].bytesWritten !== {expected_len}) {{
    throw new Error("downloadSync progress mismatch");
}}

// Existing destination without overwrite -> VAEL_FETCH_ALREADY_EXISTS
let existsErr = null;
try {{
    downloadSync({url_file}, {dest_sync_json}, {{ allowPrivateIps: true }});
}} catch (e) {{
    existsErr = e;
}}
if (!(existsErr instanceof FetchError) || existsErr.code !== "VAEL_FETCH_ALREADY_EXISTS") {{
    throw new Error(`expected VAEL_FETCH_ALREADY_EXISTS, got ${{existsErr && existsErr.code}}`);
}}

// Async download with live progress
const asyncProgress = [];
const resAsync = await download({url_file}, {dest_async_json}, {{
    allowPrivateIps: true,
    expectedSha256: {expected_sha_json}.toUpperCase(),
    onProgress(p) {{
        asyncProgress.push(p);
    }},
}});
if (resAsync.bytesWritten !== {expected_len} || resAsync.etag !== "\"v2\"") {{
    throw new Error("async download result mismatch");
}}
if (asyncProgress.length < 2 || asyncProgress[asyncProgress.length - 1].bytesWritten !== {expected_len}) {{
    throw new Error("async download progress mismatch");
}}

// Checksum mismatch -> VAEL_FETCH_CHECKSUM_MISMATCH
let shaErr = null;
try {{
    await download({url_bad_sha}, {dest_bad_sha_json}, {{
        allowPrivateIps: true,
        expectedSha256: "0".repeat(64),
    }});
}} catch (e) {{
    shaErr = e;
}}
if (!(shaErr instanceof FetchError) || shaErr.code !== "VAEL_FETCH_CHECKSUM_MISMATCH") {{
    throw new Error(`expected VAEL_FETCH_CHECKSUM_MISMATCH, got ${{shaErr && shaErr.code}}`);
}}

// Non-2xx status -> FetchError with VAEL_FETCH_HTTP_STATUS & status === 404
let statusErr = null;
try {{
    await download({url_404}, {dest_404_json}, {{ allowPrivateIps: true }});
}} catch (e) {{
    statusErr = e;
}}
if (!(statusErr instanceof FetchError) || statusErr.code !== "VAEL_FETCH_HTTP_STATUS" || statusErr.status !== 404) {{
    throw new Error(`expected 404 FetchError, got ${{statusErr && statusErr.code}} ${{statusErr && statusErr.status}}`);
}}
"#,
    ));

    assert_eq!(fs::read(&dest_sync).unwrap(), payload);
    assert_eq!(fs::read(&dest_async).unwrap(), payload);
    assert!(!dest_bad_sha.exists());
    assert!(!dest_404.exists());
}

#[test]
fn async_fetch_does_not_block_v8_event_loop() {
    let slow_server = TestServer::spawn(1, |_, _| ScriptedReply {
        status: 200,
        reason: "OK",
        headers: Vec::new(),
        body: b"delayed-ok".to_vec(),
        delay: Duration::from_millis(200),
        omit_content_length: false,
    });
    let fast_server = TestServer::spawn(1, |_, _| ScriptedReply::ok(b"fast-ok"));

    let slow_url = serde_json::to_string(&slow_server.url("/slow")).unwrap();
    let fast_url = serde_json::to_string(&fast_server.url("/fast")).unwrap();
    run_js(&format!(
        r#"
import {{ fetch }} from "vael:fetch";

const order = [];
const slowP = fetch({slow_url}, {{ allowPrivateIps: true }}).then((r) => {{ order.push("slow"); return r; }});
const fastP = fetch({fast_url}, {{ allowPrivateIps: true }}).then((r) => {{ order.push("fast"); return r; }});

const [slowResp, fastResp] = await Promise.all([slowP, fastP]);
if (slowResp.text() !== "delayed-ok" || fastResp.text() !== "fast-ok") {{
    throw new Error("unexpected body");
}}
if (order.join(",") !== "fast,slow") {{
    throw new Error(`async fetch blocked V8 event loop: ${{order.join(",")}}`);
}}
"#,
    ));
}

#[test]
fn fetch_contract_details_headers_utf8_url_and_abort_on_progress_throw() {
    let dir = tempfile::tempdir().expect("temp dir");
    let sync_abort_dest = dir.path().join("sync-abort.bin");
    let async_abort_dest = dir.path().join("async-abort.bin");
    let missing_dest = dir.path().join("missing.bin");
    let empty_dest = dir.path().join("empty.bin");

    let server = TestServer::spawn(6, |idx, _| match idx {
        0 => ScriptedReply {
            status: 200,
            reason: "OK",
            headers: vec![
                ("Set-Cookie".to_owned(), "a=1".to_owned()),
                ("Set-Cookie".to_owned(), "b=2".to_owned()),
            ],
            body: b"ok".to_vec(),
            delay: Duration::ZERO,
            omit_content_length: false,
        },
        1 => ScriptedReply {
            status: 200,
            reason: "OK",
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: vec![0xff, 0xfe, 0xfd],
            delay: Duration::ZERO,
            omit_content_length: false,
        },
        2 => ScriptedReply::ok(b""),
        3 => ScriptedReply::ok(b"payload-sync"),
        4 => ScriptedReply::ok(b"payload-async"),
        _ => ScriptedReply {
            status: 404,
            reason: "Not Found",
            headers: Vec::new(),
            body: Vec::new(),
            delay: Duration::ZERO,
            omit_content_length: false,
        },
    });

    let base_url = serde_json::to_string(&server.url("/item")).unwrap();
    let empty_url = serde_json::to_string(&server.url("/empty")).unwrap();
    let err_url = serde_json::to_string(&server.url("/missing")).unwrap();
    let empty_dest_js = serde_json::to_string(&empty_dest.to_string_lossy().into_owned()).unwrap();
    let sync_abort_js =
        serde_json::to_string(&sync_abort_dest.to_string_lossy().into_owned()).unwrap();
    let async_abort_js =
        serde_json::to_string(&async_abort_dest.to_string_lossy().into_owned()).unwrap();
    let missing_dest_js =
        serde_json::to_string(&missing_dest.to_string_lossy().into_owned()).unwrap();

    run_js(&format!(
        r#"
import {{ FetchError, download, downloadSync, fetch, fetchSync }} from "vael:fetch";

const r0 = fetchSync({base_url}, {{ allowPrivateIps: true, userAgent: null }});
const cookies = r0.headerValues("set-cookie");
if (cookies.length !== 2 || cookies[0] !== "a=1" || cookies[1] !== "b=2") {{
    throw new Error(`unexpected headerValues: ${{JSON.stringify(cookies)}}`);
}}

const r1 = fetchSync({base_url}, {{ allowPrivateIps: true }});
let utf8Err = null;
try {{
    r1.json();
}} catch (err) {{
    utf8Err = err;
}}
if (!(utf8Err instanceof FetchError) || utf8Err.code !== "VAEL_FETCH_INVALID_UTF8") {{
    throw new Error(`expected VAEL_FETCH_INVALID_UTF8, got ${{utf8Err}}`);
}}

let emptyEvents = 0;
const emptyRes = downloadSync({empty_url}, {empty_dest_js}, {{
    allowPrivateIps: true,
    onProgress: (p) => {{
        emptyEvents += 1;
        if (Object.getPrototypeOf(p) !== null || !Object.isFrozen(p)) {{
            throw new Error("progress payload must be null-prototype frozen");
        }}
    }},
}});
if (emptyEvents < 1 || emptyRes.bytesWritten !== 0) {{
    throw new Error(`expected 0-byte progress event, got ${{emptyEvents}}`);
}}

let syncAbortErr = null;
try {{
    downloadSync({base_url}, {sync_abort_js}, {{
        allowPrivateIps: true,
        onProgress: () => {{ throw new Error("abort-sync"); }},
    }});
}} catch (err) {{
    syncAbortErr = err;
}}
if (!syncAbortErr || syncAbortErr.message !== "abort-sync") {{
    throw new Error(`unexpected sync abort error: ${{syncAbortErr}}`);
}}

let asyncAbortErr = null;
try {{
    await download({base_url}, {async_abort_js}, {{
        allowPrivateIps: true,
        onProgress: () => {{ throw new Error("abort-async"); }},
    }});
}} catch (err) {{
    asyncAbortErr = err;
}}
if (!asyncAbortErr || asyncAbortErr.message !== "abort-async") {{
    throw new Error(`unexpected async abort error: ${{asyncAbortErr}}`);
}}

let statusErr = null;
try {{
    await download({err_url}, {missing_dest_js}, {{ allowPrivateIps: true }});
}} catch (err) {{
    statusErr = err;
}}
if (!(statusErr instanceof FetchError) || statusErr.code !== "VAEL_FETCH_HTTP_STATUS" || statusErr.status !== 404 || statusErr.url !== {err_url}) {{
    throw new Error(`expected FetchError.url=${{{err_url}}}, got ${{statusErr?.url}}`);
}}

let typeErr = null;
try {{
    fetchSync({base_url}, {{ allowPrivateIps: "yes" }});
}} catch (err) {{
    typeErr = err;
}}
if (!(typeErr instanceof TypeError)) {{
    throw new Error(`expected TypeError for invalid option type, got ${{typeErr}}`);
}}
"#,
    ));

    let reqs = server.requests();
    assert!(
        reqs[0]
            .headers
            .get("user-agent")
            .is_none_or(String::is_empty),
        "expected null userAgent to suppress default User-Agent, got {:?}",
        reqs[0].headers.get("user-agent")
    );
    assert!(
        !sync_abort_dest.exists(),
        "aborted sync download must not commit destination file"
    );
    assert!(
        !async_abort_dest.exists(),
        "aborted async download must not commit destination file"
    );
    assert!(empty_dest.exists(), "0-byte download should commit file");
}
