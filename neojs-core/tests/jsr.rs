//! End-to-end tests for opt-in `jsr:` module loading.
//!
//! Most tests use scripted responses: every request is answered from an
//! in-memory table, no network is used, and no host is real. Request URLs use
//! the public registry's shapes, so the production URL builder, resolver,
//! integrity checks, and cache path are exercised through real V8 evaluation.
//!
//! The `neotales` tests use unmodified captures of public `jsr.io` metadata and
//! source. See `tests/fixtures/jsr/neotales/README.md` for source URLs and
//! SHA-256 values. They run offline through the same scripted transport.

#![cfg(feature = "test-support")]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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
    sequences: Mutex<HashMap<String, VecDeque<Reply>>>,
    seen: Mutex<Vec<String>>,
    fail_if_called: bool,
}

impl Server {
    fn new() -> Self {
        Self {
            replies: Mutex::new(HashMap::new()),
            sequences: Mutex::new(HashMap::new()),
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

    /// Answers successive requests for one URL from a queue.
    fn with_sequence(self, url: String, replies: Vec<Reply>) -> Self {
        self.sequences
            .lock()
            .expect("sequence lock")
            .insert(url, replies.into());
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
            let queued = server
                .sequences
                .lock()
                .expect("sequence lock")
                .get_mut(url)
                .and_then(VecDeque::pop_front);
            let reply = match queued {
                Some(reply) => reply,
                None => server
                    .replies
                    .lock()
                    .expect("reply lock")
                    .get(url)
                    .cloned()
                    .ok_or_else(|| format!("no scripted reply for {url}"))?,
            };
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

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn checksum(bytes: &[u8]) -> String {
    format!("sha256-{}", hex(&Sha256::digest(bytes)))
}

/// Writes one entry in the remote cache's on-disk format.
///
/// This intentionally lets a test rewrite both a manifest and a body so the
/// documented cache limitation can be exercised.
fn write_cache_entry(dir: &Path, url: &str, body: &[u8]) {
    let name = hex(&Sha256::digest(url.as_bytes()));
    let stored = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs();
    let metadata = format!(
        "stored={stored}\ninitial-age=0\nbody-length={}\nbody-sha256={}\ngeneration=test\nrepresentation={}\nmax-age=3600\n",
        body.len(),
        hex(&Sha256::digest(body)),
        hex(b"test"),
    );
    let mut entry = format!("neojs-remote-cache 5\n{}\n{metadata}", metadata.len()).into_bytes();
    entry.extend_from_slice(body);
    std::fs::write(dir.join(name), entry).expect("write cache entry");
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

// Real registry fixtures captured from the public `jsr.io` origin. See
// `tests/fixtures/jsr/neotales/README.md` for source URLs and SHA-256 values.
// They are unmodified captures, not hand-written documents.
const REAL_CHARS_META: &str = include_str!("fixtures/jsr/neotales/chars/meta.json");
const REAL_CHARS_VERSION_META: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0_meta.json");
const REAL_CHARS_IS_DIGIT: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/is_digit.ts");
const REAL_CHARS_ND: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/tables/nd.ts");
const REAL_CHARS_LATIN1: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/tables/latin1.ts");
const REAL_CHARS_TO_LOWER: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/to_lower.ts");
const REAL_CHARS_TO_UPPER: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/to_upper.ts");
const REAL_CHARS_CASE: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/tables/case.ts");
const REAL_CHARS_CONSTANTS: &str = include_str!("fixtures/jsr/neotales/chars/0.0.0/constants.ts");
const REAL_SLICES_META: &str = include_str!("fixtures/jsr/neotales/slices/meta.json");
const REAL_SLICES_VERSION_META: &str = include_str!("fixtures/jsr/neotales/slices/0.0.0_meta.json");
const REAL_SLICES_CAPITALIZE: &str =
    include_str!("fixtures/jsr/neotales/slices/0.0.0/capitalize.ts");
const REAL_SLICES_UTILS: &str = include_str!("fixtures/jsr/neotales/slices/0.0.0/utils.ts");

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

// Integration test: a JSR module can import another JSR package, and each
// package's metadata is fetched once for the runtime.
#[test]
fn loads_a_jsr_dependency() {
    let dep = b"export const dep = 5;\n";
    let root = b"import { dep } from \"jsr:@scope/dep@1\";\nexport const value = dep + 1;\n";
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", root)], &[(".", "./mod.js")])),
            )
            .with(
                file_url("scope", "pkg", "1.0.0", "/mod.js"),
                Reply::ok(root),
            )
            .with(
                meta_url("scope", "dep"),
                Reply::ok(package_metadata("scope", "dep", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "dep", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", dep)], &[(".", "./mod.js")])),
            )
            .with(file_url("scope", "dep", "1.0.0", "/mod.js"), Reply::ok(dep)),
    );

    run(
        &server,
        "import { value } from \"jsr:@scope/pkg@1\";\nif (value !== 6) throw new Error(`bad value ${value}`);\n",
    )
    .expect("a JSR package may import another JSR package");
    assert_eq!(server.request_count(&meta_url("scope", "pkg")), 1);
    assert_eq!(server.request_count(&meta_url("scope", "dep")), 1);
}

// Integration test with an unmodified public-registry fixture: a real subpath
// entrypoint, its relative dependencies, and type-only imports that must not be
// fetched at runtime.
#[test]
fn loads_a_real_registry_package_subpath() {
    let server = Arc::new(
        Server::new()
            .with(meta_url("neotales", "chars"), Reply::ok(REAL_CHARS_META))
            .with(
                version_url("neotales", "chars", "0.0.0"),
                Reply::ok(REAL_CHARS_VERSION_META),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/is_digit.ts"),
                Reply::ok(REAL_CHARS_IS_DIGIT),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/tables/nd.ts"),
                Reply::ok(REAL_CHARS_ND),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/tables/latin1.ts"),
                Reply::ok(REAL_CHARS_LATIN1),
            ),
    );

    run(
        &server,
        r#"
import { isDigit } from "jsr:@neotales/chars@0.0.0/is-digit";
if (!isDigit(0x31)) throw new Error("did not detect a digit");
if (isDigit(0x41)) throw new Error("detected a letter as a digit");
const resolved = import.meta.resolve("jsr:@neotales/chars@0.0.0/is-digit");
if (resolved !== "https://jsr.io/@neotales/chars/0.0.0/is_digit.ts") {
    throw new Error(`unexpected canonical URL ${resolved}`);
}
"#,
    )
    .expect("the real chars subpath must load");
    assert!(
        server.seen().iter().all(|url| !url.ends_with("/types.ts")),
        "a type-only import must not be fetched: {:?}",
        server.seen()
    );
}

// Integration test with unmodified public-registry fixtures: a real package's
// `jsr:` dependency on another package, its version range, and the relative
// graph inside both packages.
#[test]
fn loads_a_real_cross_package_jsr_graph() {
    let server = Arc::new(
        Server::new()
            .with(meta_url("neotales", "slices"), Reply::ok(REAL_SLICES_META))
            .with(
                version_url("neotales", "slices", "0.0.0"),
                Reply::ok(REAL_SLICES_VERSION_META),
            )
            .with(
                file_url("neotales", "slices", "0.0.0", "/capitalize.ts"),
                Reply::ok(REAL_SLICES_CAPITALIZE),
            )
            .with(
                file_url("neotales", "slices", "0.0.0", "/utils.ts"),
                Reply::ok(REAL_SLICES_UTILS),
            )
            .with(meta_url("neotales", "chars"), Reply::ok(REAL_CHARS_META))
            .with(
                version_url("neotales", "chars", "0.0.0"),
                Reply::ok(REAL_CHARS_VERSION_META),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/to_lower.ts"),
                Reply::ok(REAL_CHARS_TO_LOWER),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/to_upper.ts"),
                Reply::ok(REAL_CHARS_TO_UPPER),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/tables/case.ts"),
                Reply::ok(REAL_CHARS_CASE),
            )
            .with(
                file_url("neotales", "chars", "0.0.0", "/constants.ts"),
                Reply::ok(REAL_CHARS_CONSTANTS),
            ),
    );

    run(
        &server,
        r#"
import { capitalize } from "jsr:@neotales/slices@0.0.0/capitalize";
const capitalized = String.fromCodePoint(...capitalize("hello"));
if (capitalized !== "Hello") throw new Error(`capitalize returned ${capitalized}`);
const resolved = import.meta.resolve("jsr:@neotales/slices@0.0.0/capitalize");
if (resolved !== "https://jsr.io/@neotales/slices/0.0.0/capitalize.ts") {
    throw new Error(`unexpected canonical URL ${resolved}`);
}
"#,
    )
    .expect("the real cross-package JSR graph must load");
    assert_eq!(
        server.request_count(&meta_url("neotales", "chars")),
        1,
        "the dependency's package metadata must be fetched once"
    );
}

// Integration test: a direct `https://jsr.io/...` import from a local module
// keeps the ordinary HTTPS loader's semantics. It must not fetch registry
// metadata or verify the body against a manifest.
#[test]
fn a_direct_https_registry_url_keeps_ordinary_remote_semantics() {
    let url = file_url("neotales", "chars", "0.0.0", "/mod.ts");
    let body = b"export const value = 7;\n";
    let server = Arc::new(Server::new().with(url.clone(), Reply::ok(body.as_slice())));
    let source = format!(
        "import {{ value }} from \"{url}\";\nif (value !== 7) throw new Error(`bad value ${{value}}`);\n"
    );

    run(&server, &source).expect("a direct HTTPS registry URL must load as ordinary remote");
    assert_eq!(
        server.seen(),
        vec![url],
        "a direct HTTPS import must make exactly its own request"
    );
}

// Integration test: `run_url` fetches its prepared entry once and the loader
// consumes it. A one-request budget proves a direct jsr.io entry does not fetch
// registry metadata or the file a second time, even with the cache disabled.
#[test]
fn run_url_reuses_the_prepared_registry_entry_with_one_request() {
    let url = file_url("neotales", "chars", "0.0.0", "/mod.ts");
    let body = b"export const value = 9;\n";
    let server = Arc::new(Server::new().with(url.clone(), Reply::ok(body.as_slice())));
    let mut runtime = Runtime::with_remote_transport(
        RemoteOptions::new().without_cache().with_max_requests(1),
        server.transport(),
    )
    .expect("runtime");

    runtime
        .run_url(&url)
        .expect("the prepared entry must load with one request");
    assert_eq!(
        server.seen(),
        vec![url],
        "the prepared entry must not be fetched again"
    );
}

// Integration test: an explicit jsr.io URL imported by a JSR graph module is
// verified against the manifest. An unlisted file and a non-canonical URL are
// both refused instead of falling through to the ordinary remote loader.
#[test]
fn a_jsr_graph_import_cannot_bypass_manifest_verification() {
    let cases = [
        (
            "https://jsr.io/@scope/pkg/1.0.0/evil.js",
            "not listed in the manifest",
        ),
        (
            "https://jsr.io/@scope/pkg/1.0.0/evil.js?x=1",
            "canonical https://jsr.io module URL",
        ),
    ];
    for (target, expected) in cases {
        let root = format!("import \"{target}\";\n");
        let root_bytes = root.as_bytes();
        let server = Arc::new(
            Server::new()
                .with(
                    meta_url("scope", "pkg"),
                    Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
                )
                .with(
                    version_url("scope", "pkg", "1.0.0"),
                    Reply::ok(version_metadata(
                        &[("/root.js", root_bytes)],
                        &[(".", "./root.js")],
                    )),
                )
                .with(
                    file_url("scope", "pkg", "1.0.0", "/root.js"),
                    Reply::ok(root_bytes),
                )
                .with(
                    file_url("scope", "pkg", "1.0.0", "/evil.js"),
                    Reply::ok(b"throw new Error(\"EVIL_EXECUTED\");\n".as_slice()),
                ),
        );

        let message = error_message(run(&server, "import \"jsr:@scope/pkg@1\";\n"));
        assert!(
            message.contains(expected),
            "`{target}` produced `{message}`, expected `{expected}`"
        );
        assert!(!message.contains("EVIL_EXECUTED"), "{message}");
        assert!(
            server.seen().iter().all(|url| !url.contains("evil.js")),
            "the unverified target must not be fetched: {:?}",
            server.seen()
        );
    }
}

// Integration test: after a canonical jsr.io URL has been loaded as an ordinary
// HTTPS module, a `jsr:` import of the same identity is refused. Reusing the
// unverified module would silently bypass manifest verification.
#[test]
fn a_jsr_import_cannot_reuse_an_unverified_ordinary_registry_module() {
    let url = file_url("neotales", "chars", "0.0.0", "/mod.ts");
    let body = b"import \"jsr:@neotales/chars@0.0.0\";\n";
    let server = Arc::new(
        Server::new()
            .with(url.clone(), Reply::ok(body.as_slice()))
            .with(meta_url("neotales", "chars"), Reply::ok(REAL_CHARS_META))
            .with(
                version_url("neotales", "chars", "0.0.0"),
                Reply::ok(REAL_CHARS_VERSION_META),
            ),
    );
    let source = format!("import \"{url}\";\n");

    let message = error_message(run(&server, &source));
    assert!(
        message.contains("already loaded as an ordinary HTTPS module"),
        "{message}"
    );
}

// Integration test: once a canonical jsr.io URL is admitted by `jsr:`, a later
// direct HTTPS import of the same URL reuses the verified module, and a later
// `jsr:` import is not rejected as a false collision.
#[test]
fn a_jsr_import_can_follow_a_direct_https_import_of_a_verified_url() {
    let url = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 1;\n";
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            )
            .with(url.clone(), Reply::ok(body.as_slice())),
    );

    run(
        &server,
        &format!(
            r#"
const first = await import("jsr:@scope/pkg@1");
const direct = await import("{url}");
const second = await import("jsr:@scope/pkg@1");
if (first !== direct) throw new Error("the direct import is a different module");
if (direct !== second) throw new Error("the second jsr: import is a different module");
if (first.value !== 1) throw new Error(`bad value ${{first.value}}`);
"#
        ),
    )
    .expect("a direct import of an already verified JSR URL must reuse the module");
}

// Integration test: an ordinary HTTPS module that redirects to a canonical
// jsr.io URL is registered under the final URL. A later `jsr:` import of that
// identity is refused, so it cannot reuse the unverified redirect body.
#[test]
fn an_ordinary_redirect_to_a_registry_url_cannot_be_reused_by_jsr() {
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 1;\n";
    let server = Arc::new(
        Server::new()
            .with(
                "https://elsewhere.test/entry.js".to_string(),
                Reply::redirect("https://jsr.io/@scope/pkg/1.0.0/mod.js"),
            )
            .with(target.clone(), Reply::ok(body.as_slice()))
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            ),
    );

    let message = error_message(run(
        &server,
        r#"
const direct = await import("https://elsewhere.test/entry.js");
if (direct.value !== 1) throw new Error("the ordinary redirect did not load");
await import("jsr:@scope/pkg@1");
"#,
    ));
    assert!(
        message.contains("already loaded as an ordinary HTTPS module"),
        "{message}"
    );
    assert_eq!(
        server.request_count(&target),
        1,
        "the jsr: import must not fetch the unverified redirect target again"
    );
}

// Integration test: a prepared `run_url` entry that redirects to a canonical
// jsr.io URL registers under the final URL. Its `jsr:` import of that identity
// is refused. An exact request budget proves the prepared entry is not fetched
// again.
#[test]
fn a_prepared_entry_redirect_cannot_be_reused_by_jsr() {
    let requested = "https://elsewhere.test/entry.js";
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"import \"jsr:@scope/pkg@1\";\n";
    let server = Arc::new(
        Server::new()
            .with(
                requested.to_string(),
                Reply::redirect("https://jsr.io/@scope/pkg/1.0.0/mod.js"),
            )
            .with(target.clone(), Reply::ok(body.as_slice()))
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            ),
    );
    let mut runtime = Runtime::with_remote_transport(
        RemoteOptions::new().without_cache().with_max_requests(4),
        server.transport(),
    )
    .expect("runtime");

    let error = runtime
        .run_url(requested)
        .expect_err("the jsr: import must not reuse the prepared redirect body");
    let message = error.to_string();
    assert!(
        message.contains("already loaded as an ordinary HTTPS module"),
        "{message}"
    );
    assert_eq!(
        server.seen(),
        vec![
            requested.to_string(),
            target.clone(),
            meta_url("scope", "pkg"),
            version_url("scope", "pkg", "1.0.0"),
        ],
        "the prepared entry must be fetched once and the request budget preserved"
    );
}

// Integration test: the opposite order is also safe. After a canonical jsr.io
// URL is verified, an ordinary redirect to that identity is refused before V8
// can register a second module there, and the verified module remains intact.
#[test]
fn an_ordinary_redirect_to_an_already_verified_jsr_url_is_refused() {
    let requested = "https://elsewhere.test/entry.js";
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 1;\n";
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            )
            .with(target.clone(), Reply::ok(body.as_slice()))
            .with(
                requested.to_string(),
                Reply::redirect("https://jsr.io/@scope/pkg/1.0.0/mod.js"),
            ),
    );

    run(
        &server,
        r#"
const verified = await import("jsr:@scope/pkg@1");
let caught = null;
try {
    await import("https://elsewhere.test/entry.js");
} catch (error) {
    caught = error;
}
if (verified.value !== 1) throw new Error("the verified module was replaced");
if (caught === null) throw new Error("the ordinary redirect was not refused");
if (!String(caught).includes("already a verified JSR module")) {
    throw new Error(`unexpected refusal: ${caught}`);
}
"#,
    )
    .expect("an ordinary redirect must not replace a verified JSR module");
    assert_eq!(
        server.request_count(&target),
        2,
        "the verified load and the refused ordinary redirect each fetch the target once"
    );
    assert_eq!(
        server.request_count(requested),
        1,
        "the ordinary redirect must be fetched once"
    );
}

