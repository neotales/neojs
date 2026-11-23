//! End-to-end tests for opt-in `jsr:` module loading.
//!
//! The HTTPS transport is scripted: every request is answered from an in-memory
//! table, no network is used, and no host is real. Request URLs use the public
//! registry's shapes, so the production URL builder, resolver, integrity
//! checks, and cache path are exercised through real V8 evaluation.

#![cfg(feature = "test-support")]

use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use neojs_core::Error;
use neojs_core::RemoteOptions;
use neojs_core::Runtime;
use neojs_core::test_support::TestRemoteResponse;
use sha2::Digest;
use sha2::Sha256;

const REGISTRY: &str = "https://jsr.io";

#[derive(Clone)]
struct Reply {
    status: u16,
    location: Option<&'static str>,
    cache_control: Option<&'static str>,
    body: Vec<u8>,
}

impl Reply {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            location: None,
            cache_control: Some("max-age=3600"),
            body: body.into(),
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            location: None,
            cache_control: None,
            body: Vec::new(),
        }
    }

    fn redirect(location: &'static str) -> Self {
        Self {
            status: 302,
            location: Some(location),
            cache_control: None,
            body: Vec::new(),
        }
    }
}

/// A scripted HTTPS server keyed by the full request URL.
struct Server {
    replies: Mutex<HashMap<String, Reply>>,
    seen: Mutex<Vec<String>>,
    fail_if_called: bool,
}

impl Server {
    fn new() -> Self {
        Self {
            replies: Mutex::new(HashMap::new()),
            seen: Mutex::new(Vec::new()),
            fail_if_called: false,
        }
    }

    /// A server that fails the test if the transport is asked for anything.
    fn no_network() -> Self {
        Self {
            fail_if_called: true,
            ..Self::new()
        }
    }

    fn with(self, url: String, reply: Reply) -> Self {
        self.replies.lock().expect("reply lock").insert(url, reply);
        self
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
            if server.fail_if_called {
                return Err("the scripted transport must not be called".to_string());
            }
            let reply = server
                .replies
                .lock()
                .expect("reply lock")
                .get(url)
                .cloned()
                .ok_or_else(|| format!("no scripted reply for {url}"))?;
            Ok(TestRemoteResponse {
                status: reply.status,
                location: reply.location.map(str::to_owned),
                declared_length: None,
                etag: None,
                last_modified: None,
                cache_control: reply.cache_control.map(str::to_owned),
                date: None,
                age: None,
                body: reply.body,
            })
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("seen lock").clone()
    }

    fn request_count(&self, url: &str) -> usize {
        self.seen().iter().filter(|seen| *seen == url).count()
    }
}

fn meta_url(scope: &str, name: &str) -> String {
    format!("{REGISTRY}/@{scope}/{name}/meta.json")
}

fn version_url(scope: &str, name: &str, version: &str) -> String {
    format!("{REGISTRY}/@{scope}/{name}/{version}_meta.json")
}

fn file_url(scope: &str, name: &str, version: &str, path: &str) -> String {
    format!("{REGISTRY}/@{scope}/{name}/{version}{path}")
}

