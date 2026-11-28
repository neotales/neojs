//! Integration tests for the `vael:fetch` built-in through the QuickJS [`Runtime`].
//!
//! Every network test uses a deterministic local loopback TCP server; no
//! external network access is performed.

use vaeljs_lite_core::Runtime;

fn run_js(source: &str) {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source("fetch-test.js", source)
        .expect("fetch test module");
}

#[cfg(not(feature = "vael-fetch"))]
#[test]
fn feature_off_rejects_vael_fetch_import_and_registry() {
    run_js(
        r#"
if ("fetch" in globalThis) {
    throw new Error("globalThis.fetch must never be installed");
}
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:fetch") || list.includes("fetch")) {
    throw new Error(`vael:fetch leaked into module list: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:fetch") !== undefined) {
    throw new Error("Vael.getBuiltInModule('vael:fetch') must be undefined when disabled");
}
let imported = false;
try {
    await import("vael:fetch");
    imported = true;
} catch {
    // Expected when vael-fetch is disabled.
}
if (imported) {
    throw new Error("import('vael:fetch') succeeded while vael-fetch feature was disabled");
}
"#,
    );
}

#[cfg(feature = "vael-fetch")]
mod enabled {
    use std::collections::HashMap;
    use std::fs;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use super::run_js;

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
    }

    impl ScriptedReply {
        fn ok(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                reason: "OK",
                headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
                body: body.into(),
                delay: Duration::ZERO,
            }
        }

        fn json(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                reason: "OK",
                headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
                body: body.into(),
                delay: Duration::ZERO,
            }
        }

        fn redirect(status: u16, location: impl Into<String>) -> Self {
            Self {
                status,
                reason: "Redirect",
                headers: vec![("Location".to_owned(), location.into())],
                body: Vec::new(),
                delay: Duration::ZERO,
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
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut request_line = String::new();
        let _ = reader.read_line(&mut request_line);
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_owned();
        let target = parts.next().unwrap_or("").to_owned();

        let mut headers = HashMap::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).unwrap_or(0);
            if read == 0 || line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, value)) = line.trim_end_matches(['\r', '\n']).split_once(':') {
                let lower = name.trim().to_ascii_lowercase();
                let val = value.trim().to_owned();
                if lower == "content-length" {
                    content_length = val.parse().unwrap_or(0);
                }
                headers.insert(lower, val);
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
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            reply.status,
            reply.reason,
            reply.body.len()
        );
        for (name, value) in &reply.headers {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        stream.write_all(&reply.body)?;
        stream.flush()
    }

    fn js_str(value: &str) -> String {
        serde_json::to_string(value).expect("json string")
    }

    #[test]
    fn exports_identity_and_primordial_isolation() {
        run_js(
            r#"
if ("fetch" in globalThis) {
    throw new Error("globalThis.fetch must never be installed");
}
const ns = await import("vael:fetch");
const reg = Vael.getBuiltInModule("vael:fetch");
if (Object.keys(ns).sort().join(",") !== "FetchError,downloadSync,fetchSync") {
    throw new Error(`unexpected exports: ${Object.keys(ns).sort().join(",")}`);
}
if ("default" in ns) {
    throw new Error("vael:fetch must not have a default export");
}
if (!Object.isFrozen(reg)) {
    throw new Error("registry object must be frozen");
}
if (ns.FetchError !== reg.FetchError || ns.fetchSync !== reg.fetchSync || ns.downloadSync !== reg.downloadSync) {
    throw new Error("namespace/registry export identity mismatch");
}

// Mutate global primordials after import and ensure module operations still work safely.
globalThis.Error = function FakeError() { return {}; };
globalThis.JSON = { parse: () => ({ hijacked: true }), stringify: () => "hijacked" };
globalThis.Uint8Array = function FakeUint8Array() {};
Object.defineProperty(Object.prototype, "status", {
    set() { throw new Error("Object.prototype setter invoked"); },
    configurable: true,
});
Object.defineProperty(Object.prototype, "code", {
    set() { throw new Error("Object.prototype.code setter invoked"); },
    configurable: true,
});

const err = new ns.FetchError("VAEL_FETCH_HTTP_STATUS", "boom", 503);
if (!(err instanceof ns.FetchError) || err.name !== "FetchError" || err.code !== "VAEL_FETCH_HTTP_STATUS" || err.status !== 503) {
    throw new Error("FetchError construction failed under mutated primordials");
}
let threw = false;
try {
    ns.fetchSync("http://127.0.0.1:1/test");
} catch (caught) {
    threw = true;
    if (!(caught instanceof ns.FetchError) || caught.code !== "VAEL_FETCH_SSRF_BLOCKED") {
        throw new Error(`unexpected error: ${caught && caught.code} ${caught && caught.message}`);
    }
}
if (!threw) throw new Error("expected SSRF error");
delete Object.prototype.status;
delete Object.prototype.code;
"#,
        );
    }

    #[test]
    fn fetch_sync_get_post_json_query_headers_and_response_methods() {
        let server = TestServer::spawn(3, |idx, _req| match idx {
            0 => ScriptedReply {
                status: 200,
                reason: "OK",
                headers: vec![
                    ("Content-Type".to_owned(), "application/json".to_owned()),
                    ("X-Multi".to_owned(), "one".to_owned()),
                    ("X-Multi".to_owned(), "two".to_owned()),
                ],
                body: br#"{"hello":"world","n":42}"#.to_vec(),
                delay: Duration::ZERO,
            },
            1 => ScriptedReply::json(br#"{"received":true}"#),
            _ => ScriptedReply::ok(b"raw-bytes"),
        });

        let url0 = js_str(&server.url("/items"));
        let url1 = js_str(&server.url("/submit"));
        let url2 = js_str(&server.url("/bin"));

        run_js(&format!(
            r#"
import {{ fetchSync }} from "vael:fetch";

// 1. GET with query, headers, bearerToken, custom userAgent
const r0 = fetchSync({url0}, {{
    query: {{ a: "1", b: "hello world" }},
    headers: {{ "X-Custom": "val" }},
    bearerToken: "secret-tok",
    userAgent: "vael-test-ua/1.0",
    allowPrivateIps: true,
}});
if (r0.status !== 200 || r0.statusText !== "OK" || r0.ok !== true || r0.redirected !== false) {{
    throw new Error(`unexpected response metadata: ${{JSON.stringify(r0)}}`);
}}
if (r0.header("content-type") !== "application/json" || r0.header("X-MULTI") !== "one, two") {{
    throw new Error(`header lookup failed: ${{r0.header("X-MULTI")}}`);
}}
if (JSON.stringify(r0.headerValues("x-multi")) !== JSON.stringify(["one", "two"])) {{
    throw new Error(`headerValues lookup failed: ${{JSON.stringify(r0.headerValues("x-multi"))}}`);
}}
if (r0.header("x-missing") !== undefined) {{
    throw new Error("missing header should return undefined");
}}
if (r0.headers["x-multi"] !== "one, two") {{
    throw new Error("collapsed headers object mismatch");
}}
const parsed = r0.json();
if (parsed.hello !== "world" || parsed.n !== 42) {{
    throw new Error("json() mismatch");
}}
if (r0.text() !== '{{"hello":"world","n":42}}') {{
    throw new Error("text() mismatch");
}}
const b = r0.bytes();
if (!(b instanceof Uint8Array) || b.length === 0) {{
    throw new Error("bytes() must return Uint8Array");
}}
// Defensive copy check on body/bytes
const view = r0.body;
view[0] = 0;
if (r0.bytes()[0] === 0) {{
    throw new Error("mutating response.body mutated internal bytes");
}}

// 2. POST with json option
const r1 = fetchSync({url1}, {{
    method: "post",
    json: {{ kind: "widget", count: 3 }},
    allowPrivateIps: true,
}});
if (!r1.ok || r1.json().received !== true) {{
    throw new Error("POST json failed");
}}

// 3. PUT with Uint8Array body & userAgent: null
const r2 = fetchSync({url2}, {{
    method: "PUT",
    body: new Uint8Array([1, 2, 3, 4]),
    userAgent: null,
    allowPrivateIps: true,
}});
if (!r2.ok || r2.text() !== "raw-bytes") {{
    throw new Error("PUT bytes failed");
}}
"#
        ));

        let reqs = server.requests();
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(reqs[0].target, "/items?a=1&b=hello+world");
        assert_eq!(
            reqs[0].headers.get("authorization").map(String::as_str),
            Some("Bearer secret-tok")
        );
        assert_eq!(
            reqs[0].headers.get("user-agent").map(String::as_str),
            Some("vael-test-ua/1.0")
        );
        assert_eq!(
            reqs[0].headers.get("x-custom").map(String::as_str),
            Some("val")
        );

        assert_eq!(reqs[1].method, "POST");
        assert_eq!(
            reqs[1].headers.get("content-type").map(String::as_str),
            Some("application/json; charset=utf-8")
        );
        let posted: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
        assert_eq!(posted, serde_json::json!({ "kind": "widget", "count": 3 }));

        assert_eq!(reqs[2].method, "PUT");
        assert_eq!(reqs[2].body, vec![1, 2, 3, 4]);
        assert!(
            reqs[2]
                .headers
                .get("user-agent")
                .is_none_or(String::is_empty),
            "expected userAgent: null to suppress default User-Agent, got {:?}",
            reqs[2].headers.get("user-agent")
        );
    }

    #[test]
    fn fetch_sync_redirects_ssrf_validation_and_error_codes() {
        let server = TestServer::spawn(8, |_idx, req| {
            if req.target == "/start" {
                ScriptedReply::redirect(302, "/final")
            } else if req.target == "/final" {
                ScriptedReply::ok(b"arrived")
            } else if req.target == "/bad-utf8" {
                ScriptedReply::ok(vec![0xff, 0xfe, 0x80])
            } else if req.target == "/bad-json" {
                ScriptedReply::ok(b"{not-json")
            } else if req.target == "/slow" {
                ScriptedReply {
                    status: 200,
                    reason: "OK",
                    headers: Vec::new(),
                    body: b"late".to_vec(),
                    delay: Duration::from_millis(250),
                }
            } else {
                ScriptedReply::ok(b"0123456789abcdef")
            }
        });

        let start_url = js_str(&server.url("/start"));
        let bad_utf8_url = js_str(&server.url("/bad-utf8"));
        let bad_json_url = js_str(&server.url("/bad-json"));
        let slow_url = js_str(&server.url("/slow"));
        let big_url = js_str(&server.url("/big"));

        run_js(&format!(
            r#"
import {{ FetchError, fetchSync }} from "vael:fetch";

function expectCode(fn, expectedCode) {{
    let caught = null;
    try {{
        fn();
    }} catch (err) {{
        caught = err;
    }}
    if (!caught) throw new Error(`expected FetchError(${{expectedCode}}), got success`);
    if (!(caught instanceof FetchError) || !(caught instanceof Error)) {{
        throw new Error(`expected FetchError instance, got ${{caught}}`);
    }}
    if (caught.code !== expectedCode) {{
        throw new Error(`expected code ${{expectedCode}}, got ${{caught.code}}: ${{caught.message}}`);
    }}
    return caught;
}}

// Redirect followed
const redirected = fetchSync({start_url}, {{ allowPrivateIps: true }});
if (!redirected.redirected || !redirected.url.endsWith("/final") || redirected.text() !== "arrived") {{
    throw new Error("redirect test failed");
}}

// maxRedirects: 0 -> VAEL_FETCH_TOO_MANY_REDIRECTS
expectCode(
    () => fetchSync({start_url}, {{ maxRedirects: 0, allowPrivateIps: true }}),
    "VAEL_FETCH_TOO_MANY_REDIRECTS",
);

// SSRF blocked by default
expectCode(() => fetchSync({start_url}), "VAEL_FETCH_SSRF_BLOCKED");

// Invalid URL & userinfo
expectCode(() => fetchSync("ftp://example.com/file"), "VAEL_FETCH_INVALID_URL");
expectCode(() => fetchSync("https://user:pass@example.com/"), "VAEL_FETCH_INVALID_URL");

// Mutually exclusive body and json -> VAEL_FETCH_INVALID_OPTION
expectCode(
    () => fetchSync({big_url}, {{ method: "POST", body: "a", json: {{ a: 1 }}, allowPrivateIps: true }}),
    "VAEL_FETCH_INVALID_OPTION",
);

// Proxy / getter trap on options must fail without invoking trap
let trapCalled = false;
const proxyOpts = new Proxy({{}}, {{
    get() {{ trapCalled = true; return "GET"; }},
    ownKeys() {{ trapCalled = true; return ["method"]; }},
}});
let proxyThrew = false;
try {{
    fetchSync({big_url}, proxyOpts);
}} catch {{
    proxyThrew = true;
}}
if (!proxyThrew || trapCalled) {{
    throw new Error("proxy options must be rejected without running traps");
}}

const getterOpts = {{}};
Object.defineProperty(getterOpts, "method", {{
    enumerable: true,
    get() {{ trapCalled = true; return "GET"; }},
}});
let getterThrew = false;
try {{
    fetchSync({big_url}, getterOpts);
}} catch {{
    getterThrew = true;
}}
if (!getterThrew || trapCalled) {{
    throw new Error("accessor options must be rejected without running getter");
}}

// Invalid UTF-8 & invalid JSON
const rUtf8 = fetchSync({bad_utf8_url}, {{ allowPrivateIps: true }});
expectCode(() => rUtf8.text(), "VAEL_FETCH_INVALID_UTF8");
expectCode(() => rUtf8.json(), "VAEL_FETCH_INVALID_UTF8");

const rJson = fetchSync({bad_json_url}, {{ allowPrivateIps: true }});
expectCode(() => rJson.json(), "VAEL_FETCH_INVALID_JSON");

// maxResponseBytes exceeded
expectCode(
    () => fetchSync({big_url}, {{ maxResponseBytes: 4, allowPrivateIps: true }}),
    "VAEL_FETCH_BODY_TOO_LARGE",
);

// Timeout
expectCode(
    () => fetchSync({slow_url}, {{ timeoutMs: 40, allowPrivateIps: true }}),
    "VAEL_FETCH_TIMEOUT",
);
"#
        ));
    }

    #[test]
    fn download_sync_streams_verifies_checksum_progress_and_overwrite() {
        let payload = b"vael-lite-download-payload-0123456789";
        let expected_sha: String = Sha256::digest(payload)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        let server = TestServer::spawn(5, |_idx, req| {
            if req.target == "/missing" {
                ScriptedReply {
                    status: 404,
                    reason: "Not Found",
                    headers: Vec::new(),
                    body: b"nope".to_vec(),
                    delay: Duration::ZERO,
                }
            } else {
                ScriptedReply {
                    status: 200,
                    reason: "OK",
                    headers: vec![
                        (
                            "Content-Type".to_owned(),
                            "application/octet-stream".to_owned(),
                        ),
                        ("ETag".to_owned(), "\"v1\"".to_owned()),
                    ],
                    body: payload.to_vec(),
                    delay: Duration::ZERO,
                }
            }
        });

        let dir = tempdir().expect("tempdir");
        let file_ok = dir.path().join("artifact.bin");
        let file_bad_sha = dir.path().join("bad-sha.bin");
        let file_404 = dir.path().join("missing.bin");
        let file_cb_err = dir.path().join("cb-err.bin");

        let ok_url = js_str(&server.url("/artifact.bin"));
        let missing_url = js_str(&server.url("/missing"));
        let path_ok = js_str(file_ok.to_str().unwrap());
        let path_bad_sha = js_str(file_bad_sha.to_str().unwrap());
        let path_404 = js_str(file_404.to_str().unwrap());
        let path_cb_err = js_str(file_cb_err.to_str().unwrap());
        let sha_js = js_str(&expected_sha);
        let wrong_sha_js = js_str(&"0".repeat(64));

        run_js(&format!(
            r#"
import {{ FetchError, downloadSync }} from "vael:fetch";

const events = [];
const res = downloadSync({ok_url}, {path_ok}, {{
    allowPrivateIps: true,
    expectedSha256: {sha_js},
    onProgress: (p) => events.push(p),
}});
if (
    res.path !== {path_ok} ||
    res.status !== 200 ||
    res.bytesWritten !== {len} ||
    res.sha256 !== {sha_js} ||
    res.contentType !== "application/octet-stream" ||
    res.etag !== '"v1"'
) {{
    throw new Error(`unexpected DownloadResult: ${{JSON.stringify(res)}}`);
}}
if (events.length === 0 || events[events.length - 1].bytesWritten !== {len}) {{
    throw new Error(`progress events invalid: ${{JSON.stringify(events)}}`);
}}
if (events[events.length - 1].totalBytes !== {len}) {{
    throw new Error(`totalBytes mismatch: ${{JSON.stringify(events)}}`);
}}

// Existing file without overwrite -> VAEL_FETCH_ALREADY_EXISTS
let errExists = null;
try {{
    downloadSync({ok_url}, {path_ok}, {{ allowPrivateIps: true }});
}} catch (e) {{
    errExists = e;
}}
if (!(errExists instanceof FetchError) || errExists.code !== "VAEL_FETCH_ALREADY_EXISTS") {{
    throw new Error(`expected VAEL_FETCH_ALREADY_EXISTS, got ${{errExists && errExists.code}}`);
}}

// overwrite: true succeeds
const overwritten = downloadSync({ok_url}, {path_ok}, {{
    allowPrivateIps: true,
    overwrite: true,
}});
if (overwritten.bytesWritten !== {len}) throw new Error("overwrite failed");

// Checksum mismatch -> VAEL_FETCH_CHECKSUM_MISMATCH
let errSha = null;
try {{
    downloadSync({ok_url}, {path_bad_sha}, {{
        allowPrivateIps: true,
        expectedSha256: {wrong_sha_js},
    }});
}} catch (e) {{
    errSha = e;
}}
if (!(errSha instanceof FetchError) || errSha.code !== "VAEL_FETCH_CHECKSUM_MISMATCH") {{
    throw new Error(`expected VAEL_FETCH_CHECKSUM_MISMATCH, got ${{errSha && errSha.code}}`);
}}

// Non-2xx status -> VAEL_FETCH_HTTP_STATUS with status
let err404 = null;
try {{
    downloadSync({missing_url}, {path_404}, {{ allowPrivateIps: true }});
}} catch (e) {{
    err404 = e;
}}
if (!(err404 instanceof FetchError) || err404.code !== "VAEL_FETCH_HTTP_STATUS" || err404.status !== 404 || err404.url !== {missing_url}) {{
    throw new Error(`expected 404 FetchError with url, got ${{err404 && err404.code}} ${{err404 && err404.status}} ${{err404 && err404.url}}`);
}}

// Exception thrown inside onProgress propagates cleanly
let cbCaught = null;
try {{
    downloadSync({ok_url}, {path_cb_err}, {{
        allowPrivateIps: true,
        onProgress: () => {{ throw new TypeError("progress aborted"); }},
    }});
}} catch (e) {{
    cbCaught = e;
}}
if (!cbCaught || cbCaught.message !== "progress aborted") {{
    throw new Error(`expected progress callback error, got ${{cbCaught}}`);
}}

// Unknown or wrongly-typed options throw TypeError
let optTypeErr = null;
try {{
    downloadSync({ok_url}, {path_cb_err}, {{ allowPrivateIps: "yes" }});
}} catch (e) {{
    optTypeErr = e;
}}
if (!(optTypeErr instanceof TypeError)) {{
    throw new Error(`expected TypeError for invalid option type, got ${{optTypeErr}}`);
}}
"#,
            len = payload.len()
        ));

        assert_eq!(fs::read(&file_ok).unwrap(), payload);
        assert!(!file_bad_sha.exists());
        assert!(!file_404.exists());
        assert!(!file_cb_err.exists());
    }
}