// Integration test: `import.meta.resolve` on an ordinary jsr.io URL does not
// load it. It must not register an ordinary identity that blocks a later `jsr:`
// import.
#[test]
fn an_import_meta_resolve_of_an_ordinary_registry_url_does_not_block_jsr() {
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 1;\n";
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            )
            .with(target.clone(), Reply::ok(body.as_slice())),
    );

    run(
        &server,
        r#"
const resolved = import.meta.resolve("https://jsr.io/@scope/pkg/1.0.0/mod.js");
if (resolved !== "https://jsr.io/@scope/pkg/1.0.0/mod.js") {
    throw new Error(`unexpected resolution ${resolved}`);
}
const jsr = await import("jsr:@scope/pkg@1");
if (jsr.value !== 1) throw new Error("the jsr: import was blocked");
"#,
    )
    .expect("an unresolved ordinary URL must not block a jsr: import");
    assert_eq!(
        server.request_count(&target),
        1,
        "only the jsr: load fetches the target"
    );
}

// Integration test: `import.meta.resolve` on a `jsr:` specifier is pending, not
// loaded. It must not block a later ordinary redirect to the same URL.
#[test]
fn an_import_meta_resolve_of_jsr_does_not_block_an_ordinary_redirect() {
    let requested = "https://elsewhere.test/entry.js";
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 5;\n";
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            )
            .with(target.clone(), Reply::ok(body.as_slice()))
            .with(
                requested.to_string(),
                Reply::redirect("https://jsr.io/@scope/pkg/1.0.0/mod.js"),
            ),
    );

    run(
        &server,
        r#"
const resolved = import.meta.resolve("jsr:@scope/pkg@1");
if (resolved !== "https://jsr.io/@scope/pkg/1.0.0/mod.js") {
    throw new Error(`unexpected resolution ${resolved}`);
}
const direct = await import("https://elsewhere.test/entry.js");
if (direct.value !== 5) throw new Error("the ordinary redirect was blocked");
"#,
    )
    .expect("a pending jsr: resolution must not block an ordinary redirect");
    assert_eq!(
        server.request_count(&target),
        1,
        "import.meta.resolve must not fetch the target"
    );
}

