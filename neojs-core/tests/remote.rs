//! End-to-end tests for opt-in HTTPS module loading.
//!
//! The transport is injected: every response is scripted, no network is used,
//! no host is real, and no credential exists. The JavaScript modules assert on
//! their own and throw on failure, so a test is one module graph plus the Rust
//! call that runs it.

#![cfg(feature = "test-support")]

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use std::time::Duration;

use neojs_core::Error;
use neojs_core::RemoteOptions;
use neojs_core::Runtime;
use neojs_core::test_support::TestRemoteResponse;

#[derive(Clone, Default)]
struct Reply {
    status: u16,
    location: Option<&'static str>,
    etag: Option<&'static str>,
    last_modified: Option<&'static str>,
    cache_control: Option<&'static str>,
    date: Option<&'static str>,
    age: Option<&'static str>,
    body: &'static str,
}

impl Reply {
    fn ok(body: &'static str) -> Self {
        Self {
            status: 200,
            body,
            ..Self::default()
        }
    }

    fn redirect(location: &'static str) -> Self {
        Self {
            status: 302,
            location: Some(location),
            ..Self::default()
        }
    }
}

/// A scripted HTTPS server keyed by the full request URL.
struct Server {
    replies: Mutex<HashMap<String, Reply>>,
    seen: Mutex<Vec<SeenRequest>>,
}

struct SeenRequest {
    url: String,
    if_none_match: Option<String>,
    if_modified_since: Option<String>,
}

