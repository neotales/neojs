//! End-to-end tests for `gh:` and `github:` module specifiers.
//!
//! Every response comes from a scripted in-process transport: there is no
//! network, no real host, and no credential. The tests assert the exact URLs
//! requested, so a wrong raw URL or a hidden second spelling is visible instead
//! of being absorbed by a mock.

#![cfg(feature = "test-support")]

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
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
    cache_control: Option<&'static str>,
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

    fn cacheable(body: &'static str) -> Self {
        Self {
            status: 200,
            cache_control: Some("max-age=60"),
            body,
        }
    }
}

/// A scripted HTTPS server keyed by the exact request URL.
struct Server {
    replies: Mutex<HashMap<String, Reply>>,
    seen: Mutex<Vec<String>>,
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
        move |url, _timeout, _if_none_match, _if_modified_since| {
            server.seen.lock().expect("seen lock").push(url.to_string());
            let reply = server
                .replies
                .lock()
                .expect("reply lock")
                .get(url)
                .cloned()
                .ok_or_else(|| "no scripted reply".to_string())?;
            Ok(TestRemoteResponse {
                status: reply.status,
                location: None,
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: reply.cache_control.map(str::to_owned),
                date: None,
                age: None,
                body: reply.body.as_bytes().to_vec(),
            })
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("seen lock").clone()
    }
}

fn runtime(server: &Arc<Server>) -> Runtime {
    Runtime::with_remote_transport(RemoteOptions::new().without_cache(), server.transport())
        .expect("runtime")
}

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
fn runs_a_gh_entry_with_nested_relative_typescript_and_dynamic_imports() {
    let server = Server::new(&[
        (
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/main.ts",
            Reply::ok(
                r#"
import { value } from "./lib/dep.ts";

interface Box {
    value: string;
}

const box: Box = { value };
if (box.value !== "dep") throw new Error("nested static import failed");
if (!import.meta.url.endsWith("/v1.2.3/src/main.ts")) {
    throw new Error(`unexpected identity: ${import.meta.url}`);
}

const nested = await import("./nested/thing.js");
if (nested.answer !== 42) throw new Error("nested dynamic import failed");
"#,
            ),
        ),
        (
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/lib/dep.ts",
            Reply::ok("export const value: string = \"dep\";\n"),
        ),
        (
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/nested/thing.js",
            Reply::ok(
                r#"
import { answer } from "../shared.ts";
export { answer };
"#,
            ),
        ),
        (
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/shared.ts",
            Reply::ok("export const answer: number = 42;\n"),
        ),
    ]);

    runtime(&server)
        .run_url("gh:acme/widgets@v1.2.3/src/main.ts")
        .expect("gh entry graph");

    assert_eq!(
        server.seen(),
        [
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/main.ts",
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/lib/dep.ts",
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/nested/thing.js",
            "https://raw.githubusercontent.com/acme/widgets/v1.2.3/src/shared.ts",
        ]
    );
}

#[test]
fn runs_a_github_entry_asynchronously_without_tokio() {
    let server = Server::new(&[
        (
            "https://raw.githubusercontent.com/acme/widgets/v1/src/main.ts",
            Reply::ok(
                r#"
import { value } from "./lib/dep.ts";
if (value !== 3) throw new Error("async GitHub graph failed");
"#,
            ),
        ),
        (
            "https://raw.githubusercontent.com/acme/widgets/v1/src/lib/dep.ts",
            Reply::ok("export const value: number = 3;\n"),
        ),
    ]);
    let mut runtime = runtime(&server);

    block_on_std(runtime.run_url_async("github:acme/widgets@v1/src/main.ts"))
        .expect("asynchronous GitHub entry");

    assert_eq!(
        server.seen(),
        [
            "https://raw.githubusercontent.com/acme/widgets/v1/src/main.ts",
            "https://raw.githubusercontent.com/acme/widgets/v1/src/lib/dep.ts",
        ]
    );
}

