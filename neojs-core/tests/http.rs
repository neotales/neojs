//! Integration tests for `neo:http` (both enabled and `--no-default-features`).

use std::fs;

use neojs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("neo:http assertions");
}

#[cfg(not(feature = "neo-http"))]
#[test]
fn neo_http_is_refused_when_feature_disabled() {
    run_js(
        r#"
if (Neo.getBuiltInModulesList().includes("neo:http")) {
    throw new Error("neo:http must not appear in getBuiltInModulesList when disabled");
}
if (Neo.getBuiltInModule("neo:http") !== undefined) {
    throw new Error("Neo.getBuiltInModule('neo:http') must be undefined when disabled");
}
let imported = false;
try {
    await import("neo:http");
    imported = true;
} catch (err) {
    if (!(err instanceof TypeError)) throw err;
}
if (imported) {
    throw new Error("import('neo:http') must fail when neo-http feature is disabled");
}
"#,
    );
}

#[cfg(feature = "neo-http")]
mod enabled {
    use super::run_js;
    use neojs_core::Runtime;
    use std::fs;
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    #[test]
    fn servemux_routing_groups_middleware_fetch_and_builders() {
        run_js(
            r#"
import assert from "neo:assert";
import * as http from "neo:http";
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
} from "neo:http";

const reg = Neo.getBuiltInModule("neo:http");
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
assert.equals(parseCookies("session=xyz; theme=\"light\""), { session: "xyz", theme: "light" });

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
assert.strictEquals(badJsonRes.json().code, "NEO_HTTP_INVALID_JSON");

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
    code: "NEO_HTTP_NOT_FOUND",
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
import assert from "neo:assert";
import {{ createRouter }} from "neo:http";

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
        let mut stream = TcpStream::connect(addr).expect("connect to live neo:http server");
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
import {{ writeTextFile }} from "neo:fs";
import {{
    createRouter,
    json,
    text,
    setCookie,
    recover,
    requestId,
    securityHeaders,
    serve,
}} from "neo:http";

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
import {{ writeTextFile }} from "neo:fs";
import {{ serve, text }} from "neo:http";

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
import {{ writeTextFile }} from "neo:fs";
import {{ createRouter, serve, text }} from "neo:http";

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
// Give /slow response a turn to flush, then dispose.
await new Promise((r) => Deno.core.createTimer(() => r(), 20));
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
}
