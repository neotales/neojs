//! End-to-end tests for `neo.jsonc` `ImportResolver` integration in `neojs-core`.

use std::fs;
use std::path::Path;

use neojs_core::{Error, ImportResolver, Runtime, RuntimeOptions};
use neojs_manifest::{DiscoveredManifest, Manifest, PackageName, Workspace};
use tempfile::TempDir;

fn write(root: &Path, relative: &str, content: &str) -> std::path::PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, content).expect("write file");
    path
}

fn load_standalone_resolver(manifest_path: &Path) -> ImportResolver {
    let canonical = fs::canonicalize(manifest_path).expect("canonical manifest");
    let dir = canonical.parent().expect("manifest dir").to_path_buf();
    let manifest = Manifest::from_path(&canonical).expect("parse manifest");
    ImportResolver::from_manifest(&DiscoveredManifest {
        path: canonical,
        dir,
        manifest,
    })
}

#[test]
fn runtime_new_performs_no_implicit_manifest_discovery() {
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "neo.jsonc",
        r#"{ "name": "@test/app", "imports": { "greet": "./greet.ts" } }"#,
    );
    write(
        dir.path(),
        "greet.ts",
        "export const hello = (n: string): string => `hi ${n}`;\n",
    );
    let main = write(
        dir.path(),
        "main.ts",
        "import { hello } from 'greet';\nif (hello('world') !== 'hi world') throw new Error('bad');\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    let err = runtime
        .run_file(&main)
        .expect_err("Runtime::new must not discover neo.jsonc implicitly");
    let msg = err.to_string();
    assert!(msg.contains("bare module specifiers"), "{msg}");
}

#[test]
fn resolves_exact_and_prefix_local_imports_and_type_strips_ts() {
    let dir = TempDir::new().expect("temp dir");
    let manifest_path = write(
        dir.path(),
        "neo.jsonc",
        r#"{
            "name": "@test/app",
            "imports": {
                "greet": "./src/greet.ts",
                "lib/": "./src/lib/",
                "lib/special": "./src/special.mts"
            }
        }"#,
    );
    write(
        dir.path(),
        "src/greet.ts",
        "export function greet(name: string): string { return `hello ${name}`; }\n",
    );
    write(
        dir.path(),
        "src/lib/math.ts",
        "export const add = (a: number, b: number): number => a + b;\n",
    );
    write(
        dir.path(),
        "src/special.mts",
        "export const tag: string = 'exact-wins';\n",
    );
    let main = write(
        dir.path(),
        "main.ts",
        r#"
import { greet } from "greet";
import { add } from "lib/math.ts";
import { tag } from "lib/special";
if (greet("neo") !== "hello neo") throw new Error("greet failed");
if (add(2, 3) !== 5) throw new Error("add failed");
if (tag !== "exact-wins") throw new Error("exact precedence failed");
const resolved = import.meta.resolve("greet");
if (!resolved.startsWith("file://") || !resolved.endsWith("/src/greet.ts")) {
    throw new Error(`unexpected import.meta.resolve: ${resolved}`);
}
"#,
    );

    let resolver = load_standalone_resolver(&manifest_path);
    let mut runtime = Runtime::with_import_resolver(resolver).expect("runtime");
    runtime.run_file(&main).expect("run succeeds");
}

