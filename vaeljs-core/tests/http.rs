//! Integration tests for `vael:http` (both enabled and `--no-default-features`).

use std::fs;

use vaeljs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("vael:http assertions");
}

#[cfg(not(feature = "vael-http"))]
#[test]
fn vael_http_is_refused_when_feature_disabled() {
    run_js(
        r#"
if (Vael.getBuiltInModulesList().includes("vael:http")) {
    throw new Error("vael:http must not appear in getBuiltInModulesList when disabled");
}
if (Vael.getBuiltInModule("vael:http") !== undefined) {
    throw new Error("Vael.getBuiltInModule('vael:http') must be undefined when disabled");
}
let imported = false;
try {
    await import("vael:http");
    imported = true;
} catch (err) {
    if (!(err instanceof TypeError)) throw err;
}
if (imported) {
    throw new Error("import('vael:http') must fail when vael-http feature is disabled");
}
"#,
    );
}

#[cfg(feature = "vael-http")]
mod enabled {
    use super::run_js;
    use std::fs;
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    use vaeljs_core::Runtime;

    #[test]
    fn servemux_routing_groups_middleware_fetch_and_builders() {
        run_js(
            r#"
import assert from "vael:assert";
import * as http from "vael:http";
import {
    ServeMux,
    createRouter,
    response,
    json,
    html,
    text,
    redirect,
    noContent,
    setCookie,
    HttpError,
    httpError,
    badRequest,
    unauthorized,
    forbidden,
    notFound,
    conflict,
    methodNotAllowed,
    recover,
    requestId,
    logger,
    securityHeaders,
    cors,
    compose,
    parseCookies,
    formatSetCookie,
} from "vael:http";

const reg = Vael.getBuiltInModule("vael:http");
assert.strictEquals(http.createRouter, reg.createRouter);
assert.strictEquals(http.ServeMux, reg.ServeMux);

// Response builders & immutability
const r1 = json({ hello: "world" }, { status: 201, headers: { "X-Custom": "abc" } });
assert.strictEquals(r1.status, 201);
assert.strictEquals(r1.header("content-type"), "application/json; charset=utf-8");
assert.strictEquals(r1.header("X-Custom"), "abc");
assert.equals(r1.json(), { hello: "world" });

const r2 = setCookie(r1, {
    name: "sid",
    value: "tok123",
    path: "/",
    httpOnly: true,
    secure: true,
    sameSite: "Strict",
});
const r3 = setCookie(r2, { name: "prefs", value: "dark", maxAge: 3600 });
assert.strictEquals(r1.headerValues("set-cookie").length, 0);
assert.equals(r2.headerValues("set-cookie"), [
    "sid=tok123; Path=/; HttpOnly; Secure; SameSite=Strict",
]);
assert.equals(r3.headerValues("set-cookie"), [
    "sid=tok123; Path=/; HttpOnly; Secure; SameSite=Strict",
    "prefs=dark; Max-Age=3600",
]);

assert.strictEquals(html("<h1>Hi</h1>").header("content-type"), "text/html; charset=utf-8");
assert.strictEquals(text("ok").header("content-type"), "text/plain; charset=utf-8");
const redir = redirect("/login", 307);
assert.strictEquals(redir.status, 307);
assert.strictEquals(redir.header("location"), "/login");
assert.strictEquals(redirect("/home").status, 302);
assert.strictEquals(noContent().status, 204);

// CRLF / invalid cookie / header rejection
assert.throws(() => response("x", { headers: { "Bad\r\nHeader": "1" } }));
assert.throws(() => response("x", { headers: { "x-test": "bad\r\nEvil: 1" } }));
assert.throws(() => redirect("/ok\r\nSet-Cookie: pwn=1"));
assert.throws(() => setCookie(r1, { name: "a", value: "1", sameSite: "None", secure: false }));
assert.throws(() => formatSetCookie({ name: "a", value: "bad;val" }));
assert.equals(parseCookies("session=xyz; theme=\"light\""), {
    __proto__: null,
    session: "xyz",
    theme: "light",
});
assert.strictEquals(Object.getPrototypeOf(parseCookies("a=1")), null);

// ServeMux + Go 1.22+ patterns + groups + middleware + fetch
const logs = [];
const recoveredErrors = [];
const mux = createRouter();
assert.ok(mux instanceof ServeMux);

mux.use(
    recover({
        onError(err) {
            recoveredErrors.push(err);
        },
    }),
    requestId(),
    logger({
        sink(entry) {
            logs.push(entry);
        },
    }),
    securityHeaders(),
    cors({ origin: ["https://app.example"], credentials: true }),
);

mux
    .get("/users/{id}", (req, ctx) =>
        json({
            route: "param",
            id: req.param("id"),
            ctxId: ctx.params.id,
            q: req.query.get("tab"),
            tags: req.query.getAll("tag"),
            cookie: req.cookies.sid,
            rid: ctx.state.requestId,
        }),
    )
    .get("/users/me", () => json({ route: "literal" }))
    .handle("/files/{path...}", (req) => text(`file:${req.param("path")}`))
    .post("/echo", async (req) => {
        const body = await req.json();
        return json({ got: body, ct: req.header("Content-Type") });
    })
    .put("/form", async (req) => {
        const form = await req.form();
        return json({ name: form.get("name"), items: form.getAll("item") });
    })
    .patch("/item", () => text("patched"))
    .delete("/item", () => noContent())
    .get("/boom-http", () => {
        throw notFound("user missing");
    })
    .get("/boom-unexpected", () => {
        throw new Error("SECRET_DB_PASSWORD_LEAK");
    });

mux.group("/api", (api) => {
    api.use((next) => async (req, ctx) => {
        if (req.header("authorization") !== "Bearer secret") {
            throw unauthorized("missing token");
        }
        ctx.state.role = "admin";
        return next(req, ctx);
    });
    api.get("/profile", (_req, ctx) => json({ role: ctx.state.role }));
    api.group("/v2", (v2) => {
        v2.use((next) => async (req, ctx) => {
            const res = await next(req, ctx);
            return response(res.body, {
                status: res.status,
                headers: { ...res.headers, "x-api-version": "2" },
            });
        });
        v2.get("/status", (_req, ctx) => json({ ok: true, role: ctx.state.role }));
    });
});

// Literal beats {id}
const meRes = await mux.fetch("/users/me");
assert.strictEquals(meRes.status, 200);
assert.equals(meRes.json(), { route: "literal" });
assert.strictEquals(meRes.header("x-content-type-options"), "nosniff");
assert.strictEquals(meRes.header("x-frame-options"), "DENY");
assert.strictEquals(meRes.header("referrer-policy"), "no-referrer");
assert.ok(typeof meRes.header("x-request-id") === "string" && meRes.header("x-request-id").length > 0);

// Params, query, cookies, request-id propagation
const userRes = await mux.fetch("/users/alice%20smith?tab=info&tag=a&tag=b", {
    headers: {
        Cookie: "sid=s42",
        "X-Request-Id": "req-123_abc",
    },
});
assert.strictEquals(userRes.status, 200);
assert.strictEquals(userRes.header("x-request-id"), "req-123_abc");
assert.equals(userRes.json(), {
    route: "param",
    id: "alice smith",
    ctxId: "alice smith",
    q: "info",
    tags: ["a", "b"],
    cookie: "s42",
    rid: "req-123_abc",
});

// Wildcard
const fileRes = await mux.fetch("/files/docs/2026/guide.md");
assert.strictEquals(fileRes.status, 200);
assert.strictEquals(fileRes.text(), "file:docs/2026/guide.md");

// HEAD automatic fallback & 405 Allow
const headRes = await mux.fetch("/users/42", { method: "HEAD" });
assert.strictEquals(headRes.status, 200);

const wrongMethod = await mux.fetch("/users/42", {
    method: "POST",
    headers: { Accept: "application/json" },
});
assert.strictEquals(wrongMethod.status, 405);
assert.strictEquals(wrongMethod.header("allow"), "GET, HEAD");
assert.strictEquals(wrongMethod.json().status, 405);

// 404
const missing = await mux.fetch("/nope");
assert.strictEquals(missing.status, 404);

// JSON POST & form PUT
const echoRes = await mux.fetch("/echo", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ n: 7 }),
});
assert.strictEquals(echoRes.status, 200);
assert.equals(echoRes.json(), { got: { n: 7 }, ct: "application/json" });