fn checksum(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut text = String::from("sha256-");
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn package_metadata(scope: &str, name: &str, versions: &[(&str, bool)]) -> String {
    let mut entries = serde_json::Map::new();
    for (version, yanked) in versions {
        entries.insert(
            (*version).to_string(),
            serde_json::json!({ "yanked": yanked }),
        );
    }
    serde_json::json!({
        "scope": scope,
        "name": name,
        "versions": entries,
    })
    .to_string()
}

fn version_metadata(files: &[(&str, &[u8])], exports: &[(&str, &str)]) -> String {
    let mut manifest = serde_json::Map::new();
    for (path, bytes) in files {
        manifest.insert(
            (*path).to_string(),
            serde_json::json!({
                "size": bytes.len(),
                "checksum": checksum(bytes),
            }),
        );
    }
    let mut entries = serde_json::Map::new();
    for (key, target) in exports {
        entries.insert((*key).to_string(), serde_json::json!(target));
    }
    serde_json::json!({
        "manifest": manifest,
        "exports": entries,
    })
    .to_string()
}

fn run_with(server: &Arc<Server>, options: RemoteOptions, source: &str) -> Result<(), Error> {
    let directory = tempfile::tempdir().expect("temp dir");
    let entry = directory.path().join("main.js");
    fs::write(&entry, source).expect("write entry");
    let mut runtime = Runtime::with_remote_transport(options, server.transport()).expect("runtime");
    runtime.run_file(&entry)
}

fn run(server: &Arc<Server>, source: &str) -> Result<(), Error> {
    run_with(server, RemoteOptions::new().without_cache(), source)
}

fn run_default(source: &str) -> Result<(), Error> {
    let directory = tempfile::tempdir().expect("temp dir");
    let entry = directory.path().join("main.js");
    fs::write(&entry, source).expect("write entry");
    Runtime::new().expect("runtime").run_file(&entry)
}

fn error_message(result: Result<(), Error>) -> String {
    match result {
        Ok(()) => panic!("expected an error"),
        Err(error) => error.to_string(),
    }
}

const MOD_TS: &[u8] = b"import { isDigit } from \"chars\";\nimport { helper } from \"./lib/helper.ts\";\ninterface Box { value: number }\nexport const value: number = helper();\nexport const label: string = \"typed\";\nexport const builtin: boolean = isDigit(55);\n";
const HELPER_TS: &[u8] = b"export function helper(): number { return 7; }\n";
const JOIN_JS: &[u8] = b"export function join(a, b) { return a + b; }\n";

// Integration test through the actual runtime and the real registry URL
// builder: a range, an exact version, a subpath, a dynamic import, erasable
// TypeScript, a built-in import, and a relative TypeScript dependency.
#[test]
fn loads_jsr_entrypoints_subpaths_and_relative_typescript() {
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.1.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.1.0"),
                Reply::ok(version_metadata(
                    &[
                        ("/mod.ts", MOD_TS),
                        ("/lib/helper.ts", HELPER_TS),
                        ("/join.js", JOIN_JS),
                    ],
                    &[(".", "./mod.ts"), ("./join", "./join.js")],
                )),
            )
            .with(
                file_url("scope", "pkg", "1.1.0", "/mod.ts"),
                Reply::ok(MOD_TS),
            )
            .with(
                file_url("scope", "pkg", "1.1.0", "/lib/helper.ts"),
                Reply::ok(HELPER_TS),
            )
            .with(
                file_url("scope", "pkg", "1.1.0", "/join.js"),
                Reply::ok(JOIN_JS),
            ),
    );

    run(
        &server,
        r#"
import { value, label, builtin } from "jsr:@scope/pkg@1";
const join = await import("jsr:@scope/pkg@1/join");
if (value !== 7) throw new Error(`static import returned ${value}`);
if (label !== "typed") throw new Error("type stripping failed");
if (builtin !== true) throw new Error("built-in import failed");
if (join.join("a", "b") !== "ab") throw new Error("dynamic import failed");
const resolved = import.meta.resolve("jsr:@scope/pkg@1");
if (resolved !== "https://jsr.io/@scope/pkg/1.1.0/mod.ts") {
    throw new Error(`unexpected canonical URL ${resolved}`);
}
"#,
    )
    .expect("JSR entrypoints, subpath, and relative TypeScript must load");
}