#[test]
fn resolves_workspace_packages_and_rejects_unexported_subpaths() {
    let dir = TempDir::new().expect("temp dir");
    let root_manifest = write(
        dir.path(),
        "neo.jsonc",
        r#"{
            "name": "@ws/root",
            "workspaces": ["packages/*"],
            "imports": {
                "shared-util": "./shared/util.ts"
            }
        }"#,
    );
    write(
        dir.path(),
        "shared/util.ts",
        "export const rootTag = 'root';\n",
    );
    write(
        dir.path(),
        "packages/a/neo.jsonc",
        r#"{
            "name": "@ws/a",
            "exports": "./mod.ts",
            "imports": {
                "local-only": "./local.ts"
            }
        }"#,
    );
    write(
        dir.path(),
        "packages/a/local.ts",
        "export const fromA = 'member-a';\n",
    );
    write(
        dir.path(),
        "packages/b/neo.jsonc",
        r#"{
            "name": "@ws/b",
            "exports": {
                ".": "./index.ts",
                "./sub": "./sub.mjs"
            }
        }"#,
    );
    write(
        dir.path(),
        "packages/b/index.ts",
        "export const bName: string = 'pkg-b';\n",
    );
    write(
        dir.path(),
        "packages/b/sub.mjs",
        "export const subVal = 99;\n",
    );
    write(
        dir.path(),
        "packages/b/secret.ts",
        "export const secret = 'nope';\n",
    );
    let entry = write(
        dir.path(),
        "packages/a/mod.ts",
        r#"
import { fromA } from "local-only";
import { rootTag } from "shared-util";
import { bName } from "@ws/b";
import { subVal } from "@ws/b/sub";
if (fromA !== "member-a" || rootTag !== "root" || bName !== "pkg-b" || subVal !== 99) {
    throw new Error("workspace resolution mismatch");
}
"#,
    );

    let workspace = Workspace::load(&root_manifest).expect("workspace");
    let active = PackageName::parse("@ws/a").expect("pkg name");
    let resolver = ImportResolver::from_workspace(&workspace, Some(&active)).expect("resolver");

    let mut runtime = Runtime::with_import_resolver(resolver.clone()).expect("runtime");
    runtime.run_file(&entry).expect("workspace imports succeed");

    let bad_entry = write(
        dir.path(),
        "packages/a/bad.ts",
        r#"import { secret } from "@ws/b/secret"; console.log(secret);"#,
    );
    let mut bad_runtime = Runtime::with_import_resolver(resolver).expect("runtime");
    let err = bad_runtime
        .run_file(&bad_entry)
        .expect_err("unexported subpath must fail");
    let msg = err.to_string();
    assert!(msg.contains("not exported"), "{msg}");
}

