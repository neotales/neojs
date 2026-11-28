#![cfg(feature = "vael-http")]

//! Slowloris header and slow-body 408 timeout tests using the `test-support`
//! `op_vael_http_set_test_timeouts` hook.

use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use vaeljs_core::Runtime;

fn send_raw_http(addr: &str, raw_request: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to live vael:http server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    stream
        .write_all(raw_request.as_bytes())
        .expect("write HTTP request");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read HTTP response");
    String::from_utf8_lossy(&buf).into_owned()
}

fn spawn_test_server(
    js_template: &str,
) -> (tempfile::TempDir, String, std::thread::JoinHandle<()>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let ready_file = dir.path().join("server.addr");
    let entry = dir.path().join("server.js");
    let ready_lit = format!("{:?}", ready_file.to_str().unwrap());
    let js = js_template.replace("__READY_PATH__", &ready_lit);
    fs::write(&entry, js).expect("write test server script");

    let handle = std::thread::spawn(move || {
        let mut runtime = Runtime::new().expect("runtime");
        runtime.run_file(&entry).expect("test server run_file");
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let addr = loop {
        if let Ok(contents) = fs::read_to_string(&ready_file)
            && !contents.trim().is_empty()
        {
            break contents.trim().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for test server ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    (dir, addr, handle)
}

#[test]
fn live_server_times_out_slowloris_partial_headers_and_slow_body_with_408() {
    let (_dir, addr, handle) = spawn_test_server(
        r#"
import { writeTextFile } from "vael:fs";
import { createRouter, serve, text } from "vael:http";

const timeoutRes = Deno.core.ops.op_vael_http_set_test_timeouts(150, 150);
if (!timeoutRes || timeoutRes.ok !== true) {
    throw new Error("failed to configure test timeouts");
}

let server;
const mux = createRouter()
    .get("/ok", () => text("ok"))
    .post("/echo", async (req) => text(await req.text()))
    .post("/shutdown", () => {
        Promise.resolve().then(() => server.close());
        return text("bye");
    });

server = serve({ host: "127.0.0.1", port: 0 }, mux);
writeTextFile(__READY_PATH__, server.addr);
await server.finished;
"#,
    );

    // 1. Slowloris partial headers (never finishes \r\n\r\n) -> 408 Request Timeout.
    {
        let mut stream = TcpStream::connect(&addr).expect("connect slowloris header");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read timeout");
        stream
            .write_all(b"GET /ok HTTP/1.1\r\nHost: local")
            .expect("write partial headers");
        stream.flush().expect("flush partial headers");

        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .expect("read slowloris header 408 response");
        let resp = String::from_utf8_lossy(&buf);
        assert!(
            resp.starts_with("HTTP/1.1 408 Request Timeout\r\n"),
            "expected 408 for slowloris headers, got: {resp:?}"
        );
    }

    // 2. Slow trickle POST body (incomplete Content-Length) -> 408 Request Timeout.
    {
        let mut stream = TcpStream::connect(&addr).expect("connect slow trickle body");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read timeout");
        stream
            .write_all(
                b"POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\nConnection: close\r\n\r\nhello",
            )
            .expect("write partial body");
        stream.flush().expect("flush partial body");
        std::thread::sleep(Duration::from_millis(30));
        let _ = stream.write_all(b"!");
        let _ = stream.flush();

        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .expect("read slow body 408 response");
        let resp = String::from_utf8_lossy(&buf);
        assert!(
            resp.starts_with("HTTP/1.1 408 Request Timeout\r\n"),
            "expected 408 for slow body, got: {resp:?}"
        );
    }

    // 3. Normal fast request still succeeds and server shuts down cleanly.
    let ok_resp = send_raw_http(
        &addr,
        "GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(ok_resp.starts_with("HTTP/1.1 200 OK\r\n"), "{ok_resp}");
    assert!(ok_resp.ends_with("ok"), "{ok_resp}");

    let shut = send_raw_http(
        &addr,
        "POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
    assert!(shut.starts_with("HTTP/1.1 200 OK\r\n"), "{shut}");
    handle.join().expect("slowloris server exited cleanly");
}