impl Server {
    fn new(replies: &[(&str, Reply)]) -> Arc<Self> {
        let mut map = HashMap::new();
        for (url, reply) in replies {
            map.insert((*url).to_string(), reply.clone());
        }
        Arc::new(Self {
            replies: Mutex::new(map),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn transport(
        self: &Arc<Self>,
    ) -> impl Fn(&str, Duration, Option<&str>, Option<&str>) -> Result<TestRemoteResponse, String>
    + Send
    + Sync
    + 'static {
        let server = Arc::clone(self);
        move |url, _timeout, if_none_match, if_modified_since| {
            server.seen.lock().expect("seen lock").push(SeenRequest {
                url: url.to_string(),
                if_none_match: if_none_match.map(str::to_owned),
                if_modified_since: if_modified_since.map(str::to_owned),
            });
            let reply = server
                .replies
                .lock()
                .expect("reply lock")
                .get(url)
                .cloned()
                .ok_or_else(|| "no scripted reply".to_string())?;
            Ok(TestRemoteResponse {
                status: reply.status,
                location: reply.location.map(str::to_owned),
                declared_length: None,
                etag: reply.etag.map(str::to_owned),
                last_modified: reply.last_modified.map(str::to_owned),
                cache_control: reply.cache_control.map(str::to_owned),
                date: reply.date.map(str::to_owned),
                age: reply.age.map(str::to_owned),
                body: reply.body.as_bytes().to_vec(),
            })
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("seen lock")
            .iter()
            .map(|request| request.url.clone())
            .collect()
    }

    fn validators(&self) -> Vec<(Option<String>, Option<String>)> {
        self.seen
            .lock()
            .expect("seen lock")
            .iter()
            .map(|request| {
                (
                    request.if_none_match.clone(),
                    request.if_modified_since.clone(),
                )
            })
            .collect()
    }
}

fn remote_runtime(server: &Arc<Server>) -> Runtime {
    remote_runtime_with_options(RemoteOptions::new().without_cache(), server)
}

fn remote_runtime_with_options(options: RemoteOptions, server: &Arc<Server>) -> Runtime {
    Runtime::with_remote_transport(options, server.transport()).expect("runtime")
}

#[test]
fn runs_a_remote_entry_with_nested_static_dynamic_and_typescript_imports() {
    let server = Server::new(&[
        (
            "https://mods.test/main.ts",
            Reply::ok(
                r#"
import { value } from "./lib/dep.js";
import { run as runOne } from "./state.ts?v=1";
import { run as runTwo } from "./state.ts?v=2";

interface Box {
    value: string;
}

const box: Box = { value };
if (box.value !== "dep") throw new Error("nested static import failed");
if (new Set([runOne, runTwo]).size !== 2) {
    throw new Error("query identities were merged");
}

const dynamic = await import("./dyn.js");
if (dynamic.default() !== "dynamic") throw new Error("dynamic import failed");
const again = await import("./dyn.js");
if (again !== dynamic) throw new Error("dynamic import was not cached");

const nested = await import("./nested/entry.js");
if (nested.answer !== 42) throw new Error("nested dynamic import failed");
console.log("remote graph ok");
"#,
            ),
        ),
        (
            "https://mods.test/lib/dep.js",
            Reply::ok("export const value = \"dep\";\n"),
        ),
        (
            "https://mods.test/state.ts?v=1",
            Reply::ok(
                r#"
globalThis.__remoteCount = (globalThis.__remoteCount ?? 0) + 1;
export const run: number = globalThis.__remoteCount;
"#,
            ),
        ),
        (
            "https://mods.test/state.ts?v=2",
            Reply::ok(
                r#"
globalThis.__remoteCount = (globalThis.__remoteCount ?? 0) + 1;
export const run: number = globalThis.__remoteCount;
"#,
            ),
        ),
        (
            "https://mods.test/dyn.js",
            Reply::ok("export default () => \"dynamic\";\n"),
        ),
        (
            "https://mods.test/nested/entry.js",
            Reply::ok(
                r#"
import { answer } from "./answer.ts";
export { answer };
"#,
            ),
        ),
        (
            "https://mods.test/nested/answer.ts",
            Reply::ok("export const answer: number = 42;\n"),
        ),
    ]);

    let mut runtime = remote_runtime(&server);
    runtime
        .run_url("https://mods.test/main.ts")
        .expect("remote graph");

    assert_eq!(
        server.seen(),
        [
            "https://mods.test/main.ts",
            "https://mods.test/lib/dep.js",
            "https://mods.test/state.ts?v=1",
            "https://mods.test/state.ts?v=2",
            "https://mods.test/dyn.js",
            "https://mods.test/nested/entry.js",
            "https://mods.test/nested/answer.ts",
        ]
    );
}

#[test]
fn a_redirect_makes_the_final_url_the_identity_and_base() {
    let server = Server::new(&[
        (
            "https://mods.test/entry.js",
            Reply::redirect("https://cdn.test/dir/real.js"),
        ),
        (
            "https://cdn.test/dir/real.js",
            Reply::ok(
                r#"
import { value } from "./next.js";
if (import.meta.url !== "https://cdn.test/dir/real.js") {
    throw new Error(`unexpected import.meta.url: ${import.meta.url}`);
}
if (value !== "final base") throw new Error("relative import used the requested base");
"#,
            ),
        ),
        (
            "https://cdn.test/dir/next.js",
            Reply::ok("export const value = \"final base\";\n"),
        ),
        (
            "https://mods.test/next.js",
            Reply::ok(
                "export const value = \"requested base\";\n\
                 throw new Error(\"the requested base must not be used\");\n",
            ),
        ),
    ]);

    let mut runtime = remote_runtime(&server);
    runtime
        .run_url("https://mods.test/entry.js")
        .expect("redirected entry uses its final identity");

    assert_eq!(
        server.seen(),
        [
            "https://mods.test/entry.js",
            "https://cdn.test/dir/real.js",
            "https://cdn.test/dir/next.js",
        ]
    );
}

#[test]
fn a_local_module_can_import_an_explicit_remote_url() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { greet } from "https://mods.test/greet.js";
if (greet("world") !== "hello, world") throw new Error("remote import failed");
"#,
    )
    .expect("write source");
    let server = Server::new(&[(
        "https://mods.test/greet.js",
        Reply::ok("export function greet(name) { return `hello, ${name}`; }\n"),
    )]);

    let mut runtime = remote_runtime(&server);
    runtime.run_file(&file).expect("local imports remote");
}

#[test]
fn a_test_file_in_a_remote_runtime_can_import_a_remote_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("remote.test.js");
    fs::write(
        &file,
        r#"
import test from "test";
import { greet } from "https://mods.test/greet.js";
test("remote import", () => {
    if (greet("world") !== "hello, world") throw new Error("remote import failed");
});
"#,
    )
    .expect("write source");
    let server = Server::new(&[(
        "https://mods.test/greet.js",
        Reply::ok("export function greet(name) { return `hello, ${name}`; }\n"),
    )]);