// Integration test: a failed ordinary load is not recorded. A later `jsr:`
// import of the same URL must load and verify normally.
#[test]
fn a_failed_ordinary_load_does_not_block_a_jsr_import() {
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 3;\n";
    let server = Arc::new(
        Server::new()
            .with_sequence(
                target.clone(),
                vec![Reply::status(404), Reply::ok(body.as_slice())],
            )
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            ),
    );

    run(
        &server,
        r#"
let failed = null;
try {
    await import("https://jsr.io/@scope/pkg/1.0.0/mod.js");
} catch (error) {
    failed = error;
}
if (failed === null) throw new Error("the ordinary load was expected to fail");
const jsr = await import("jsr:@scope/pkg@1");
if (jsr.value !== 3) throw new Error("the jsr: import was blocked by a failed load");
"#,
    )
    .expect("a failed ordinary load must not poison the jsr: identity");
    assert_eq!(
        server.request_count(&target),
        2,
        "the failed ordinary attempt and the verified jsr: load each fetch once"
    );
}

// Integration test: a failed JSR load is not marked loaded. A later ordinary
// redirect to the same URL must load as ordinary.
#[test]
fn a_failed_jsr_load_does_not_block_an_ordinary_redirect() {
    let requested = "https://elsewhere.test/entry.js";
    let target = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let declared = b"export const value = 1;\n";
    let actual = b"export const value = 4;\n";
    let server = Arc::new(
        Server::new()
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(
                    &[("/mod.js", declared)],
                    &[(".", "./mod.js")],
                )),
            )
            .with(target.clone(), Reply::ok(actual.as_slice()))
            .with(
                requested.to_string(),
                Reply::redirect("https://jsr.io/@scope/pkg/1.0.0/mod.js"),
            ),
    );

    run(
        &server,
        r#"
let failed = null;
try {
    await import("jsr:@scope/pkg@1");
} catch (error) {
    failed = error;
}
if (failed === null) throw new Error("the jsr: load was expected to fail");
const direct = await import("https://elsewhere.test/entry.js");
if (direct.value !== 4) throw new Error("the ordinary redirect was blocked");
"#,
    )
    .expect("a failed jsr: load must not poison the ordinary identity");
    assert_eq!(
        server.request_count(&target),
        2,
        "the failed verified load and the ordinary redirect each fetch once"
    );
    assert_eq!(server.request_count(requested), 1);
}