#[test]
fn the_request_budget_applies_through_github_loading() {
    let server = Server::new(&[
        (
            "https://raw.githubusercontent.com/acme/widgets/v1/main.js",
            Reply::ok("import \"./dep.js\";\n"),
        ),
        (
            "https://raw.githubusercontent.com/acme/widgets/v1/dep.js",
            Reply::ok("export const value = 1;\n"),
        ),
    ]);
    let options = RemoteOptions::new().with_max_requests(1).without_cache();
    let mut runtime = Runtime::with_remote_transport(options, server.transport()).expect("runtime");

    let error = runtime
        .run_url("gh:acme/widgets@v1/main.js")
        .expect_err("the second request exceeds the budget");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(error.to_string().contains("1 request limit"), "{error}");
    assert_eq!(
        server.seen(),
        ["https://raw.githubusercontent.com/acme/widgets/v1/main.js"]
    );
}

#[test]
fn the_github_alias_and_the_legacy_spelling_reach_the_same_url() {
    let server = Server::new(&[(
        "https://raw.githubusercontent.com/acme/widgets/v1/src/main.js",
        Reply::ok("export const value = 1;\n"),
    )]);

    runtime(&server)
        .run_url("github:acme/widgets@v1/src/main.js")
        .expect("github alias");

    let server = Server::new(&[(
        "https://raw.githubusercontent.com/acme/widgets/v1/src/main.js",
        Reply::ok("export const value = 1;\n"),
    )]);
    runtime(&server)
        .run_url("gh:acme/widgets/src/main.js/@v1")
        .expect("legacy gh spelling");
    assert_eq!(
        server.seen(),
        ["https://raw.githubusercontent.com/acme/widgets/v1/src/main.js"]
    );
}

#[test]
fn both_schemes_share_one_cache_entry_by_raw_url() {
    let cache = tempfile::tempdir().expect("cache dir");
    let raw = "https://raw.githubusercontent.com/acme/widgets/v1/src/main.js";
    let server = Server::new(&[(raw, Reply::cacheable("export const value = 1;\n"))]);
    let options = RemoteOptions::new().with_cache_dir(cache.path());

    Runtime::with_remote_transport(options.clone(), server.transport())
        .expect("runtime")
        .run_url("gh:acme/widgets@v1/src/main.js")
        .expect("gh entry");
    Runtime::with_remote_transport(options, server.transport())
        .expect("runtime")
        .run_url("github:acme/widgets@v1/src/main.js")
        .expect("github alias uses the cached raw URL");

    assert_eq!(server.seen(), [raw]);
}

#[test]
fn a_local_module_can_import_both_schemes() {
    let server = Server::new(&[
        (
            "https://raw.githubusercontent.com/acme/a/v1/a.js",
            Reply::ok("export const a = \"a\";\n"),
        ),
        (
            "https://raw.githubusercontent.com/acme/b/v2/b.js",
            Reply::ok("export const b = \"b\";\n"),
        ),
    ]);
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { a } from "gh:acme/a@v1/a.js";
import { b } from "github:acme/b@v2/b.js";
if (`${a}${b}` !== "ab") throw new Error("github imports failed");
"#,
    )
    .expect("write source");

    let mut runtime = runtime(&server);
    runtime.run_file(&file).expect("local imports GitHub");

    assert_eq!(
        server.seen(),
        [
            "https://raw.githubusercontent.com/acme/a/v1/a.js",
            "https://raw.githubusercontent.com/acme/b/v2/b.js",
        ]
    );
}

#[test]
fn a_remote_module_can_import_a_github_specifier() {
    let server = Server::new(&[
        (
            "https://mods.test/main.js",
            Reply::ok(
                r#"
import { answer } from "github:acme/widgets@v1/src/answer.ts";
if (answer !== 42) throw new Error("remote github import failed");
"#,
            ),
        ),
        (
            "https://raw.githubusercontent.com/acme/widgets/v1/src/answer.ts",
            Reply::ok("export const answer: number = 42;\n"),
        ),
    ]);

    runtime(&server)
        .run_url("https://mods.test/main.js")
        .expect("a remote module may import GitHub");

    assert_eq!(
        server.seen(),
        [
            "https://mods.test/main.js",
            "https://raw.githubusercontent.com/acme/widgets/v1/src/answer.ts",
        ]
    );
}

#[test]
fn github_loading_is_disabled_without_an_opt_in() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("main.js");
    fs::write(&file, "import \"gh:acme/widgets@v1/main.js\";\n").expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("import must fail");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(error.to_string().contains("disabled"), "{error}");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_url("gh:acme/widgets@v1/main.js")
        .expect_err("entry must fail");
    assert!(matches!(error, Error::RemoteDisabled), "{error:?}");
}