#[test]
fn builtins_are_never_intercepted_and_format_and_npm_checks_hold() {
    let dir = TempDir::new().expect("temp dir");
    let manifest_path = write(
        dir.path(),
        "neo.jsonc",
        r#"{
            "name": "@test/policy",
            "imports": {
                "neo:os": "./fake-os.js",
                "NEO:os": "./fake-os.js",
                "node:fs": "./fake-fs.js",
                "NODE:fs": "./fake-fs.js",
                "bad-cjs": "./legacy.cjs",
                "npm-dep": "npm:lodash@4.17.21"
            }
        }"#,
    );
    write(
        dir.path(),
        "fake-os.js",
        "export const platform = 'fake';\n",
    );
    write(
        dir.path(),
        "fake-fs.js",
        "export const readFile = () => {};\n",
    );
    write(dir.path(), "legacy.cjs", "module.exports = 1;\n");

    let resolver = load_standalone_resolver(&manifest_path);

    // `neo:os` and uppercase `NEO:os` must use real built-in, not `./fake-os.js`.
    let os_main = write(
        dir.path(),
        "os_check.ts",
        r#"
import { platform } from "neo:os";
import { platform as upperPlatform } from "NEO:os";
if (platform === "fake" || upperPlatform === "fake" || platform !== upperPlatform) {
    throw new Error("neo:os or NEO:os was shadowed by imports");
}
"#,
    );
    Runtime::with_import_resolver(resolver.clone())
        .expect("runtime")
        .run_file(&os_main)
        .expect("neo:os and NEO:os succeed");

    // `node:fs` must use the real built-in rather than `./fake-fs.js` (when
    // `node-compat` is enabled), and uppercase `NODE:fs` / unsupported `node:*`
    // specifiers remain rejected without ever falling back to `imports`.
    #[cfg(feature = "node-compat")]
    {
        let node_fs_main = write(
            dir.path(),
            "node_fs_check.ts",
            "import { existsSync } from \"node:fs\";\nif (typeof existsSync !== \"function\") throw new Error(\"node:fs shadowed\");\n",
        );
        Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&node_fs_main)
            .expect("node:fs succeeds and is not shadowed");

        let node_disabled_err = Runtime::with_options(
            neojs_core::RuntimeOptions::new()
                .with_import_resolver(resolver.clone())
                .with_node_compat(false),
        )
        .expect("runtime")
        .run_file(&node_fs_main)
        .expect_err("node:fs with --no-node-compat must be rejected");
        assert!(
            node_disabled_err.to_string().contains("Node.js built-in"),
            "{node_disabled_err}"
        );
    }

    #[cfg(not(feature = "node-compat"))]
    {
        let node_fs_main = write(dir.path(), "node_fs_disabled.ts", "import \"node:fs\";\n");
        let err = Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&node_fs_main)
            .expect_err("node:fs without node-compat feature must be rejected");
        assert!(err.to_string().contains("Node.js built-in"), "{err}");
    }

    for (file, spec) in [
        ("node_unsupported_check.ts", "node:test"),
        ("node_upper_check.ts", "NODE:fs"),
    ] {
        let node_main = write(dir.path(), file, &format!("import \"{spec}\";\n"));
        let err = Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&node_main)
            .expect_err("unsupported node: and NODE:fs must be rejected");
        assert!(err.to_string().contains("Node.js built-in"), "{err}");
    }

    // `.cjs` target must fail neojs-module-policy check.
    let cjs_main = write(dir.path(), "cjs_check.ts", r#"import "bad-cjs";"#);
    let err = Runtime::with_import_resolver(resolver.clone())
        .expect("runtime")
        .run_file(&cjs_main)
        .expect_err(".cjs must be rejected");
    assert!(err.to_string().contains(".cjs extension"), "{err}");

    // `npm:` target must fail clearly when disabled or missing from offline CAS/lockfile.
    let npm_main = write(dir.path(), "npm_check.ts", r#"import "npm-dep";"#);
    let err = Runtime::with_import_resolver(resolver)
        .expect("runtime")
        .run_file(&npm_main)
        .expect_err("npm: without cache/remote must fail");
    #[cfg(not(feature = "npm"))]
    assert!(
        err.to_string().contains("npm: modules are not implemented"),
        "{err}"
    );
    #[cfg(feature = "npm")]
    assert!(
        err.to_string().contains("offline") || err.to_string().contains("allow-remote"),
        "{err}"
    );
}

#[test]
fn remote_targets_in_imports_fail_closed_without_remote_enabled() {
    let dir = TempDir::new().expect("temp dir");
    let manifest_path = write(
        dir.path(),
        "neo.jsonc",
        r#"{
            "name": "@test/remote-gated",
            "imports": {
                "https-dep": "https://example.test/mod.ts",
                "gh-dep": "gh:owner/repo@v1.0.0/mod.ts",
                "jsr-dep": "jsr:@scope/pkg@1.0.0"
            }
        }"#,
    );
    let resolver = load_standalone_resolver(&manifest_path);

    for (name, spec) in [
        ("https.ts", "https-dep"),
        ("gh.ts", "gh-dep"),
        ("jsr.ts", "jsr-dep"),
    ] {
        let entry = write(dir.path(), name, &format!("import \"{spec}\";\n"));
        let err = Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&entry)
            .expect_err("remote target must fail closed without remote enabled");
        assert!(
            matches!(err, Error::Core(_)),
            "expected Error::Core for {spec}, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("remote module loading is disabled")
                || msg.contains("require remote module loading"),
            "unexpected error for {spec}: {msg}"
        );
    }

    // Also verify RuntimeOptions::new().with_import_resolver(...) behaves identically.
    let entry = dir.path().join("https.ts");
    let err = Runtime::with_options(RuntimeOptions::new().with_import_resolver(resolver))
        .expect("runtime")
        .run_file(&entry)
        .expect_err("RuntimeOptions without with_remote must fail closed");
    assert!(
        err.to_string()
            .contains("remote module loading is disabled"),
        "{err}"
    );
}