// Integration test: a range and an exact version resolve to one canonical
// module identity, a yanked version is skipped by a range but selectable
// explicitly, and a prerelease is not selected by `@1`.
#[test]
fn selects_versions_and_one_identity() {
    let versions = [
        ("1.0.0", false),
        ("1.0.1", true),
        ("1.1.0", false),
        ("2.0.0-beta.1", false),
    ];
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &versions)),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(
                    &[("/v1.js", b"export const version = \"1.0.0\";\nexport const loads = 1;\n")],
                    &[(".", "./v1.js")],
                )),
            )
            .with(
                file_url("scope", "pkg", "1.0.0", "/v1.js"),
                Reply::ok(b"export const version = \"1.0.0\";\nexport const loads = 1;\n"),
            )
            .with(
                version_url("scope", "pkg", "1.0.1"),
                Reply::ok(version_metadata(
                    &[("/v101.js", b"export const version = \"1.0.1\";\nexport const loads = 1;\n")],
                    &[(".", "./v101.js")],
                )),
            )
            .with(
                file_url("scope", "pkg", "1.0.1", "/v101.js"),
                Reply::ok(b"export const version = \"1.0.1\";\nexport const loads = 1;\n"),
            )
            .with(
                version_url("scope", "pkg", "1.1.0"),
                Reply::ok(version_metadata(
                    &[("/v11.js", b"globalThis.__jsr_loads = (globalThis.__jsr_loads ?? 0) + 1;\nexport const version = \"1.1.0\";\nexport const loads = globalThis.__jsr_loads;\n")],
                    &[(".", "./v11.js")],
                )),
            )
            .with(
                file_url("scope", "pkg", "1.1.0", "/v11.js"),
                Reply::ok(b"globalThis.__jsr_loads = (globalThis.__jsr_loads ?? 0) + 1;\nexport const version = \"1.1.0\";\nexport const loads = globalThis.__jsr_loads;\n"),
            ),
    );

    run(
        &server,
        r#"
const ranged = await import("jsr:@scope/pkg@1");
const exact = await import("jsr:@scope/pkg@1.1.0");
const older = await import("jsr:@scope/pkg@1.0.0");
const yanked = await import("jsr:@scope/pkg@1.0.1");
if (ranged.version !== "1.1.0") throw new Error(`range picked ${ranged.version}`);
if (ranged !== exact) throw new Error("the range and exact version are different modules");
if (ranged.loads !== 1) throw new Error(`the module evaluated ${ranged.loads} times`);
if (older.version !== "1.0.0") throw new Error(`exact picked ${older.version}`);
if (yanked.version !== "1.0.1") throw new Error(`yanked picked ${yanked.version}`);
const resolved = import.meta.resolve("jsr:@scope/pkg@1");
if (resolved !== "https://jsr.io/@scope/pkg/1.1.0/v11.js") {
    throw new Error(`unexpected canonical URL ${resolved}`);
}
"#,
    )
    .expect("version selection and exact identity must hold");

    assert_eq!(
        server.request_count(&meta_url("scope", "pkg")),
        1,
        "package metadata must be fetched once for the runtime"
    );
}

// Integration test: a length or checksum mismatch is reported before the
// module body executes, including when the bytes have the declared length.
#[test]
fn verifies_length_and_checksum_before_execution() {
    let original = b"export const value = 1;\n";
    let cases = [
        (
            "size",
            b"throw new Error(\"EXECUTED\");\n".as_slice(),
            "has 29 bytes, expected 24",
        ),
        (
            "checksum",
            b"export const value = 2;\n".as_slice(),
            "expected sha256-",
        ),
    ];
    for (name, body, expected) in cases {
        let server = Arc::new(
            Server::new()
                .with(
                    meta_url("scope", "pkg"),
                    Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
                )
                .with(
                    version_url("scope", "pkg", "1.0.0"),
                    Reply::ok(version_metadata(
                        &[("/mod.js", original)],
                        &[(".", "./mod.js")],
                    )),
                )
                .with(
                    file_url("scope", "pkg", "1.0.0", "/mod.js"),
                    Reply::ok(body),
                ),
        );
        let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains("JSR file verification failed"),
            "{name}: {message}"
        );
        assert!(message.contains(expected), "{name}: {message}");
        assert!(
            !message.contains("EXECUTED"),
            "{name}: verification failed after execution: {message}"
        );
    }
}

// Integration test: a missing export is reported before any file is fetched.
#[test]
fn rejects_a_missing_export() {
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(
                    &[("/mod.js", b"export const value = 1;\n")],
                    &[(".", "./mod.js")],
                )),
            ),
    );

    let message = error_message(run(&server, "import \"jsr:@scope/pkg@1/missing\";\n"));
    assert!(
        message.contains("the package version has no export `./missing`"),
        "{message}"
    );
    assert!(
        server.seen().iter().all(|url| !url.ends_with("/mod.js")),
        "a missing export must not fetch a file: {:?}",
        server.seen()
    );
}