const badJsonRes = await mux.fetch("/echo", {
    method: "POST",
    headers: { Accept: "application/json" },
    body: "{not-json",
});
assert.strictEquals(badJsonRes.status, 400);
assert.strictEquals(badJsonRes.json().code, "VAEL_HTTP_INVALID_JSON");

const formRes = await mux.fetch("/form", {
    method: "PUT",
    body: "name=hello+world&item=x&item=y",
});
assert.equals(formRes.json(), { name: "hello world", items: ["x", "y"] });

// HttpError & recover()
const httpErrRes = await mux.fetch("/boom-http", {
    headers: { Accept: "application/json" },
});
assert.strictEquals(httpErrRes.status, 404);
assert.equals(httpErrRes.json(), {
    error: "user missing",
    status: 404,
    code: "VAEL_HTTP_NOT_FOUND",
});

const unexpRes = await mux.fetch("/boom-unexpected", {
    headers: { Accept: "application/json" },
});
assert.strictEquals(unexpRes.status, 500);
assert.ok(!unexpRes.text().includes("SECRET_DB_PASSWORD_LEAK"));
assert.strictEquals(recoveredErrors.length, 1);

// Group-local middleware & nested group
const unauth = await mux.fetch("/api/profile", { headers: { Accept: "application/json" } });
assert.strictEquals(unauth.status, 401);

const authOk = await mux.fetch("/api/v2/status", {
    headers: { Authorization: "Bearer secret" },
});
assert.strictEquals(authOk.status, 200);
assert.strictEquals(authOk.header("x-api-version"), "2");
assert.equals(authOk.json(), { ok: true, role: "admin" });

// CORS preflight & origin header
const preflight = await mux.fetch("/api/profile", {
    method: "OPTIONS",
    headers: {
        Origin: "https://app.example",
        "Access-Control-Request-Method": "GET",
    },
});
assert.strictEquals(preflight.status, 204);
assert.strictEquals(preflight.header("access-control-allow-origin"), "https://app.example");
assert.strictEquals(preflight.header("access-control-allow-credentials"), "true");
assert.strictEquals(preflight.header("vary"), "Origin");

// compose left-to-right order
const trace = [];
const mwA = (next) => async (req, ctx) => {
    trace.push("A1");
    const r = await next(req, ctx);
    trace.push("A2");
    return r;
};
const mwB = (next) => async (req, ctx) => {
    trace.push("B1");
    const r = await next(req, ctx);
    trace.push("B2");
    return r;
};
const composed = compose(mwA, mwB)(() => {
    trace.push("H");
    return text("ok");
});
await composed({ method: "GET", url: "/", path: "/" }, { state: {} });
assert.equals(trace, ["A1", "B1", "H", "B2", "A2"]);