#[cfg(feature = "test-support")]
mod remote_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use neojs_core::RemoteOptions;
    use neojs_core::test_support::TestRemoteResponse;
    use sha2::{Digest, Sha256};

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }

    #[test]
    fn remote_targets_in_imports_succeed_when_remote_enabled_and_cannot_escape_back_to_local() {
        let dir = TempDir::new().expect("temp dir");
        let manifest_path = write(
            dir.path(),
            "neo.jsonc",
            r#"{
                "name": "@test/remote-enabled",
                "imports": {
                    "local-secret": "./secret.ts",
                    "https-dep": "https://example.test/mod.ts",
                    "gh-dep": "gh:owner/repo@v1.0.0/gh.ts",
                    "jsr-dep": "jsr:@scope/pkg@1.0.0",
                    "evil-remote": "https://example.test/evil.ts",
                    "evil-jsr": "jsr:@scope/evil@1.0.0"
                }
            }"#,
        );
        write(
            dir.path(),
            "secret.ts",
            "export const secret = 'LOCAL_SECRET';\n",
        );

        let jsr_mod_bytes = b"export const jsrVal: string = 'from-jsr';\n";
        let jsr_evil_bytes = b"import { secret } from 'local-secret'; export const x = secret;\n";

        let mut replies: HashMap<String, Vec<u8>> = HashMap::new();
        replies.insert(
            "https://example.test/mod.ts".to_string(),
            b"export const httpsVal: number = 42;\n".to_vec(),
        );
        replies.insert(
            "https://raw.githubusercontent.com/owner/repo/v1.0.0/gh.ts".to_string(),
            b"export const ghVal: string = 'from-gh';\n".to_vec(),
        );
        replies.insert(
            "https://example.test/evil.ts".to_string(),
            b"import { secret } from 'local-secret'; export const x = secret;\n".to_vec(),
        );
        // JSR @scope/pkg@1.0.0
        replies.insert(
            "https://jsr.io/@scope/pkg/meta.json".to_string(),
            br#"{"scope":"scope","name":"pkg","latest":"1.0.0","versions":{"1.0.0":{}}}"#.to_vec(),
        );
        replies.insert(
            "https://jsr.io/@scope/pkg/1.0.0_meta.json".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "exports": { ".": "./mod.ts" },
                "manifest": {
                    "/mod.ts": {
                        "size": jsr_mod_bytes.len(),
                        "checksum": format!("sha256-{}", sha256_hex(jsr_mod_bytes))
                    }
                }
            }))
            .expect("json"),
        );
        replies.insert(
            "https://jsr.io/@scope/pkg/1.0.0/mod.ts".to_string(),
            jsr_mod_bytes.to_vec(),
        );
        // JSR @scope/evil@1.0.0
        replies.insert(
            "https://jsr.io/@scope/evil/meta.json".to_string(),
            br#"{"scope":"scope","name":"evil","latest":"1.0.0","versions":{"1.0.0":{}}}"#.to_vec(),
        );
        replies.insert(
            "https://jsr.io/@scope/evil/1.0.0_meta.json".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "exports": { ".": "./mod.ts" },
                "manifest": {
                    "/mod.ts": {
                        "size": jsr_evil_bytes.len(),
                        "checksum": format!("sha256-{}", sha256_hex(jsr_evil_bytes))
                    }
                }
            }))
            .expect("json"),
        );
        replies.insert(
            "https://jsr.io/@scope/evil/1.0.0/mod.ts".to_string(),
            jsr_evil_bytes.to_vec(),
        );

        let replies = Arc::new(replies);
        let make_transport = |map: Arc<HashMap<String, Vec<u8>>>| {
            move |url: &str,
                  _timeout: std::time::Duration,
                  _if_none_match: Option<&str>,
                  _if_modified_since: Option<&str>| {
                let body = map
                    .get(url)
                    .cloned()
                    .ok_or_else(|| format!("unexpected URL {url}"))?;
                Ok(TestRemoteResponse {
                    status: 200,
                    cache_control: Some("max-age=3600".to_string()),
                    body,
                    ..TestRemoteResponse::default()
                })
            }
        };

        let resolver = load_standalone_resolver(&manifest_path);

        // 1. Remote `https:`, `gh:`, and `jsr:` targets resolve and execute from local entry.
        let ok_entry = write(
            dir.path(),
            "ok.ts",
            r#"
import { httpsVal } from "https-dep";
import { ghVal } from "gh-dep";
import { jsrVal } from "jsr-dep";
if (httpsVal !== 42 || ghVal !== "from-gh" || jsrVal !== "from-jsr") {
    throw new Error("remote import-map target mismatch");
}
"#,
        );
        let mut rt = Runtime::with_remote_transport_and_resolver(
            RemoteOptions::new().without_cache(),
            resolver.clone(),
            make_transport(Arc::clone(&replies)),
        )
        .expect("runtime");
        rt.run_file(&ok_entry)
            .expect("remote import-map targets succeed");

        // 2. A remote `https:` module MUST NOT use local project's ImportResolver to escape to local `file:`!
        let evil_https_entry = write(
            dir.path(),
            "evil_https.ts",
            r#"import { x } from "evil-remote"; console.log(x);"#,
        );
        let mut rt2 = Runtime::with_remote_transport_and_resolver(
            RemoteOptions::new().without_cache(),
            resolver.clone(),
            make_transport(Arc::clone(&replies)),
        )
        .expect("runtime");
        let err = rt2
            .run_file(&evil_https_entry)
            .expect_err("remote referrer must not consult local ImportResolver");
        assert!(err.to_string().contains("bare module specifiers"), "{err}");

        // 3. A remote `jsr:` module MUST NOT use local project's ImportResolver to escape to local `file:`!
        let evil_jsr_entry = write(
            dir.path(),
            "evil_jsr.ts",
            r#"import { x } from "evil-jsr"; console.log(x);"#,
        );
        let mut rt3 = Runtime::with_remote_transport_and_resolver(
            RemoteOptions::new().without_cache(),
            resolver,
            make_transport(replies),
        )
        .expect("runtime");
        let err = rt3
            .run_file(&evil_jsr_entry)
            .expect_err("JSR referrer must not consult local ImportResolver");
        assert!(err.to_string().contains("bare module specifiers"), "{err}");
    }

    #[test]
    fn manifest_https_error_redacts_query_string_secret() {
        let dir = TempDir::new().expect("temp dir");
        let manifest_path = write(
            dir.path(),
            "neo.jsonc",
            r#"{
                "name": "@test/redact",
                "imports": {
                    "signed-mod": "https://example.test/mod.ts?token=supersecret"
                }
            }"#,
        );
        let entry = write(
            dir.path(),
            "main.ts",
            "import { value } from 'signed-mod';\nconsole.log(value);\n",
        );
        let resolver = load_standalone_resolver(&manifest_path);

        // Runtime error (HTTP 500 or thrown error inside loaded module) must redact `?token=supersecret`.
        let mut rt = Runtime::with_remote_transport_and_resolver(
            RemoteOptions::new().without_cache(),
            resolver,
            |url, _timeout, _if_none_match, _if_modified_since| {
                assert_eq!(url, "https://example.test/mod.ts?token=supersecret");
                Ok(TestRemoteResponse {
                    status: 500,
                    body: Vec::new(),
                    ..TestRemoteResponse::default()
                })
            },
        )
        .expect("runtime");

        let err = rt.run_file(&entry).expect_err("HTTP 500 must fail");
        let msg = err.to_string();
        assert!(msg.contains("https://example.test/mod.ts"), "{msg}");
        assert!(!msg.contains("supersecret"), "query secret leaked: {msg}");
        assert!(!msg.contains("token="), "query key leaked: {msg}");
    }
}