// Integration test: an ordinary redirect to a differently encoded canonical
// jsr.io path is recorded under the canonical identity, so a later `jsr:`
// import cannot reuse it.
#[test]
fn an_encoded_redirect_to_a_registry_identity_is_detected() {
    let requested = "https://elsewhere.test/entry.js";
    let encoded = "https://jsr.io/@scope/pkg/1.0.0/m%6Fd.js";
    let canonical = file_url("scope", "pkg", "1.0.0", "/mod.js");
    let body = b"export const value = 1;\n";
    let server = Arc::new(
        Server::new()
            .with(
                requested.to_string(),
                Reply::redirect("https://jsr.io/@scope/pkg/1.0.0/m%6Fd.js"),
            )
            .with(encoded.to_string(), Reply::ok(body.as_slice()))
            .with(canonical.clone(), Reply::ok(body.as_slice()))
            .with(
                meta_url("scope", "pkg"),
                Reply::ok(package_metadata("scope", "pkg", &[("1.0.0", false)])),
            )
            .with(
                version_url("scope", "pkg", "1.0.0"),
                Reply::ok(version_metadata(&[("/mod.js", body)], &[(".", "./mod.js")])),
            ),
    );

    let message = error_message(run(
        &server,
        r#"
const direct = await import("https://elsewhere.test/entry.js");
if (direct.value !== 1) throw new Error("the encoded redirect did not load");
await import("jsr:@scope/pkg@1");
"#,
    ));
    assert!(
        message.contains("already loaded as an ordinary HTTPS module"),
        "{message}"
    );
    assert_eq!(
        server.request_count(encoded) + server.request_count(&canonical),
        1,
        "the jsr: resolution must not fetch the encoded or canonical file again"
    );
}