    let mut runtime = remote_runtime(&server);
    runtime
        .run_file(&file)
        .expect("local test file imports remote");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report.total, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(server.seen(), ["https://mods.test/greet.js"]);
}

#[test]
fn a_remote_module_cannot_import_a_file_url() {
    let server = Server::new(&[(
        "https://mods.test/main.js",
        Reply::ok("import \"file:///etc/passwd\";\n"),
    )]);

    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("file import must fail");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(message.contains("cannot import a file:"), "{message}");
}

#[test]
fn remote_loading_is_disabled_by_default_for_urls_and_imports() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("URL entry must be disabled");
    assert!(matches!(error, Error::RemoteDisabled));

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, "import \"https://mods.test/main.js\";\n").expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("remote import must fail");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(error.to_string().contains("disabled"), "{error}");
}

#[test]
fn only_https_and_credential_free_urls_are_accepted() {
    let server = Server::new(&[]);
    let mut runtime = remote_runtime(&server);

    let error = runtime
        .run_url("http://mods.test/main.js")
        .expect_err("http must fail");
    match error {
        Error::RemoteEntry { reason, .. } => assert!(reason.contains("https"), "{reason}"),
        other => panic!("unexpected error: {other:?}"),
    }

    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://user:secret@mods.test/main.js")
        .expect_err("credentials must fail");
    match error {
        Error::RemoteEntry { url, reason } => {
            assert!(reason.contains("username or password"), "{reason}");
            assert!(!url.contains("secret"), "{url}");
            assert!(!reason.contains("secret"), "{reason}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(server.seen().is_empty());
}

#[test]
fn http_status_and_transport_errors_are_reported() {
    let server = Server::new(&[(
        "https://mods.test/missing.js",
        Reply {
            status: 404,
            body: "not found",
            ..Reply::default()
        },
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/missing.js")
        .expect_err("404 must fail");
    match error {
        Error::RemoteEntry { reason, .. } => assert!(reason.contains("404"), "{reason}"),
        other => panic!("unexpected error: {other:?}"),
    }

    let empty = Server::new(&[]);
    let mut runtime = remote_runtime(&empty);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("missing reply must fail");
    match error {
        Error::RemoteEntry { reason, .. } => {
            assert!(reason.contains("request for"), "{reason}")
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let mut runtime = remote_runtime(&empty);
    let error = runtime
        .run_url("https://mods.test/main.js?token=secret")
        .expect_err("missing reply must fail");
    match error {
        Error::RemoteEntry { url, reason } => {
            assert!(!url.contains("token"), "{url}");
            assert!(!reason.contains("token"), "{reason}");
            assert!(!reason.contains("secret"), "{reason}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn the_body_limit_applies_to_a_remote_entry() {
    let server = Server::new(&[(
        "https://mods.test/big.js",
        Reply::ok("export default 1; export default 2;\n"),
    )]);
    let mut runtime = Runtime::with_remote_transport(
        RemoteOptions::new()
            .with_max_module_bytes(4)
            .without_cache(),
        server.transport(),
    )
    .expect("runtime");

    let error = runtime
        .run_url("https://mods.test/big.js")
        .expect_err("limit must fail");
    match error {
        Error::RemoteEntry { reason, .. } => {
            assert!(reason.contains("4 byte limit"), "{reason}")
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn the_redirect_limit_applies_to_a_remote_entry() {
    let server = Server::new(&[
        ("https://mods.test/one.js", Reply::redirect("/two.js")),
        ("https://mods.test/two.js", Reply::redirect("/three.js")),
        ("https://mods.test/three.js", Reply::ok("")),
    ]);
    let mut runtime = Runtime::with_remote_transport(
        RemoteOptions::new().with_max_redirects(1).without_cache(),
        server.transport(),
    )
    .expect("runtime");

    let error = runtime
        .run_url("https://mods.test/one.js")
        .expect_err("redirect limit must fail");
    match error {
        Error::RemoteEntry { reason, .. } => {
            assert!(reason.contains("too many redirects"), "{reason}")
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn a_downgrade_redirect_is_refused_before_the_target_is_requested() {
    let server = Server::new(&[(
        "https://mods.test/main.js",
        Reply::redirect("http://mods.test/main.js"),
    )]);
    let mut runtime = remote_runtime(&server);

    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("downgrade must fail");
    match error {
        Error::RemoteEntry { reason, .. } => assert!(reason.contains("https"), "{reason}"),
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(server.seen(), ["https://mods.test/main.js"]);
}

#[test]
fn typescript_that_needs_a_transform_is_a_remote_typescript_error() {
    let server = Server::new(&[(
        "https://mods.test/main.ts",
        Reply::ok("enum Color { Red }\n"),
    )]);
    let mut runtime = remote_runtime(&server);

    let error = runtime
        .run_url("https://mods.test/main.ts")
        .expect_err("transform must fail");
    match error {
        Error::RemoteTypeScript { url, reason } => {
            assert_eq!(url, "https://mods.test/main.ts");
            assert!(reason.contains("enum"), "{reason}");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    // A preparation failure leaves the runtime unused.
    runtime
        .run_url("https://mods.test/other.js")
        .expect_err("the same runtime is still usable, but this URL has no reply");
}

/// A waker that unparks the thread waiting in [`block_on_std`].
struct WakeSignal(mpsc::Sender<()>);

impl Wake for WakeSignal {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

/// Polls a future to completion on the current thread, without a Tokio runtime.
fn block_on_std<F: Future>(future: F) -> F::Output {
    let (sender, receiver) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeSignal(sender)));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => receiver.recv().expect("the future must wake the thread"),
        }
    }
}

#[test]
fn runs_a_remote_entry_asynchronously_without_tokio() {
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok(
                r#"
const dynamic = await import("./dyn.js");
if (dynamic.value !== 7) throw new Error("async remote graph failed");
"#,
            ),
        ),
        (
            "https://mods.test/dyn.js",
            Reply::ok("export const value = 7;\n"),
        ),
    ]);
    let mut runtime = remote_runtime(&server);

    block_on_std(runtime.run_url_async("https://mods.test/main.js"))
        .expect("asynchronous remote entry");
}

#[test]
fn v8_errors_do_not_echo_a_redirected_final_query() {
    let server = Server::new(&[
        (
            "https://mods.test/entry.js",
            Reply::redirect("https://cdn.test/real.js?token=secret"),
        ),
        (
            "https://cdn.test/real.js?token=secret",
            Reply::ok("throw new Error(\"entry boom\");\n"),
        ),
    ]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/entry.js")
        .expect_err("entry throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(message.contains("entry boom"), "{message}");
    assert!(!message.contains("token"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn imported_v8_errors_do_not_echo_a_redirected_final_query() {
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok("import \"./child.js\";\n"),
        ),
        (
            "https://mods.test/child.js",
            Reply::redirect("https://cdn.test/child.js?token=secret"),
        ),
        (
            "https://cdn.test/child.js?token=secret",
            Reply::ok("throw new Error(\"import boom\");\n"),
        ),
    ]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("import throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(message.contains("import boom"), "{message}");
    assert!(!message.contains("token"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn a_mimicking_syntax_error_is_indistinguishable_and_may_be_redacted() {
    let server = Server::new(&[(
        "https://mods.test/mimic.js?token=secret",
        Reply::ok(
            "throw new SyntaxError(\"Missing initializer at https://mods.test/mimic.js?token=secret:1:1\");\n",
        ),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/mimic.js?token=secret")
        .expect_err("throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(
        !message.contains("token"),
        "a message that exactly mimics a generated SyntaxError location is not distinguishable: {message}"
    );
}

#[test]
fn generated_syntax_errors_do_not_echo_a_module_query() {
    let server = Server::new(&[(
        "https://mods.test/bad.js?token=secret",
        Reply::ok("const value: number = 1;\n"),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/bad.js?token=secret")
        .expect_err("the module has a syntax error");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(!message.contains("token"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn link_errors_do_not_echo_a_module_query() {
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok("import { missing } from \"./module.js?token=secret\";\n"),
        ),
        (
            "https://mods.test/module.js?token=secret",
            Reply::ok("export const present = 1;\n"),
        ),
    ]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("a missing export fails to link");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(message.contains("does not provide"), "{message}");
    assert!(!message.contains("token"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn v8_redaction_preserves_an_unrelated_url_in_the_exception_value() {
    let server = Server::new(&[(
        "https://mods.test/value.js",
        Reply::ok("throw new Error(\"keep https://value.test/a?x=1\");\n"),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/value.js")
        .expect_err("throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(
        message.contains("keep https://value.test/a?x=1"),
        "{message}"
    );
}

#[test]
fn a_script_supplied_exception_value_is_not_rewritten() {
    let server = Server::new(&[(
        "https://mods.test/value.js?token=secret",
        Reply::ok("throw new Error(\"value https://mods.test/value.js?token=secret\");\n"),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/value.js?token=secret")
        .expect_err("throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(
        message.contains("value https://mods.test/value.js?token=secret"),
        "{message}"
    );

    let server = Server::new(&[(
        "https://mods.test/relative.js",
        Reply::ok("throw new Error(\"keep './module.js?token=secret'\");\n"),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/relative.js")
        .expect_err("throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(
        message.contains("keep './module.js?token=secret'"),
        "{message}"
    );

    let server = Server::new(&[(
        "https://mods.test/error-shaped.js",
        Reply::ok(
            "throw new Error(\"Cannot find module './module.js?token=secret' at https://mods.test/error-shaped.js?token=secret:1:1\");\n",
        ),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/error-shaped.js")
        .expect_err("throws");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(
        message.contains("Cannot find module './module.js?token=secret' at https://mods.test/error-shaped.js?token=secret:1:1"),
        "{message}"
    );
}

#[test]
fn typescript_diagnostics_do_not_echo_a_query_token() {
    let server = Server::new(&[(
        "https://mods.test/bad.ts?token=secret",
        Reply::ok("enum Color { Red }\n"),
    )]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/bad.ts?token=secret")
        .expect_err("entry transform must fail");
    let message = error.to_string();
    assert!(!message.contains("token"), "{message}");
    assert!(!message.contains("secret"), "{message}");

    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok("import \"./bad.ts?token=secret\";\n"),
        ),
        (
            "https://mods.test/bad.ts?token=secret",
            Reply::ok("enum Color { Red }\n"),
        ),
    ]);
    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("import transform must fail");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(!message.contains("token"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn the_module_format_follows_the_final_url() {
    let server = Server::new(&[
        (
            "https://mods.test/entry.js",
            Reply::redirect("https://mods.test/real.ts"),
        ),
        (
            "https://mods.test/real.ts",
            Reply::ok("export const value: number = 1;\n"),
        ),
        (
            "https://mods.test/typed.ts",
            Reply::redirect("https://mods.test/plain.js"),
        ),
        (
            "https://mods.test/plain.js",
            Reply::ok("export const value: number = 1;\n"),
        ),
    ]);

    let mut runtime = remote_runtime(&server);
    runtime
        .run_url("https://mods.test/entry.js")
        .expect("a final .ts is stripped");

    let mut runtime = remote_runtime(&server);
    let error = runtime
        .run_url("https://mods.test/typed.ts")
        .expect_err("a final .js is not stripped");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(error.to_string().contains("Missing initializer"), "{error}");
}

#[test]
fn an_import_format_follows_the_final_url() {
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok(
                r#"
const typed = await import("./renamed.js");
if (typed.value !== 1) throw new Error("final .ts import was not stripped");
"#,
            ),
        ),
        (
            "https://mods.test/renamed.js",
            Reply::redirect("https://mods.test/renamed.ts"),
        ),
        (
            "https://mods.test/renamed.ts",
            Reply::ok("export const value: number = 1;\n"),
        ),
    ]);
    let mut runtime = remote_runtime(&server);
    runtime
        .run_url("https://mods.test/main.js")
        .expect("import format follows final URL");
}

#[test]
fn the_aggregate_module_and_byte_limits_bound_the_graph() {
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok("import \"./one.js\";\nimport \"./two.js\";\n"),
        ),
        ("https://mods.test/one.js", Reply::ok("")),
        ("https://mods.test/two.js", Reply::ok("")),
    ]);
    let options = RemoteOptions::new()
        .with_max_remote_modules(2)
        .without_cache();
    let mut runtime = remote_runtime_with_options(options, &server);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("three modules exceed a two-module budget");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(error.to_string().contains("module limit"), "{error}");

    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok("import \"./big.js\";\n"),
        ),
        (
            "https://mods.test/big.js",
            Reply::ok("export const value = \"0123456789\";\n"),
        ),
    ]);
    let options = RemoteOptions::new()
        .with_max_total_bytes(30)
        .without_cache();
    let mut runtime = remote_runtime_with_options(options, &server);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("two bodies exceed a small byte budget");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(
        error.to_string().contains("remaining graph budget")
            || error.to_string().contains("byte total limit"),
        "{error}"
    );
}

#[test]
fn caught_imports_cannot_bypass_the_graph_budget_or_publish_rejected_entries() {
    let cache = tempfile::tempdir().expect("cache dir");
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply {
                status: 200,
                cache_control: Some("max-age=60"),
                body: r#"
for (let index = 0; index < 6; index += 1) {
    try {
        await import(`./module.js?${index}`);
    } catch {
        // The runtime budget rejects all but the first distinct import.
    }
}
"#,
                ..Reply::default()
            },
        ),
        (
            "https://mods.test/module.js?0",
            Reply {
                status: 200,
                cache_control: Some("max-age=60"),
                body: "export const value = 0;\n",
                ..Reply::default()
            },
        ),
    ]);
    let options = RemoteOptions::new()
        .with_max_remote_modules(2)
        .with_cache_dir(cache.path());

    remote_runtime_with_options(options, &server)
        .run_url("https://mods.test/main.js")
        .expect("the caught errors are handled");

    assert_eq!(
        server.seen(),
        ["https://mods.test/main.js", "https://mods.test/module.js?0",]
    );
    let entries = fs::read_dir(cache.path())
        .expect("cache dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        })
        .count();
    assert_eq!(entries, 2, "rejected imports must not be cached");
}

#[test]
fn repeatedly_caught_oversized_imports_exhaust_the_attempted_graph() {
    let big = "x".repeat(400);
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok(
                r#"
for (let index = 0; index < 6; index += 1) {
    try {
        await import(`./module.js?${index}`);
    } catch {
        // Each distinct failure consumes an attempt until the cap is reached.
    }
}
"#,
            ),
        ),
        (
            "https://mods.test/module.js?0",
            Reply {
                status: 200,
                body: Box::leak(big.clone().into_boxed_str()),
                ..Reply::default()
            },
        ),
        (
            "https://mods.test/module.js?1",
            Reply {
                status: 200,
                body: Box::leak(big.clone().into_boxed_str()),
                ..Reply::default()
            },
        ),
    ]);
    let options = RemoteOptions::new()
        .with_max_module_bytes(300)
        .with_max_remote_modules(3)
        .without_cache();
    let mut runtime = remote_runtime_with_options(options, &server);
    runtime
        .run_url("https://mods.test/main.js")
        .expect("the caught failures are handled");

    assert_eq!(
        server.seen(),
        [
            "https://mods.test/main.js",
            "https://mods.test/module.js?0",
            "https://mods.test/module.js?1",
        ]
    );
}

#[test]
fn a_preflight_rejection_refunds_the_entry_budget() {
    let server = Server::new(&[
        (
            "https://mods.test/bad.json",
            Reply {
                status: 200,
                body: "{}",
                ..Reply::default()
            },
        ),
        (
            "https://mods.test/good.js",
            Reply::ok("globalThis.__good = true;\n"),
        ),
    ]);
    let options = RemoteOptions::new()
        .with_max_remote_modules(1)
        .without_cache();
    let mut runtime = remote_runtime_with_options(options, &server);

    let error = runtime
        .run_url("https://mods.test/bad.json")
        .expect_err("an unsupported format fails preflight");
    assert!(matches!(error, Error::RemoteEntry { .. }), "{error:?}");
    runtime
        .run_url("https://mods.test/good.js")
        .expect("a failed preflight must refund the module slot");
}

#[test]
fn a_failed_entry_preflight_still_spends_its_network_request() {
    let server = Server::new(&[
        (
            "https://mods.test/bad.ts",
            Reply::ok("enum Color { Red }\n"),
        ),
        (
            "https://mods.test/good.js",
            Reply::ok("globalThis.__good = true;\n"),
        ),
    ]);
    let options = RemoteOptions::new()
        .with_max_requests(1)
        .with_max_remote_modules(1)
        .without_cache();
    let mut runtime = remote_runtime_with_options(options, &server);

    let error = runtime
        .run_url("https://mods.test/bad.ts")
        .expect_err("preflight fails");
    assert!(matches!(error, Error::RemoteTypeScript { .. }), "{error:?}");
    let error = runtime
        .run_url("https://mods.test/good.js")
        .expect_err("the request budget is spent");
    match error {
        Error::RemoteEntry { reason, .. } => {
            assert!(reason.contains("1 request limit"), "{reason}")
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn a_utf8_preflight_rejection_refunds_the_entry_budget() {
    let transport = |url: &str, _: Duration, _: Option<&str>, _: Option<&str>| {
        if url.ends_with("bad.js") {
            Ok(TestRemoteResponse {
                status: 200,
                body: vec![0xff, 0xfe],
                ..TestRemoteResponse::default()
            })
        } else {
            Ok(TestRemoteResponse {
                status: 200,
                body: b"globalThis.__good = true;\n".to_vec(),
                ..TestRemoteResponse::default()
            })
        }
    };
    let options = RemoteOptions::new()
        .with_max_remote_modules(1)
        .without_cache();
    let mut runtime = Runtime::with_remote_transport(options, transport).expect("runtime");

    let error = runtime
        .run_url("https://mods.test/bad.js")
        .expect_err("invalid UTF-8 fails preflight");
    assert!(matches!(error, Error::RemoteEntry { .. }), "{error:?}");
    runtime
        .run_url("https://mods.test/good.js")
        .expect("a failed preflight must refund the module slot");
}

#[test]
fn a_typescript_preflight_rejection_refunds_the_entry_budget() {
    let server = Server::new(&[
        (
            "https://mods.test/bad.ts",
            Reply::ok("enum Color { Red }\n"),
        ),
        (
            "https://mods.test/good.js",
            Reply::ok("globalThis.__good = true;\n"),
        ),
    ]);
    let options = RemoteOptions::new()
        .with_max_remote_modules(1)
        .without_cache();
    let mut runtime = remote_runtime_with_options(options, &server);

    let error = runtime
        .run_url("https://mods.test/bad.ts")
        .expect_err("a transform-required module fails preflight");
    assert!(matches!(error, Error::RemoteTypeScript { .. }), "{error:?}");
    runtime
        .run_url("https://mods.test/good.js")
        .expect("a failed preflight must refund the module slot");
}

#[test]
fn dropping_an_unpolled_remote_entry_future_performs_no_io() {
    let server = Server::new(&[("https://mods.test/main.js", Reply::ok(""))]);
    let mut runtime = remote_runtime(&server);

    let future = runtime.run_url_async("https://mods.test/main.js");
    drop(future);

    assert!(server.seen().is_empty(), "an unpolled future fetched a URL");
    runtime
        .run_url("https://mods.test/main.js")
        .expect("the runtime was not consumed");
}

#[test]
fn a_second_entry_is_refused_after_a_remote_entry() {
    let server = Server::new(&[("https://mods.test/main.js", Reply::ok(""))]);
    let mut runtime = remote_runtime(&server);
    runtime
        .run_url("https://mods.test/main.js")
        .expect("first entry");

    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("second entry must fail");
    assert!(matches!(error, Error::RuntimeConsumed));
}

#[test]
fn a_fresh_cached_module_is_reused_across_runtimes() {
    let cache = tempfile::tempdir().expect("cache dir");
    let server = Server::new(&[(
        "https://mods.test/main.js",
        Reply {
            status: 200,
            cache_control: Some("max-age=60"),
            body: "if (globalThis.__cached !== undefined) {} else {};\n",
            ..Reply::default()
        },
    )]);
    let options = RemoteOptions::new().with_cache_dir(cache.path());

    remote_runtime_with_options(options.clone(), &server)
        .run_url("https://mods.test/main.js")
        .expect("first runtime");
    remote_runtime_with_options(options, &server)
        .run_url("https://mods.test/main.js")
        .expect("second runtime");

    assert_eq!(server.seen(), ["https://mods.test/main.js"]);
    assert_eq!(server.validators(), [(None, None)]);
}

#[test]
fn a_stale_cached_module_revalidates_and_reuses_the_304_body() {
    type Validators = Arc<Mutex<Vec<Option<String>>>>;

    fn transport(
        calls: Arc<AtomicUsize>,
        validators: Validators,
    ) -> impl Fn(&str, Duration, Option<&str>, Option<&str>) -> Result<TestRemoteResponse, String>
    + Send
    + Sync
    + 'static {
        move |_url, _timeout, etag, _last_modified| {
            validators
                .lock()
                .expect("validators")
                .push(etag.map(str::to_owned));
            let call = calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(TestRemoteResponse {
                    status: 200,
                    cache_control: Some("max-age=0".to_string()),
                    etag: Some("\"v1\"".to_string()),
                    body: b"globalThis.__stale = true;\n".to_vec(),
                    ..TestRemoteResponse::default()
                })
            } else {
                Ok(TestRemoteResponse {
                    status: 304,
                    cache_control: Some("max-age=0".to_string()),
                    etag: Some("\"v1\"".to_string()),
                    ..TestRemoteResponse::default()
                })
            }
        }
    }

    let cache = tempfile::tempdir().expect("cache dir");
    let calls = Arc::new(AtomicUsize::new(0));
    let validators = Arc::new(Mutex::new(Vec::new()));
    let options = RemoteOptions::new().with_cache_dir(cache.path());

    Runtime::with_remote_transport(
        options.clone(),
        transport(Arc::clone(&calls), Arc::clone(&validators)),
    )
    .expect("first runtime")
    .run_url("https://mods.test/main.js")
    .expect("first fetch");
    Runtime::with_remote_transport(
        options,
        transport(Arc::clone(&calls), Arc::clone(&validators)),
    )
    .expect("second runtime")
    .run_url("https://mods.test/main.js")
    .expect("revalidated cache");

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        *validators.lock().expect("validators"),
        [None, Some("\"v1\"".to_string())]
    );
}

#[test]
fn the_cache_key_includes_the_query() {
    let cache = tempfile::tempdir().expect("cache dir");
    let server = Server::new(&[
        (
            "https://mods.test/main.js?one",
            Reply {
                status: 200,
                cache_control: Some("max-age=60"),
                body: "globalThis.__one = true;\n",
                ..Reply::default()
            },
        ),
        (
            "https://mods.test/main.js?two",
            Reply {
                status: 200,
                cache_control: Some("max-age=60"),
                body: "globalThis.__two = true;\n",
                ..Reply::default()
            },
        ),
    ]);
    let options = RemoteOptions::new().with_cache_dir(cache.path());

    remote_runtime_with_options(options.clone(), &server)
        .run_url("https://mods.test/main.js?one")
        .expect("one");
    remote_runtime_with_options(options.clone(), &server)
        .run_url("https://mods.test/main.js?two")
        .expect("two");
    remote_runtime_with_options(options, &server)
        .run_url("https://mods.test/main.js?one")
        .expect("one again");

    assert_eq!(
        server.seen(),
        [
            "https://mods.test/main.js?one",
            "https://mods.test/main.js?two",
        ]
    );
}

#[test]
fn an_unwritable_cache_does_not_fail_the_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let blocker = dir.path().join("not-a-directory");
    fs::write(&blocker, "x").expect("write blocker");
    let server = Server::new(&[("https://mods.test/main.js", Reply::ok(""))]);
    let options = RemoteOptions::new().with_cache_dir(blocker);

    remote_runtime_with_options(options, &server)
        .run_url("https://mods.test/main.js")
        .expect("a cache write failure must not fail loading");
}