// Integration test: a registry module cannot leave its package version, the
// registry origin, or the module graph through a relative or file import.
#[test]
fn rejects_relative_imports_that_leave_the_package_version() {
    let cases = [
        ("../../evil.js", "leaves the package version directory"),
        ("/evil.js", "leaves the package version directory"),
        ("//evil.test/evil.js", "leaves the registry origin"),
        ("file:///etc/hosts", "cannot import a local file"),
        ("./%2e%2e/evil.js", "not accepted in a JSR relative import"),
    ];
    for (specifier, expected) in cases {
        let body = format!("import \"{specifier}\";\n");
        let server = Arc::new(
            Server::new()
                .with(
                    meta_url("scope", "pkg"),
                    Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
                )
                .with(
                    version_url("scope", "pkg", "1.0.0"),
                    Reply::ok(version_metadata(
                        &[
                            ("/mod.js", body.as_bytes()),
                            ("/evil.js", b"export const evil = 1;\n"),
                        ],
                        &[(".", "./mod.js")],
                    )),
                )
                .with(
                    file_url("scope", "pkg", "1.0.0", "/mod.js"),
                    Reply::ok(body.as_bytes()),
                )
                .with(
                    file_url("scope", "pkg", "1.0.0", "/evil.js"),
                    Reply::ok(b"export const evil = 1;\n"),
                ),
        );
        let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains(expected),
            "`{specifier}` produced `{message}`, expected `{expected}`"
        );
    }
}

// Integration test: the user-profile remote cache serves JSR metadata and files
// on a later runtime without contacting the transport again.
#[test]
fn reuses_the_remote_cache_without_a_request() {
    let module = b"export const value = 42;\n";
    let build = || {
        Arc::new(
            Server::new()
                .with(
                    meta_url("scope", "pkg"),
                    Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
                )
                .with(
                    version_url("scope", "pkg", "1.0.0"),
                    Reply::ok(version_metadata(
                        &[("/mod.js", module)],
                        &[(".", "./mod.js")],
                    )),
                )
                .with(
                    file_url("scope", "pkg", "1.0.0", "/mod.js"),
                    Reply::ok(module),
                ),
        )
    };
    let cache = tempfile::tempdir().expect("cache dir");
    let options = || RemoteOptions::new().with_cache_dir(cache.path());

    let first = build();
    run_with(
        &first,
        options(),
        "import { value } from \"jsr:@scope/pkg@1\";\nif (value !== 42) throw new Error(\"bad value\");\n",
    )
    .expect("the first run must load over the transport");

    let second = Arc::new(Server::no_network());
    run_with(
        &second,
        options(),
        "import { value } from \"jsr:@scope/pkg@1\";\nif (value !== 42) throw new Error(\"bad value\");\n",
    )
    .expect("a fresh runtime must reuse the cache");
    assert!(
        second.seen().is_empty(),
        "the cached run contacted the transport: {:?}",
        second.seen()
    );
}

// Integration test: package metadata for another package is rejected before a
// version or file is fetched.
#[test]
fn rejects_package_metadata_for_another_package() {
    let server = Arc::new(Server::new().with(
        meta_url("scope", "pkg"),
        Reply::ok(package_metadata("other", "pkg", &[("1.0.0", false)])),
    ));

    let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
    assert!(
        message.contains("registry returned metadata for `@other/pkg`"),
        "{message}"
    );
    assert!(
        message.contains("when `@scope/pkg` was requested"),
        "{message}"
    );
}

// Integration test: a missing package is reported as a missing registry
// document, not as a transport failure.
#[test]
fn reports_a_missing_package() {
    let server = Arc::new(Server::new().with(meta_url("scope", "pkg"), Reply::status(404)));

    let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
    assert!(
        message.contains("the registry has no package metadata for `@scope/pkg`"),
        "{message}"
    );
}

// Integration test: a redirect from a JSR registry request is subject to the
// remote client's HTTPS-only policy, and a downgrade never reaches the target.
#[test]
fn rejects_a_downgrade_redirect_for_jsr_metadata() {
    let server = Arc::new(Server::new().with(
        meta_url("scope", "pkg"),
        Reply::redirect("http://jsr.io/@scope/pkg/meta.json"),
    ));

    let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
    assert!(message.contains("refusing a redirect"), "{message}");
    assert!(message.contains("https"), "{message}");
}

// Integration test: a file: redirect target is refused before it is requested.
#[test]
fn rejects_a_file_redirect_for_jsr_metadata() {
    let server = Arc::new(Server::new().with(
        meta_url("scope", "pkg"),
        Reply::redirect("file:///etc/passwd"),
    ));

    let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
    assert!(message.contains("refusing a redirect"), "{message}");
    assert!(message.contains("file"), "{message}");
}

// Integration test: the default runtime still rejects `jsr:` and names the
// remote opt-in.
#[test]
fn the_default_runtime_rejects_jsr() {
    let message = error_message(run_default("import \"jsr:@scope/pkg@1\";\n"));
    assert!(
        message.contains("jsr: modules require remote module loading"),
        "{message}"
    );
}