#[test]
fn malformed_github_specifiers_fail_before_any_request() {
    let server = Server::new(&[]);

    for entry in [
        "gh:acme/widgets@v1",
        "gh:acme/widgets@v1/",
        "gh:acme/widgets@v1/../secret.ts",
        "gh:acme/widgets@v1/%2e%2e/secret.ts",
        "gh:acme/widgets@v1/a\\b.ts",
        "github:acme/widgets@v1/main.js?token=secret",
    ] {
        let error = runtime(&server)
            .run_url(entry)
            .expect_err("entry must fail");
        let Error::RemoteEntry { url, reason } = error else {
            panic!("{entry}: unexpected error: {error:?}");
        };
        assert!(
            reason.contains("GitHub specifier is invalid"),
            "{entry}: {reason}"
        );
        assert!(!reason.contains("token"), "{entry}: {reason}");
        assert!(!url.contains("token"), "{entry}: {url}");
    }

    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("main.js");
    fs::write(&file, "import \"gh:acme/widgets@v1/../secret.ts\";\n").expect("write source");
    let mut runtime = runtime(&server);
    let error = runtime.run_file(&file).expect_err("import must fail");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(
        error.to_string().contains("GitHub specifier is invalid"),
        "{error}"
    );
    assert_eq!(server.seen(), Vec::<String>::new());
}

#[test]
fn a_malformed_github_specifier_is_never_echoed_in_an_error() {
    let server = Server::new(&[]);

    // Entry path: neither the placeholder URL nor the reason may contain the
    // input, which `url` treats as an opaque path and so cannot redact.
    let error = runtime(&server)
        .run_url("gh:user:secret@host/repo@v1/main.js")
        .expect_err("must fail");
    let Error::RemoteEntry { url, reason } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert!(!reason.contains("secret"), "{reason}");
    assert!(!url.contains("secret"), "{url}");
    assert_eq!(url, "<invalid GitHub specifier>");
    assert_eq!(server.seen(), Vec::<String>::new());

    // Import path: the same redaction must hold for a local and a remote
    // importer, for a credential-shaped owner, an encoded traversal, and a
    // query-like token in the path.
    let dir = tempfile::tempdir().expect("dir");
    for (name, specifier) in [
        ("credential.js", "gh:user:secret@host/repo@v1/main.js"),
        ("traversal.js", "github:acme/widgets@v1/%2e%2e/secret.ts"),
        ("query.js", "github:acme/widgets@v1/main.js?token=secret"),
    ] {
        let file = dir.path().join(name);
        fs::write(&file, format!("import \"{specifier}\";\n")).expect("write source");
        let mut runtime = runtime(&server);
        let error = runtime.run_file(&file).expect_err("import must fail");
        let Error::Core(error) = error else {
            panic!("{name}: unexpected error: {error:?}");
        };
        let message = error.to_string();
        assert!(
            message.contains("GitHub specifier is invalid"),
            "{name}: {message}"
        );
        assert!(!message.contains("secret"), "{name}: {message}");
        assert!(!message.contains("token"), "{name}: {message}");
    }

    // Remote importer: a fetched module may import the same malformed
    // specifier. Only the entry request may happen; the malformed import must
    // fail before a raw URL is ever requested, and the error must not echo the
    // input.
    let remote = Server::new(&[(
        "https://mods.test/main.js",
        Reply::ok("import \"gh:user:secret@host/repo@v1/main.js\";\n"),
    )]);
    let mut runtime = runtime(&remote);
    let error = runtime
        .run_url("https://mods.test/main.js")
        .expect_err("the remote import must fail");
    let Error::Core(error) = error else {
        panic!("unexpected error: {error:?}");
    };
    let message = error.to_string();
    assert!(message.contains("GitHub specifier is invalid"), "{message}");
    assert!(!message.contains("secret"), "{message}");
    assert!(!message.contains("token"), "{message}");
    assert_eq!(remote.seen(), ["https://mods.test/main.js"]);

    assert_eq!(server.seen(), Vec::<String>::new());
}