// Error factories sanity
assert.ok(httpError(418, "teapot") instanceof HttpError);
assert.strictEquals(badRequest().status, 400);
assert.strictEquals(forbidden().status, 403);
assert.strictEquals(conflict().status, 409);
assert.strictEquals(methodNotAllowed(["GET"]).headers.allow, "GET");

assert.ok(logs.length >= 10);
"#,
        );
    }

    #[test]
    fn static_files_serves_mime_types_and_blocks_traversal_and_symlink_escape() {
        let dir = tempfile::tempdir().expect("temp dir");
        let assets = dir.path().join("assets");
        let sub = assets.join("sub");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&sub).expect("create assets/sub");
        fs::create_dir_all(&outside).expect("create outside");

        fs::write(assets.join("index.html"), "<h1>Home</h1>").expect("write index.html");
        fs::write(assets.join("app.mjs"), "export const x = 1;").expect("write app.mjs");
        fs::write(sub.join("data.json"), "{\"ok\":true}").expect("write data.json");
        fs::write(outside.join("secret.txt"), "TOP_SECRET").expect("write secret.txt");

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret.txt"), assets.join("escape.txt"))
            .expect("create symlink escape");

        let entry = dir.path().join("main.js");
        let assets_lit = serde_json::to_string(assets.to_str().expect("UTF-8 path")).unwrap();
        let is_unix = cfg!(unix);

        let js = format!(
            r#"
import assert from "vael:assert";
import {{ createRouter }} from "vael:http";

const mux = createRouter().static("/assets", {assets_lit});

const rootRes = await mux.fetch("/assets");
assert.strictEquals(rootRes.status, 200);
assert.strictEquals(rootRes.header("content-type"), "text/html; charset=utf-8");
assert.strictEquals(rootRes.text(), "<h1>Home</h1>");

const slashRes = await mux.fetch("/assets/");
assert.strictEquals(slashRes.status, 200);
assert.strictEquals(slashRes.text(), "<h1>Home</h1>");

const jsRes = await mux.fetch("/assets/app.mjs");
assert.strictEquals(jsRes.status, 200);
assert.strictEquals(jsRes.header("content-type"), "text/javascript; charset=utf-8");

const nestedRes = await mux.fetch("/assets/sub/data.json");
assert.strictEquals(nestedRes.status, 200);
assert.strictEquals(nestedRes.header("content-type"), "application/json");
assert.equals(nestedRes.json(), {{ ok: true }});

const missingRes = await mux.fetch("/assets/does-not-exist.txt");
assert.strictEquals(missingRes.status, 404);

const postRes = await mux.fetch("/assets/app.mjs", {{ method: "POST" }});
assert.strictEquals(postRes.status, 405);
assert.strictEquals(postRes.header("allow"), "GET, HEAD");

for (const badPath of [
    "/assets/../outside/secret.txt",
    "/assets/%2e%2e/outside/secret.txt",
    "/assets/..%2foutside/secret.txt",
]) {{
    const badRes = await mux.fetch(badPath);
    assert.ok(badRes.status === 400 || badRes.status === 404, `expected 400/404 for ${{badPath}}, got ${{badRes.status}}`);
    assert.ok(!badRes.text().includes("TOP_SECRET"));
}}

if ({is_unix}) {{
    const escRes = await mux.fetch("/assets/escape.txt");
    assert.ok(escRes.status === 400 || escRes.status === 404, `expected 400/404 for symlink escape, got ${{escRes.status}}`);
    assert.ok(!escRes.text().includes("TOP_SECRET"));
}}
"#
        );

        fs::write(&entry, js).expect("write main.js");
        let mut runtime = Runtime::new().expect("runtime");
        runtime.run_file(&entry).expect("static assertions");
    }

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

    #[test]
    fn live_hyper_tcp_server_handles_requests_limits_crlf_and_clean_close() {
        let dir = tempfile::tempdir().expect("temp dir");
        let assets = dir.path().join("public");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&assets).expect("create public");
        fs::create_dir_all(&outside).expect("create outside");
        fs::write(assets.join("hello.txt"), "hello-static").expect("write hello.txt");
        fs::write(outside.join("secret.txt"), "TOP_SECRET").expect("write secret.txt");

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret.txt"), assets.join("escape.txt"))
            .expect("create symlink");

        let ready_file = dir.path().join("server.addr");
        let entry = dir.path().join("server.js");

        let js = format!(
            r#"
import {{ writeTextFile }} from "vael:fs";
import {{
    createRouter,
    json,
    text,
    setCookie,
    recover,
    requestId,
    securityHeaders,
    serve,
}} from "vael:http";

const mux = createRouter()
    .use(recover(), requestId(), securityHeaders())
    .get("/users/{{id}}", (req, ctx) =>
        json({{
            id: req.param("id"),
            q: req.query.get("q"),
            cookie: req.cookies.sid ?? null,
            requestId: ctx.state.requestId,
        }})
    )
    .post("/echo", async (req) => {{
        const data = await req.json();
        return json({{ echoed: data }});
    }})
    .get("/cookies", () => {{
        let res = text("ok");
        res = setCookie(res, {{ name: "a", value: "1", httpOnly: true }});
        res = setCookie(res, {{ name: "b", value: "2", path: "/" }});
        return res;
    }})
    .static("/assets", {assets_path});

let server;
mux.post("/shutdown", () => {{
    // Schedule close right after returning this response.
    Promise.resolve().then(() => server.close());
    return json({{ closing: true }});
}});

server = serve({{ host: "127.0.0.1", port: 0, maxBodyBytes: 64 }}, mux);
writeTextFile({ready_path}, server.addr);
await server.finished;
"#,
            assets_path = serde_json::to_string(assets.to_str().unwrap()).unwrap(),
            ready_path = serde_json::to_string(ready_file.to_str().unwrap()).unwrap(),
        );

        fs::write(&entry, js).expect("write server.js");

        let handle = std::thread::spawn(move || {
            let mut runtime = Runtime::new().expect("runtime");
            runtime.run_file(&entry).expect("live server run_file");
        });

        // Wait for ready_file to appear with non-empty addr.
        let deadline = Instant::now() + Duration::from_secs(10);
        let addr = loop {
            if let Ok(contents) = fs::read_to_string(&ready_file)
                && !contents.trim().is_empty()
            {
                break contents.trim().to_owned();
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for server ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        // 1. GET with path param, query, cookies
        let resp1 = send_raw_http(
            &addr,
            "GET /users/42?q=rust HTTP/1.1\r\nHost: localhost\r\nCookie: sid=abc\r\nConnection: close\r\n\r\n",
        );
        assert!(resp1.starts_with("HTTP/1.1 200 OK\r\n"), "{resp1}");
        assert!(resp1.contains("x-content-type-options: nosniff"), "{resp1}");
        assert!(resp1.contains("x-request-id:"), "{resp1}");
        assert!(resp1.contains(r#""id":"42""#), "{resp1}");
        assert!(resp1.contains(r#""q":"rust""#), "{resp1}");
        assert!(resp1.contains(r#""cookie":"abc""#), "{resp1}");

        // 2. POST JSON body
        let body = r#"{"msg":"hi"}"#;
        let req2 = format!(
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let resp2 = send_raw_http(&addr, &req2);
        assert!(resp2.starts_with("HTTP/1.1 200 OK\r\n"), "{resp2}");
        assert!(resp2.contains(r#"{"echoed":{"msg":"hi"}}"#), "{resp2}");

        // 3. Multiple Set-Cookie headers
        let resp3 = send_raw_http(
            &addr,
            "GET /cookies HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(resp3.starts_with("HTTP/1.1 200 OK\r\n"), "{resp3}");
        assert!(resp3.contains("set-cookie: a=1; HttpOnly\r\n"), "{resp3}");
        assert!(resp3.contains("set-cookie: b=2; Path=/\r\n"), "{resp3}");

        // 4. Static file serving + traversal/symlink rejection
        let resp4 = send_raw_http(
            &addr,
            "GET /assets/hello.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(resp4.starts_with("HTTP/1.1 200 OK\r\n"), "{resp4}");
        assert!(resp4.ends_with("hello-static"), "{resp4}");

        let resp_trav = send_raw_http(
            &addr,
            "GET /assets/%2e%2e/outside/secret.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(
            resp_trav.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{resp_trav}"
        );
        assert!(!resp_trav.contains("TOP_SECRET"), "{resp_trav}");

        #[cfg(unix)]
        {
            let resp_sym = send_raw_http(
                &addr,
                "GET /assets/escape.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            );
            assert!(
                resp_sym.starts_with("HTTP/1.1 400 Bad Request\r\n"),
                "{resp_sym}"
            );
            assert!(!resp_sym.contains("TOP_SECRET"), "{resp_sym}");
        }

        // 5. Oversized body (> 64 bytes) -> 413 Payload Too Large
        let big = "x".repeat(128);
        let req_big = format!(
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            big.len(),
            big
        );
        let resp_big = send_raw_http(&addr, &req_big);
        assert!(
            resp_big.starts_with("HTTP/1.1 413 Payload Too Large\r\n"),
            "{resp_big}"
        );

        // 6. Malformed header / CRLF rejection by Hyper parser -> 400 Bad Request
        let resp_bad = send_raw_http(
            &addr,
            "GET /users/1 HTTP/1.1\r\nHost: localhost\r\nBad Header: nope\r\nConnection: close\r\n\r\n",
        );
        assert!(
            resp_bad.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{resp_bad}"
        );

        // 7. Clean shutdown
        let resp_shut = send_raw_http(
            &addr,
            "POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(resp_shut.starts_with("HTTP/1.1 200 OK\r\n"), "{resp_shut}");
        assert!(resp_shut.contains(r#""closing":true"#), "{resp_shut}");

        handle.join().expect("server thread must exit cleanly");

        // Port must be released immediately after server.close().
        assert!(
            TcpStream::connect(&addr).is_err(),
            "listener port {addr} should be closed after server.close()"
        );
    }

    #[test]
    fn live_server_accepts_plain_handler_and_abort_signal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready_file = dir.path().join("handler.addr");
        let entry = dir.path().join("handler.js");

        let js = format!(
            r#"
import {{ writeTextFile }} from "vael:fs";
import {{ serve, text }} from "vael:http";

let abortCb = null;
const signal = {{
    aborted: false,
    addEventListener(type, fn) {{
        if (type === "abort") abortCb = fn;
    }},
    removeEventListener() {{}},
}};

const server = serve(
    {{ host: "127.0.0.1", port: 0, signal }},
    (req) => {{
        if (req.path === "/abort") {{
            signal.aborted = true;
            if (abortCb) abortCb();
        }}
        return text(`raw:${{req.method}}:${{req.path}}`);
    }},
);
writeTextFile({ready_path}, server.addr);
await server.finished;
"#,
            ready_path = serde_json::to_string(ready_file.to_str().unwrap()).unwrap(),
        );
        fs::write(&entry, js).expect("write handler.js");

        let handle = std::thread::spawn(move || {
            let mut runtime = Runtime::new().expect("runtime");
            runtime.run_file(&entry).expect("plain handler run_file");
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
                "timed out waiting for handler server"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        let resp1 = send_raw_http(
            &addr,
            "GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(resp1.starts_with("HTTP/1.1 200 OK\r\n"), "{resp1}");
        assert!(resp1.ends_with("raw:GET:/hello"), "{resp1}");

        let resp2 = send_raw_http(
            &addr,
            "POST /abort HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(resp2.starts_with("HTTP/1.1 200 OK\r\n"), "{resp2}");
        assert!(resp2.ends_with("raw:POST:/abort"), "{resp2}");

        handle.join().expect("handler server thread exited cleanly");
    }

    #[test]
    fn live_server_dispatches_concurrent_async_handlers_and_async_dispose() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready_file = dir.path().join("concurrent.addr");
        let entry = dir.path().join("concurrent.js");

        let js = format!(
            r#"
import {{ writeTextFile }} from "vael:fs";
import {{ createRouter, serve, text }} from "vael:http";

let releaseSlow;
const slowGate = new Promise((resolve) => {{
    releaseSlow = resolve;
}});

const mux = createRouter()
    .get("/slow", async () => {{
        await slowGate;
        return text("slow-done");
    }})
    .get("/fast", () => {{
        releaseSlow();
        return text("fast-done");
    }});

const server = serve({{ host: "127.0.0.1", port: 0 }}, mux);
writeTextFile({ready_path}, server.addr);

// Close once both requests have been served; verify Symbol.asyncDispose exists.
if (typeof server[Symbol.asyncDispose] !== "function") {{
    throw new Error("missing Symbol.asyncDispose");
}}
await slowGate;
await server[Symbol.asyncDispose]();
await server.finished;
"#,
            ready_path = serde_json::to_string(ready_file.to_str().unwrap()).unwrap(),
        );
        fs::write(&entry, js).expect("write concurrent.js");

        let handle = std::thread::spawn(move || {
            let mut runtime = Runtime::new().expect("runtime");
            runtime
                .run_file(&entry)
                .expect("concurrent server run_file");
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
                "timed out waiting for concurrent server"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        let addr_slow = addr.clone();
        let slow_thread = std::thread::spawn(move || {
            send_raw_http(
                &addr_slow,
                "GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
        });

        // Wait briefly so /slow enters the JS handler and awaits slowGate before /fast runs.
        std::thread::sleep(Duration::from_millis(30));

        let fast_resp = send_raw_http(
            &addr,
            "GET /fast HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(fast_resp.starts_with("HTTP/1.1 200 OK\r\n"), "{fast_resp}");
        assert!(fast_resp.ends_with("fast-done"), "{fast_resp}");

        let slow_resp = slow_thread.join().expect("slow thread");
        assert!(slow_resp.starts_with("HTTP/1.1 200 OK\r\n"), "{slow_resp}");
        assert!(slow_resp.ends_with("slow-done"), "{slow_resp}");

        handle.join().expect("concurrent server exited cleanly");
    }

    #[test]
    fn regression_cors_prototype_pollution_rab_and_serve_validation() {
        run_js(
            r#"
import assert from "vael:assert";
import {
    createRouter,
    response,
    json,
    text,
    httpError,
    HttpError,
    cors,
    logger,
    serve,
    parseCookies,
} from "vael:http";

// 1. cors({ origin: "*", credentials: true }) must throw TypeError at setup.
assert.throws(
    () => cors({ origin: "*", credentials: true }),
    TypeError,
);
assert.throws(
    () => cors({ credentials: true }),
    TypeError,
);

// 2. Dynamic/restricted cors() must emit Vary: Origin even when allowedOrigin is null.
const restrictedMux = createRouter()
    .use(cors({ origin: ["https://allowed.example"], credentials: true }))
    .get("/data", () => json({ ok: true }));

const disallowed = await restrictedMux.fetch("/data", {
    headers: { Origin: "https://evil.example" },
});
assert.strictEquals(disallowed.status, 200);
assert.strictEquals(disallowed.header("access-control-allow-origin"), undefined);
assert.strictEquals(disallowed.header("vary"), "Origin");

const disallowedPreflight = await restrictedMux.fetch("/data", {
    method: "OPTIONS",
    headers: {
        Origin: "https://evil.example",
        "Access-Control-Request-Method": "GET",
    },
});
assert.strictEquals(disallowedPreflight.status, 204);
assert.strictEquals(disallowedPreflight.header("access-control-allow-origin"), undefined);
assert.strictEquals(disallowedPreflight.header("vary"), "Origin");

// 3. serve(null, fn) and serve(false, fn) must throw TypeError (never coerce via || {}).
assert.throws(() => serve(null, () => text("ok")), TypeError);
assert.throws(() => serve(false, () => text("ok")), TypeError);
assert.throws(() => serve(123, () => text("ok")), TypeError);

// 4. OOB ResizableArrayBuffer Uint8Array view must be rejected cleanly with TypeError.
const ab = new ArrayBuffer(16, { maxByteLength: 32 });
const oobView = new Uint8Array(ab, 8, 4);
ab.resize(4);
assert.throws(() => response(oobView), TypeError);
await assert.rejects(
    () => restrictedMux.fetch("/data", { method: "POST", body: oobView }),
    TypeError,
);

// 5. Subclassed / fake Uint8Array must be rejected cleanly.
const fakeU8 = Object.create(Uint8Array.prototype);
assert.throws(() => response(fakeU8), TypeError);

// 6. ServeMux.handle invalid pattern must not leak handler slots.
const mux2 = createRouter();
assert.throws(() => mux2.handle("INVALID PATTERN", () => text("no")));
mux2.get("/ok", () => text("yes"));
assert.strictEquals((await mux2.fetch("/ok")).text(), "yes");

// 7. Object.prototype pollution resilience.
Object.defineProperty(Object.prototype, "code", {
    configurable: true,
    set() {
        throw new Error("code setter trap");
    },
    get() {
        return "POLLUTED_CODE";
    },
});
Object.defineProperty(Object.prototype, "host", {
    configurable: true,
    value: "evil.invalid",
});
Object.defineProperty(Object.prototype, "credentials", {
    configurable: true,
    value: true,
});
Object.defineProperty(Object.prototype, "admin", {
    configurable: true,
    value: "1",
});
try {
    const err = httpError(400, "bad", "MY_CODE");
    assert.ok(err instanceof HttpError);
    assert.strictEquals(err.code, "MY_CODE");
    assert.strictEquals(err.status, 400);

    const cookies = parseCookies("sid=abc");
    assert.strictEquals(Object.getPrototypeOf(cookies), null);
    assert.strictEquals(cookies.admin, undefined);

    const pollMux = createRouter().get("/items/{id}", (req) =>
        json({
            hasAdminHeader: "admin" in req.headers,
            hasAdminParam: "admin" in req.params,
            hasAdminCookie: "admin" in req.cookies,
            protoHeaders: Object.getPrototypeOf(req.headers) === null,
            protoParams: Object.getPrototypeOf(req.params) === null,
            protoCookies: Object.getPrototypeOf(req.cookies) === null,
        }),
    );
    const pollRes = await pollMux.fetch("/items/1", { headers: { Cookie: "sid=1" } });
    assert.equals(pollRes.json(), {
        hasAdminHeader: false,
        hasAdminParam: false,
        hasAdminCookie: false,
        protoHeaders: true,
        protoParams: true,
        protoCookies: true,
    });

    // Empty cors({}) must not pick up Object.prototype.credentials = true.
    const cleanCors = cors({});
    assert.strictEquals(typeof cleanCors, "function");

    // logger() must sanitize control characters and ANSI escapes before emitting log lines.
    const emitted = [];
    const origInfo = console.info;
    console.info = (msg) => {
        emitted.push(String(msg));
    };
    try {
        const logMux = createRouter().use(logger()).get("/hello", () => text("ok"));
        await logMux.fetch("/hello\r\nFAKE 200\x1b[31m\x7f");
        assert.strictEquals(emitted.length, 1);
        const line = emitted[0];
        assert.ok(!/[\x00-\x1f\x7f-\x9f]/.test(line), `control chars leaked: ${JSON.stringify(line)}`);
        assert.ok(line.includes("\\x0d\\x0a"), line);
        assert.ok(line.includes("\\x1b"), line);
        assert.ok(line.includes("\\x7f"), line);
    } finally {
        console.info = origInfo;
    }
} finally {
    delete Object.prototype.code;
    delete Object.prototype.host;
    delete Object.prototype.credentials;
    delete Object.prototype.admin;
}
"#,
        );
    }

    #[test]
    fn live_server_handles_conflicting_framing_and_unserializable_httperror() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready_file = dir.path().join("framing.addr");
        let entry = dir.path().join("framing.js");

        let js = format!(
            r#"
import {{ writeTextFile }} from "vael:fs";
import {{ createRouter, response, httpError, serve }} from "vael:http";

let server;
const mux = createRouter()
    .get("/bad-framing", () =>
        response("hello", {{
            status: 200,
            headers: {{
                "content-length": "999",
                "transfer-encoding": "chunked",
            }},
        }})
    )
    .get("/head-ok", () =>
        response(null, {{
            status: 200,
            headers: {{
                "content-length": "1234",
            }},
        }})
    )
    .get("/bigint-error", () => {{
        throw httpError(400, "bad", "BAD_BIGINT", {{ n: 1n }});
    }})
    .post("/close", () => {{
        Promise.resolve().then(() => server.close());
        return response("bye", {{ status: 200 }});
    }});

server = serve({{ host: "127.0.0.1", port: 0 }}, mux);
writeTextFile({ready_path}, server.addr);
await server.finished;
"#,
            ready_path = serde_json::to_string(ready_file.to_str().unwrap()).unwrap(),
        );
        fs::write(&entry, js).expect("write framing.js");

        let handle = std::thread::spawn(move || {
            let mut runtime = Runtime::new().expect("runtime");
            runtime.run_file(&entry).expect("framing server run_file");
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
                "timed out waiting for framing server"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        // 1. Conflicting Content-Length / Transfer-Encoding must not panic Hyper; body length 5 wins.
        let r1 = send_raw_http(
            &addr,
            "GET /bad-framing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(r1.starts_with("HTTP/1.1 200 OK\r\n"), "{r1}");
        assert!(r1.contains("content-length: 5\r\n"), "{r1}");
        assert!(
            !r1.to_ascii_lowercase().contains("transfer-encoding:"),
            "{r1}"
        );
        assert!(r1.ends_with("hello"), "{r1}");

        // 2. HEAD response preserves valid numeric Content-Length with empty body.
        let r_head = send_raw_http(
            &addr,
            "HEAD /head-ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(r_head.starts_with("HTTP/1.1 200 OK\r\n"), "{r_head}");
        assert!(r_head.contains("content-length: 1234\r\n"), "{r_head}");

        // 3. HttpError with BigInt details + Accept: application/json must still reply cleanly (500), never hang!
        let r_bigint = send_raw_http(
            &addr,
            "GET /bigint-error HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        );
        assert!(
            r_bigint.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
            "{r_bigint}"
        );

        // 4. Subsequent request works and clean close finishes cleanly.
        let r_close = send_raw_http(
            &addr,
            "POST /close HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(r_close.starts_with("HTTP/1.1 200 OK\r\n"), "{r_close}");
        assert!(r_close.ends_with("bye"), "{r_close}");

        handle.join().expect("framing server exited cleanly");
    }

    fn spawn_test_server(
        js_template: &str,
    ) -> (tempfile::TempDir, String, std::thread::JoinHandle<()>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready_file = dir.path().join("server.addr");
        let entry = dir.path().join("server.js");
        let ready_lit = serde_json::to_string(ready_file.to_str().unwrap()).unwrap();
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

    fn read_http_response(stream: &mut TcpStream, buf: &mut Vec<u8>) -> (String, Vec<u8>) {
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos;
            }
            let mut tmp = [0_u8; 4096];
            let n = stream.read(&mut tmp).expect("read response header");
            assert!(n > 0, "unexpected EOF while reading response headers");
            buf.extend_from_slice(&tmp[..n]);
        };

        let headers_bytes = buf[..header_end].to_vec();
        let headers_str = String::from_utf8_lossy(&headers_bytes).into_owned();
        let mut rest = buf.split_off(header_end + 4);
        *buf = rest;

        let content_length = headers_str
            .lines()
            .skip(1)
            .find_map(|line| {
                let (name, val) = line.split_once(':')?;
                if name.trim().eq_ignore_ascii_case("content-length") {
                    val.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        while buf.len() < content_length {
            let mut tmp = [0_u8; 4096];
            let n = stream.read(&mut tmp).expect("read response body");
            assert!(n > 0, "unexpected EOF while reading response body");
            buf.extend_from_slice(&tmp[..n]);
        }

        let body = buf[..content_length].to_vec();
        rest = buf.split_off(content_length);
        *buf = rest;

        (headers_str, body)
    }

    #[test]
    fn live_server_rejects_oversized_chunked_body_without_oom() {
        let (_dir, addr, handle) = spawn_test_server(
            r#"
import { writeTextFile } from "vael:fs";
import { createRouter, json, serve } from "vael:http";

let server;
const mux = createRouter()
    .post("/upload", async (req) => {
        const text = await req.text();
        return json({ len: text.length, body: text });
    })
    .post("/shutdown", () => {
        Promise.resolve().then(() => server.close());
        return json({ ok: true });
    });

server = serve({ host: "127.0.0.1", port: 0, maxBodyBytes: 64 }, mux);
writeTextFile(__READY_PATH__, server.addr);
await server.finished;
"#,
        );

        // 1. Complete chunked request exceeding maxBodyBytes (80 > 64) without Content-Length.
        let chunk1 = "a".repeat(40);
        let chunk2 = "b".repeat(40);
        let req_full = format!(
            "POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            chunk1.len(),
            chunk1,
            chunk2.len(),
            chunk2,
        );
        let resp_full = send_raw_http(&addr, &req_full);
        assert!(
            resp_full.starts_with("HTTP/1.1 413 Payload Too Large\r\n"),
            "{resp_full}"
        );

        // 2. Streaming chunked request exceeding maxBodyBytes mid-stream without terminating 0-chunk.
        let mut stream = TcpStream::connect(&addr).expect("connect streaming chunked");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set read timeout");
        stream
            .write_all(
                b"POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n20\r\n",
            )
            .expect("write chunked head");
        stream.write_all(&[b'x'; 32]).expect("write chunk 1");
        stream.write_all(b"\r\n").expect("write chunk 1 crlf");
        stream.flush().expect("flush chunk 1");
        std::thread::sleep(Duration::from_millis(10));
        stream.write_all(b"40\r\n").expect("write chunk 2 size");
        stream.write_all(&[b'y'; 64]).expect("write chunk 2");
        stream.write_all(b"\r\n").expect("write chunk 2 crlf");
        stream.flush().expect("flush chunk 2");

        let mut resp_buf = Vec::new();
        let _ = stream.read_to_end(&mut resp_buf);
        let resp_stream = String::from_utf8_lossy(&resp_buf);
        assert!(
            resp_stream.starts_with("HTTP/1.1 413 Payload Too Large\r\n"),
            "{resp_stream}"
        );
        drop(stream);

        // 3. Valid small chunked request still succeeds afterward.
        let valid_chunk = "hello-chunk!";
        let req_ok = format!(
            "POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            valid_chunk.len(),
            valid_chunk,
        );
        let resp_ok = send_raw_http(&addr, &req_ok);
        assert!(resp_ok.starts_with("HTTP/1.1 200 OK\r\n"), "{resp_ok}");
        assert!(resp_ok.contains(r#""len":12"#), "{resp_ok}");
        assert!(resp_ok.contains(valid_chunk), "{resp_ok}");

        let shut = send_raw_http(
            &addr,
            "POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(shut.starts_with("HTTP/1.1 200 OK\r\n"), "{shut}");
        handle.join().expect("chunked server exited cleanly");
    }

    #[test]
    fn live_server_reuses_keep_alive_connections_sequentially_and_concurrently() {
        let (_dir, addr, handle) = spawn_test_server(
            r#"
import { writeTextFile } from "vael:fs";
import { createRouter, serve, text } from "vael:http";

let server;
const mux = createRouter()
    .get("/ping/{n}", (req) => text(`pong:${req.param("n")}`))
    .post("/echo", async (req) => text(`echo:${await req.text()}`))
    .post("/shutdown", () => {
        Promise.resolve().then(() => server.close());
        return text("bye");
    });

server = serve({ host: "127.0.0.1", port: 0 }, mux);
writeTextFile(__READY_PATH__, server.addr);
await server.finished;
"#,
        );

        // 1. Sequential reuse over one TCP connection.
        {
            let mut stream = TcpStream::connect(&addr).expect("connect sequential keep-alive");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            let mut buf = Vec::new();
            for i in 0..10 {
                let conn_hdr = if i == 9 { "close" } else { "keep-alive" };
                let req = if i % 2 == 0 {
                    format!(
                        "GET /ping/{i} HTTP/1.1\r\nHost: localhost\r\nConnection: {conn_hdr}\r\n\r\n"
                    )
                } else {
                    let payload = format!("msg-{i}");
                    format!(
                        "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: {conn_hdr}\r\n\r\n{payload}",
                        payload.len()
                    )
                };
                stream
                    .write_all(req.as_bytes())
                    .expect("write keep-alive request");
                let (headers, body) = read_http_response(&mut stream, &mut buf);
                assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
                let expected = if i % 2 == 0 {
                    format!("pong:{i}")
                } else {
                    format!("echo:msg-{i}")
                };
                assert_eq!(String::from_utf8_lossy(&body), expected);
            }
        }

        // 2. Pipelined requests over one TCP connection.
        {
            let mut stream = TcpStream::connect(&addr).expect("connect pipelined keep-alive");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            let pipelined = concat!(
                "GET /ping/100 HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n",
                "GET /ping/101 HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n",
                "GET /ping/102 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            );
            stream
                .write_all(pipelined.as_bytes())
                .expect("write pipelined requests");
            let mut buf = Vec::new();
            for expected in ["pong:100", "pong:101", "pong:102"] {
                let (headers, body) = read_http_response(&mut stream, &mut buf);
                assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
                assert_eq!(String::from_utf8_lossy(&body), expected);
            }
        }

        // 3. Concurrent keep-alive clients.
        let mut workers = Vec::new();
        for worker_id in 0..4 {
            let worker_addr = addr.clone();
            workers.push(std::thread::spawn(move || {
                let mut stream =
                    TcpStream::connect(&worker_addr).expect("connect concurrent keep-alive");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("read timeout");
                let mut buf = Vec::new();
                for step in 0..5 {
                    let conn_hdr = if step == 4 { "close" } else { "keep-alive" };
                    let req = format!(
                        "GET /ping/{worker_id}-{step} HTTP/1.1\r\nHost: localhost\r\nConnection: {conn_hdr}\r\n\r\n"
                    );
                    stream.write_all(req.as_bytes()).expect("write request");
                    let (headers, body) = read_http_response(&mut stream, &mut buf);
                    assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
                    assert_eq!(
                        String::from_utf8_lossy(&body),
                        format!("pong:{worker_id}-{step}")
                    );
                }
            }));
        }
        for w in workers {
            w.join().expect("keep-alive worker succeeded");
        }

        let shut = send_raw_http(
            &addr,
            "POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(shut.starts_with("HTTP/1.1 200 OK\r\n"), "{shut}");
        handle.join().expect("keep-alive server exited cleanly");
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

    #[test]
    fn live_server_handles_connection_churn_and_abrupt_disconnects_without_permit_leak() {
        let (_dir, addr, handle) = spawn_test_server(
            r#"
import { writeTextFile } from "vael:fs";
import { createRouter, serve, text } from "vael:http";

let server;
const mux = createRouter()
    .get("/ok", () => text("ok"))
    .get("/slow-ack", async () => {
        await new Promise((r) => Deno.core.createTimer(() => r(), 5));
        return text("done");
    })
    .post("/shutdown", () => {
        Promise.resolve().then(() => server.close());
        return text("bye");
    });

server = serve({ host: "127.0.0.1", port: 0 }, mux);
writeTextFile(__READY_PATH__, server.addr);
await server.finished;
"#,
        );

        // Exceed MAX_CONCURRENT_CONNECTIONS (1024) across rapid abrupt disconnect cycles
        // to prove semaphore permits and active connection guards never leak.
        for i in 0..1100 {
            let mut stream = TcpStream::connect(&addr).expect("connect churn");
            match i % 3 {
                0 => {
                    // Immediate disconnect with zero bytes.
                }
                1 => {
                    // Partial request line then abrupt disconnect.
                    let _ = stream.write_all(b"GET /ok HTTP/1.");
                }
                _ => {
                    // Complete request then drop before reading async response.
                    let _ = stream.write_all(
                        b"GET /slow-ack HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    );
                }
            }
            drop(stream);

            if i % 200 == 199 {
                let probe = send_raw_http(
                    &addr,
                    "GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                );
                assert!(probe.starts_with("HTTP/1.1 200 OK\r\n"), "{probe}");
                assert!(probe.ends_with("ok"), "{probe}");
            }
        }

        let final_ok = send_raw_http(
            &addr,
            "GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(final_ok.starts_with("HTTP/1.1 200 OK\r\n"), "{final_ok}");
        assert!(final_ok.ends_with("ok"), "{final_ok}");

        let shut = send_raw_http(
            &addr,
            "POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(shut.starts_with("HTTP/1.1 200 OK\r\n"), "{shut}");

        handle.join().expect("churn server exited cleanly");
        assert!(
            TcpStream::connect(&addr).is_err(),
            "listener port {addr} must be closed after server.close()"
        );
    }
}
