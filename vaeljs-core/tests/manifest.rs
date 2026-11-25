//! End-to-end tests for `vael.jsonc` `ImportResolver` integration in `vaeljs-core`.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use vaeljs_core::{Error, ImportResolver, Runtime, RuntimeOptions};
use vaeljs_manifest::{DiscoveredManifest, Manifest, PackageName, Workspace, discover_manifest};

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
        "vael.jsonc",
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
        .expect_err("Runtime::new must not discover vael.jsonc implicitly");
    let msg = err.to_string();
    assert!(msg.contains("bare module specifiers"), "{msg}");
}

#[test]
fn resolves_exact_and_prefix_local_imports_and_type_strips_ts() {
    let dir = TempDir::new().expect("temp dir");
    let manifest_path = write(
        dir.path(),
        "vael.jsonc",
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
        "vael.jsonc",
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
        "packages/a/vael.jsonc",
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
        "packages/b/vael.jsonc",
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
        "vael.jsonc",
        r#"{
            "name": "@test/policy",
            "imports": {
                "vael:os": "./fake-os.js",
                "VAEL:os": "./fake-os.js",
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

    // `vael:os` and uppercase `VAEL:os` must use real built-in, not `./fake-os.js`.
    let os_main = write(
        dir.path(),
        "os_check.ts",
        r#"
import { platform } from "vael:os";
import { platform as upperPlatform } from "VAEL:os";
if (platform === "fake" || upperPlatform === "fake" || platform !== upperPlatform) {
    throw new Error("vael:os or VAEL:os was shadowed by imports");
}
"#,
    );
    Runtime::with_import_resolver(resolver.clone())
        .expect("runtime")
        .run_file(&os_main)
        .expect("vael:os and VAEL:os succeed");

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
            vaeljs_core::RuntimeOptions::new()
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

    // `.cjs` target must fail vaeljs-module-policy check.
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
        "vael.jsonc",
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

#[test]
fn plaintext_http_in_ancestor_manifest_is_rejected_as_unsupported_transport() {
    let dir = TempDir::new().expect("temp dir");
    write(
        dir.path(),
        "vael.jsonc",
        r#"{
            "name": "@test/insecure-http",
            "imports": {
                "insecure": "http://cdn.example.test/mod.ts"
            }
        }"#,
    );
    let nested = write(
        dir.path(),
        "apps/nested/sub/main.ts",
        "import { x } from 'insecure';\nconsole.log(x);\n",
    );

    let err = discover_manifest(nested.parent().unwrap(), None)
        .expect_err("plaintext http:// target in ancestor vael.jsonc must fail closed");
    assert!(
        matches!(err, vaeljs_manifest::Error::UnsupportedTransport { .. }),
        "expected UnsupportedTransport, got {err:?}"
    );
    assert!(
        err.to_string().contains("https:"),
        "expected HTTPS-only reason, got: {err}"
    );
}

#[cfg(feature = "test-support")]
mod remote_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use sha2::{Digest, Sha256};
    use vaeljs_core::RemoteOptions;
    use vaeljs_core::test_support::TestRemoteResponse;

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
            "vael.jsonc",
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
            "vael.jsonc",
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

    #[cfg(feature = "npm")]
    #[test]
    fn ancestor_import_map_resolves_all_external_schemes_and_persists_lockfile_at_manifest_root() {
        use std::collections::BTreeMap;
        use std::io::Write as _;
        use std::sync::Mutex;

        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use sha2::Sha512;
        use vaeljs_npm::Lockfile;

        fn write_tar_header(name: &str, size: usize) -> [u8; 512] {
            let mut header = [0u8; 512];
            let name_bytes = name.as_bytes();
            let copy_len = name_bytes.len().min(100);
            header[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            header[100..108].copy_from_slice(b"0000644\0");
            header[108..116].copy_from_slice(b"0000000\0");
            header[116..124].copy_from_slice(b"0000000\0");
            let size_str = format!("{size:011o}\0");
            header[124..136].copy_from_slice(size_str.as_bytes());
            header[136..148].copy_from_slice(b"00000000000\0");
            header[148..156].copy_from_slice(b"        ");
            header[156] = b'0';
            header[257..263].copy_from_slice(b"ustar\0");
            header[263..265].copy_from_slice(b"00");
            let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
            let csum = format!("{sum:06o}\0 ");
            header[148..156].copy_from_slice(csum.as_bytes());
            header
        }

        fn build_tgz(files: &[(&str, &str)]) -> (Vec<u8>, String) {
            let mut tar_buf = Vec::new();
            for (rel_path, content) in files {
                let full = format!("package/{rel_path}");
                let bytes = content.as_bytes();
                tar_buf.extend_from_slice(&write_tar_header(&full, bytes.len()));
                if !bytes.is_empty() {
                    tar_buf.extend_from_slice(bytes);
                    let pad = (512 - (bytes.len() % 512)) % 512;
                    tar_buf.resize(tar_buf.len() + pad, 0);
                }
            }
            tar_buf.resize(tar_buf.len() + 1024, 0);
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            gz.write_all(&tar_buf).expect("write tar");
            let tgz = gz.finish().expect("finish gzip");
            let sri = format!("sha512-{}", BASE64_STANDARD.encode(Sha512::digest(&tgz)));
            (tgz, sri)
        }

        fn publish_npm(
            routes: &mut BTreeMap<String, Vec<u8>>,
            name: &str,
            version: &str,
            pkg_json: &str,
            files: &[(&str, &str)],
        ) {
            let mut all_files = vec![("package.json", pkg_json)];
            all_files.extend_from_slice(files);
            let (tgz, sri) = build_tgz(&all_files);
            let encoded_name = if let Some(rest) = name.strip_prefix('@') {
                let (scope, pkg) = rest.split_once('/').expect("scoped name");
                format!("@{scope}%2f{pkg}")
            } else {
                name.to_owned()
            };
            let tarball_url = format!("https://registry.npmjs.org/{name}/-/{version}.tgz");
            let packument = serde_json::json!({
                "name": name,
                "dist-tags": { "latest": version },
                "versions": {
                    version: {
                        "version": version,
                        "dependencies": {},
                        "dist": {
                            "tarball": tarball_url,
                            "integrity": sri
                        }
                    }
                }
            });
            routes.insert(
                format!("https://registry.npmjs.org/{encoded_name}"),
                serde_json::to_vec(&packument).expect("packument json"),
            );
            routes.insert(tarball_url, tgz);
        }

        let dir = TempDir::new().expect("temp dir");
        let cache = TempDir::new().expect("cache dir");

        write(
            dir.path(),
            "vael.jsonc",
            r#"{
                "name": "@demo/ancestor-imports",
                "imports": {
                    "my-npm": "npm:demo-pkg@^1.0.0",
                    "my-npm/": "npm:demo-pkg@^1.0.0/",
                    "my-scoped": "npm:@acme/lib@^1.0.0",
                    "my-jsr": "jsr:@demo/pkg@^1.0.0",
                    "my-jsr/": "jsr:@demo/pkg@^1.0.0/",
                    "my-gh": "gh:acme/repo@v1.0.0/mod.ts",
                    "my-github": "github:acme/repo@v1.0.0/alt.ts",
                    "gh-root/": "gh:acme/repo@v1.0.0/",
                    "gh-src/": "gh:acme/repo@v1.0.0/src/",
                    "my-https": "https://cdn.example.test/mod.ts",
                    "my-https/": "https://cdn.example.test/lib/"
                }
            }"#,
        );

        let main_source = r#"
import { npmVal } from "my-npm";
import { npmSubVal } from "my-npm/sub";
import { scopedVal } from "my-scoped";
import { jsrVal } from "my-jsr";
import { jsrSubVal } from "my-jsr/sub";
import { ghVal } from "my-gh";
import { githubVal } from "my-github";
import { ghUtil } from "gh-root/util.ts";
import { ghHelper } from "gh-src/helper.ts";
import { httpsVal } from "my-https";
import { httpsSubVal } from "my-https/sub.ts";

const dynNpm = await import("my-npm");
const dynJsr = await import("my-jsr");

if (npmVal !== "npm-root" || npmSubVal !== "npm-sub" || scopedVal !== "npm-scoped") {
    throw new Error("npm import-map mismatch");
}
if (dynNpm.npmVal !== "npm-root") {
    throw new Error("dynamic npm import mismatch");
}
if (jsrVal !== "jsr-root" || jsrSubVal !== "jsr-sub") {
    throw new Error("jsr import-map mismatch");
}
if (dynJsr.jsrVal !== "jsr-root") {
    throw new Error("dynamic jsr import mismatch");
}
if (
    ghVal !== "gh-mod" ||
    githubVal !== "github-alt" ||
    ghUtil !== "gh-root-util" ||
    ghHelper !== "gh-src-helper"
) {
    throw new Error("gh/github import-map mismatch");
}
if (httpsVal !== "https-root" || httpsSubVal !== "https-sub") {
    throw new Error("https import-map mismatch");
}
"#;
        // Guard against any scheme prefix leaking into the JS/TS source itself.
        for forbidden in ["npm:", "jsr:", "gh:", "github:", "https:"] {
            assert!(
                !main_source.contains(forbidden),
                "script must use bare specifiers only, found {forbidden}"
            );
        }

        let main = write(dir.path(), "apps/nested/sub/main.ts", main_source);

        let mut routes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        publish_npm(
            &mut routes,
            "demo-pkg",
            "1.0.0",
            r#"{
                "name": "demo-pkg",
                "version": "1.0.0",
                "type": "module",
                "exports": {
                    ".": "./index.ts",
                    "./sub": "./sub.ts"
                }
            }"#,
            &[
                ("index.ts", "export const npmVal: string = 'npm-root';\n"),
                ("sub.ts", "export const npmSubVal: string = 'npm-sub';\n"),
            ],
        );
        publish_npm(
            &mut routes,
            "@acme/lib",
            "1.0.0",
            r#"{
                "name": "@acme/lib",
                "version": "1.0.0",
                "type": "module",
                "exports": "./index.ts"
            }"#,
            &[(
                "index.ts",
                "export const scopedVal: string = 'npm-scoped';\n",
            )],
        );

        let jsr_mod_bytes = b"export const jsrVal: string = 'jsr-root';\n";
        let jsr_sub_bytes = b"export const jsrSubVal: string = 'jsr-sub';\n";
        routes.insert(
            "https://jsr.io/@demo/pkg/meta.json".to_owned(),
            br#"{"scope":"demo","name":"pkg","latest":"1.0.0","versions":{"1.0.0":{}}}"#.to_vec(),
        );
        routes.insert(
            "https://jsr.io/@demo/pkg/1.0.0_meta.json".to_owned(),
            serde_json::to_vec(&serde_json::json!({
                "exports": {
                    ".": "./mod.ts",
                    "./sub": "./sub.ts"
                },
                "manifest": {
                    "/mod.ts": {
                        "size": jsr_mod_bytes.len(),
                        "checksum": format!("sha256-{}", sha256_hex(jsr_mod_bytes))
                    },
                    "/sub.ts": {
                        "size": jsr_sub_bytes.len(),
                        "checksum": format!("sha256-{}", sha256_hex(jsr_sub_bytes))
                    }
                }
            }))
            .expect("jsr version meta"),
        );
        routes.insert(
            "https://jsr.io/@demo/pkg/1.0.0/mod.ts".to_owned(),
            jsr_mod_bytes.to_vec(),
        );
        routes.insert(
            "https://jsr.io/@demo/pkg/1.0.0/sub.ts".to_owned(),
            jsr_sub_bytes.to_vec(),
        );

        routes.insert(
            "https://raw.githubusercontent.com/acme/repo/v1.0.0/mod.ts".to_owned(),
            b"export const ghVal: string = 'gh-mod';\n".to_vec(),
        );
        routes.insert(
            "https://raw.githubusercontent.com/acme/repo/v1.0.0/alt.ts".to_owned(),
            b"export const githubVal: string = 'github-alt';\n".to_vec(),
        );
        routes.insert(
            "https://raw.githubusercontent.com/acme/repo/v1.0.0/util.ts".to_owned(),
            b"export const ghUtil: string = 'gh-root-util';\n".to_vec(),
        );
        routes.insert(
            "https://raw.githubusercontent.com/acme/repo/v1.0.0/src/helper.ts".to_owned(),
            b"export const ghHelper: string = 'gh-src-helper';\n".to_vec(),
        );

        routes.insert(
            "https://cdn.example.test/mod.ts".to_owned(),
            b"export const httpsVal: string = 'https-root';\n".to_vec(),
        );
        routes.insert(
            "https://cdn.example.test/lib/sub.ts".to_owned(),
            b"export const httpsSubVal: string = 'https-sub';\n".to_vec(),
        );

        let routes = Arc::new(routes);
        let requested_urls = Arc::new(Mutex::new(Vec::<String>::new()));

        let discovered = discover_manifest(main.parent().expect("nested dir"), None)
            .expect("discover_manifest")
            .expect("ancestor vael.jsonc must be discovered");
        let canonical_root_manifest =
            fs::canonicalize(dir.path().join("vael.jsonc")).expect("canonical manifest");
        assert_eq!(discovered.path, canonical_root_manifest);

        let resolver = ImportResolver::from_manifest(&discovered);

        // Pass 1: online mock transport fetches npm, jsr, gh/github, and https into CAS + cache
        // and writes `vael.lock` next to the ancestor `vael.jsonc`.
        let routes_ref = Arc::clone(&routes);
        let urls_ref = Arc::clone(&requested_urls);
        let opts = RuntimeOptions::new()
            .with_remote(RemoteOptions::new().with_cache_dir(cache.path()))
            .with_import_resolver(resolver.clone());
        let mut rt = Runtime::with_options_and_transport(
            opts,
            move |url, _timeout, _if_none_match, _if_modified_since| {
                urls_ref.lock().expect("lock").push(url.to_owned());
                let body = routes_ref
                    .get(url)
                    .cloned()
                    .ok_or_else(|| format!("unexpected URL {url}"))?;
                Ok(TestRemoteResponse {
                    status: 200,
                    declared_length: Some(body.len() as u64),
                    cache_control: Some("max-age=3600".to_owned()),
                    body,
                    ..TestRemoteResponse::default()
                })
            },
        )
        .expect("online runtime");
        rt.run_file(&main)
            .expect("nested script resolves all four bare import-map types");

        let root_lock_path = dir.path().join("vael.lock");
        assert!(
            root_lock_path.exists(),
            "vael.lock must be persisted next to ancestor vael.jsonc"
        );
        assert!(
            !dir.path().join("apps/nested/sub/vael.lock").exists(),
            "vael.lock must not be written inside nested script dir"
        );
        assert!(!dir.path().join("node_modules").exists());
        assert!(!dir.path().join("apps/nested/sub/node_modules").exists());

        let lock = Lockfile::read_from_path(&root_lock_path).expect("read vael.lock");
        assert_eq!(
            lock.specifiers.get("demo-pkg@^1.0.0").map(String::as_str),
            Some("demo-pkg@1.0.0")
        );
        assert_eq!(
            lock.specifiers.get("@acme/lib@^1.0.0").map(String::as_str),
            Some("@acme/lib@1.0.0")
        );
        assert!(lock.packages.contains_key("demo-pkg@1.0.0"));
        assert!(lock.packages.contains_key("@acme/lib@1.0.0"));

        // Pass 2: subsequent offline runtime with NO remote transport succeeds using
        // the ancestor `vael.lock` and populated CAS (with cold dynamic `import("my-npm")`).
        let npm_offline_source = r#"
import { npmSubVal } from "my-npm/sub";
import { scopedVal } from "my-scoped";
const dynNpm = await import("my-npm");
if (
    npmSubVal !== "npm-sub" ||
    scopedVal !== "npm-scoped" ||
    dynNpm.npmVal !== "npm-root"
) {
    throw new Error("offline npm import-map mismatch");
}
"#;
        for forbidden in ["npm:", "jsr:", "gh:", "github:", "https:"] {
            assert!(!npm_offline_source.contains(forbidden));
        }
        let npm_offline = write(
            dir.path(),
            "apps/nested/sub/npm_offline.ts",
            npm_offline_source,
        );
        let rediscovered = discover_manifest(npm_offline.parent().expect("nested dir"), None)
            .expect("discover_manifest")
            .expect("ancestor vael.jsonc");
        let offline_resolver = ImportResolver::from_manifest(&rediscovered);

        let offline_opts = RuntimeOptions::new()
            .with_offline(true)
            .with_frozen_lockfile(true)
            .with_npm_cache_dir(cache.path())
            .with_import_resolver(offline_resolver.clone());
        Runtime::with_options(offline_opts)
            .expect("offline runtime")
            .run_file(&npm_offline)
            .expect("offline runtime must succeed from ancestor vael.lock + CAS");

        // Pass 3: cold dynamic `import("my-jsr")` without a prior static JSR import.
        let jsr_dyn_only = write(
            dir.path(),
            "apps/nested/sub/jsr_dyn.ts",
            r#"
const dynJsr = await import("my-jsr");
if (dynJsr.jsrVal !== "jsr-root") {
    throw new Error("cold dynamic jsr import mismatch");
}
"#,
        );
        let jsr_opts = RuntimeOptions::new()
            .with_remote(RemoteOptions::new().with_cache_dir(cache.path()))
            .with_import_resolver(offline_resolver);
        Runtime::with_options_and_transport(
            jsr_opts,
            |_url, _timeout, _if_none_match, _if_modified_since| {
                Err("unexpected network call; should be cached or fail if missed".to_owned())
            },
        )
        .expect("cached jsr runtime")
        .run_file(&jsr_dyn_only)
        .expect("cold dynamic import(\"my-jsr\") succeeds from cache");
    }
}