// Integration test: the cache is local mutable state. Rewriting both the
// version metadata and the file keeps the checksum consistent, so the loader
// accepts the substituted body. This documents a limitation: the checksum
// proves consistency with the retrieved metadata, not publisher identity or
// authentication.
#[test]
fn a_consistent_cache_rewrite_is_not_authenticated() {
    let original = b"export const value = 1;\n";
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
                Reply::ok(original.as_slice()),
            ),
    );
    let cache = tempfile::tempdir().expect("cache dir");
    let options = || RemoteOptions::new().with_cache_dir(cache.path());
    run_with(
        &server,
        options(),
        "import { value } from \"jsr:@scope/pkg@1\";\nif (value !== 1) throw new Error(\"first\");\n",
    )
    .expect("the first run must load over the transport");

    let substituted = b"export const value = 2;\n";
    let substituted_metadata = version_metadata(&[("/mod.js", substituted)], &[(".", "./mod.js")]);
    write_cache_entry(
        cache.path(),
        &version_url("scope", "pkg", "1.0.0"),
        substituted_metadata.as_bytes(),
    );
    write_cache_entry(
        cache.path(),
        &file_url("scope", "pkg", "1.0.0", "/mod.js"),
        substituted,
    );

    let offline = Arc::new(Server::no_network());
    run_with(
        &offline,
        options(),
        "import { value } from \"jsr:@scope/pkg@1\";\nif (value !== 2) throw new Error(`substituted value ${value}`);\n",
    )
    .expect("a consistent cache rewrite is accepted; this is the documented limitation");
    assert!(
        offline.seen().is_empty(),
        "the cache rewrite must be served without a request: {:?}",
        offline.seen()
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
