//! End-to-end V8 integration tests for `npm:` and `package.json` resolution,
//! `neo.lock` v1 enforcement, CAS verification, zero `node_modules`, zero
//! install-script execution, `--no-npm` runtime opt-out, and `--no-default-features`
//! compile-time opt-out.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(not(feature = "npm"))]
use neojs_core::Runtime;

fn write_file(root: &Path, relative: &str, content: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, content).expect("write file");
    path
}

#[cfg(feature = "npm")]
mod cas_and_lockfile_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use neojs_core::{NpmOptions, Runtime, RuntimeOptions};
    use neojs_npm::{
        CasStore, DEFAULT_REGISTRY_ORIGIN, LockedPackage, Lockfile, NpmLimits, TarballIngestor,
    };
    use sha2::{Digest, Sha512};

    fn write_tar_header(name: &str, typeflag: u8, size: usize) -> [u8; 512] {
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
        header[156] = typeflag;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let csum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(csum.as_bytes());
        header
    }

    pub(super) fn build_package_tgz(files: &[(&str, &str)]) -> (Vec<u8>, String) {
        let mut tar_buf = Vec::new();
        for (rel_path, content) in files {
            let full_path = format!("package/{rel_path}");
            let bytes = content.as_bytes();
            let hdr = write_tar_header(&full_path, b'0', bytes.len());
            tar_buf.extend_from_slice(&hdr);
            if !bytes.is_empty() {
                tar_buf.extend_from_slice(bytes);
                let pad = (512 - (bytes.len() % 512)) % 512;
                tar_buf.resize(tar_buf.len() + pad, 0);
            }
        }
        tar_buf.resize(tar_buf.len() + 1024, 0);
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_buf).unwrap();
        let tgz = gz.finish().unwrap();
        let sri = format!("sha512-{}", BASE64_STANDARD.encode(Sha512::digest(&tgz)));
        (tgz, sri)
    }

    fn seed_cas_package(
        cas: &CasStore,
        pkg_json: &str,
        files: &[(&str, &str)],
    ) -> (String, String) {
        let mut all = vec![("package.json", pkg_json)];
        all.extend_from_slice(files);
        let (tgz, sri) = build_package_tgz(&all);
        let tree = TarballIngestor::new(cas.clone(), NpmLimits::default())
            .ingest(DEFAULT_REGISTRY_ORIGIN, &sri, &tgz)
            .expect("ingest package into CAS");
        (sri, tree.tree_sha256)
    }

    #[test]
    fn npm_specifier_static_and_dynamic_work_from_cas_and_lockfile_in_v8() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let cas = CasStore::new(cache.path());

        let (sri, tree_sha256) = seed_cas_package(
            &cas,
            r#"{
                "name": "@acme/greeter",
                "version": "1.0.0",
                "type": "module",
                "exports": {
                    ".": { "import": "./index.mjs" },
                    "./format": { "import": "./format.ts" }
                }
            }"#,
            &[
                (
                    "index.mjs",
                    "import { fmt } from './format.ts'; export const greet = (n) => `hi ${fmt(n)}`;\n",
                ),
                (
                    "format.ts",
                    "export const fmt = (s: string): string => s.toUpperCase();\n",
                ),
            ],
        );

        let mut lock = Lockfile::new();
        lock.specifiers.insert(
            "@acme/greeter@^1.0.0".to_owned(),
            "@acme/greeter@1.0.0".to_owned(),
        );
        lock.packages.insert(
            "@acme/greeter@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: sri,
                tree_sha256: Some(tree_sha256),
                tarball_url: "https://registry.npmjs.org/@acme/greeter/-/1.0.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.write_atomic(&project.path().join("neo.lock")).unwrap();

        let main = write_file(
            project.path(),
            "main.ts",
            r#"
import { greet } from "npm:@acme/greeter@^1.0.0";
if (greet("neo") !== "hi NEO") throw new Error("static npm: failed");
const dyn = await import("npm:@acme/greeter@^1.0.0/format");
if (dyn.fmt("ok") !== "OK") throw new Error("dynamic npm: failed");
"#,
        );

        let opts = RuntimeOptions::new().with_npm_cache_dir(cache.path());
        Runtime::with_options(opts)
            .unwrap()
            .run_file(&main)
            .expect("npm: static and dynamic succeed offline from CAS");
        assert!(!project.path().join("node_modules").exists());
    }

    #[test]
    fn package_json_bare_subpath_dev_deps_and_hash_imports_zero_node_modules() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let cas = CasStore::new(cache.path());

        let (greeter_sri, greeter_tree) = seed_cas_package(
            &cas,
            r#"{
                "name": "@acme/greeter",
                "version": "1.2.0",
                "type": "module",
                "exports": {
                    ".": "./index.mjs",
                    "./format": "./format.mjs"
                }
            }"#,
            &[
                ("index.mjs", "export const greet = (s) => `hello ${s}`;\n"),
                ("format.mjs", "export const fmt = (s) => `[${s}]`;\n"),
            ],
        );
        let (dev_sri, dev_tree) = seed_cas_package(
            &cas,
            r#"{
                "name": "dev-helper",
                "version": "2.0.0",
                "module": "./esm.mjs"
            }"#,
            &[("esm.mjs", "export const tag = 'dev-ok';\n")],
        );

        let mut lock = Lockfile::new();
        lock.specifiers.insert(
            "@acme/greeter@^1.0.0".to_owned(),
            "@acme/greeter@1.2.0".to_owned(),
        );
        lock.specifiers.insert(
            "dev-helper@^2.0.0".to_owned(),
            "dev-helper@2.0.0".to_owned(),
        );
        lock.packages.insert(
            "@acme/greeter@1.2.0".to_owned(),
            LockedPackage {
                version: "1.2.0".to_owned(),
                integrity: greeter_sri,
                tree_sha256: Some(greeter_tree),
                tarball_url: "https://registry.npmjs.org/@acme/greeter/-/1.2.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.packages.insert(
            "dev-helper@2.0.0".to_owned(),
            LockedPackage {
                version: "2.0.0".to_owned(),
                integrity: dev_sri,
                tree_sha256: Some(dev_tree),
                tarball_url: "https://registry.npmjs.org/dev-helper/-/2.0.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.write_atomic(&project.path().join("neo.lock")).unwrap();

        write_file(
            project.path(),
            "package.json",
            r##"{
                "name": "demo-app",
                "version": "0.1.0",
                "type": "module",
                "imports": {
                    "#local": "./src/local.ts",
                    "#fmt": "@acme/greeter/format"
                },
                "dependencies": {
                    "@acme/greeter": "^1.0.0",
                    "neo:os": "^9.9.9"
                },
                "devDependencies": {
                    "dev-helper": "^2.0.0"
                }
            }"##,
        );
        write_file(
            project.path(),
            "src/local.ts",
            "export const localVal: number = 42;\n",
        );
        let main = write_file(
            project.path(),
            "src/main.ts",
            r##"
import { platform } from "neo:os";
import { greet } from "@acme/greeter";
import { fmt } from "@acme/greeter/format";
import { tag } from "dev-helper";
import { localVal } from "#local";
import { fmt as hashFmt } from "#fmt";

if (typeof platform !== "string" || platform.length === 0) throw new Error("neo:os hijacked");
if (greet("world") !== "hello world") throw new Error("bare dep failed");
if (fmt("a") !== "[a]" || hashFmt("b") !== "[b]") throw new Error("subpath/#import failed");
if (tag !== "dev-ok") throw new Error("devDependencies failed");
if (localVal !== 42) throw new Error("local #import failed");
"##,
        );

        let opts = RuntimeOptions::new()
            .with_frozen_lockfile(true)
            .with_offline(true)
            .with_npm_cache_dir(cache.path());
        Runtime::with_options(opts)
            .unwrap()
            .run_file(&main)
            .expect("package.json resolution succeeds without node_modules");
        assert!(!project.path().join("node_modules").exists());
    }

    #[test]
    fn transitive_dependencies_resolve_cleanly_inside_cas() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let cas = CasStore::new(cache.path());

        let (b_sri, b_tree) = seed_cas_package(
            &cas,
            r#"{"name":"pkg-b","version":"1.4.0","type":"module","exports":"./b.mjs"}"#,
            &[("b.mjs", "export const b = (x) => x * 2;\n")],
        );
        let (a_sri, a_tree) = seed_cas_package(
            &cas,
            r#"{
                "name": "pkg-a",
                "version": "1.0.0",
                "type": "module",
                "exports": "./a.mjs",
                "dependencies": { "pkg-b": "^1.0.0" }
            }"#,
            &[
                (
                    "a.mjs",
                    "import { b } from 'pkg-b'; import { offset } from './helper.mjs'; export const calc = (x) => b(x) + offset;\n",
                ),
                ("helper.mjs", "export const offset = 5;\n"),
            ],
        );

        let mut lock = Lockfile::new();
        lock.specifiers
            .insert("pkg-a@^1.0.0".to_owned(), "pkg-a@1.0.0".to_owned());
        lock.packages.insert(
            "pkg-a@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: a_sri,
                tree_sha256: Some(a_tree),
                tarball_url: "https://registry.npmjs.org/pkg-a/-/1.0.0.tgz".to_owned(),
                dependencies: BTreeMap::from([("pkg-b".to_owned(), "pkg-b@1.4.0".to_owned())]),
            },
        );
        lock.packages.insert(
            "pkg-b@1.4.0".to_owned(),
            LockedPackage {
                version: "1.4.0".to_owned(),
                integrity: b_sri,
                tree_sha256: Some(b_tree),
                tarball_url: "https://registry.npmjs.org/pkg-b/-/1.4.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.write_atomic(&project.path().join("neo.lock")).unwrap();

        let main = write_file(
            project.path(),
            "main.js",
            r#"
import { calc } from "npm:pkg-a@^1.0.0";
if (calc(10) !== 25) throw new Error("transitive npm calc failed");
"#,
        );

        let opts = RuntimeOptions::new()
            .with_frozen_lockfile(true)
            .with_offline(true)
            .with_npm_cache_dir(cache.path());
        Runtime::with_options(opts)
            .unwrap()
            .run_file(&main)
            .expect("transitive CAS graph succeeds");
        assert!(!project.path().join("node_modules").exists());
    }

    #[test]
    fn frozen_lockfile_fails_closed_on_missing_entry_or_tampered_integrity() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let cas = CasStore::new(cache.path());

        let (sri, tree) = seed_cas_package(
            &cas,
            r#"{"name":"@acme/greeter","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
            &[("index.mjs", "export const greet = () => 'locked-ok';\n")],
        );

        let mut lock = Lockfile::new();
        lock.specifiers.insert(
            "@acme/greeter@^1.0.0".to_owned(),
            "@acme/greeter@1.0.0".to_owned(),
        );
        lock.packages.insert(
            "@acme/greeter@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: sri,
                tree_sha256: Some(tree),
                tarball_url: "https://registry.npmjs.org/@acme/greeter/-/1.0.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        let lock_path = project.path().join("neo.lock");
        lock.write_atomic(&lock_path).unwrap();

        write_file(
            project.path(),
            "package.json",
            r#"{
                "name": "lock-app",
                "type": "module",
                "dependencies": {
                    "@acme/greeter": "^1.0.0",
                    "extra-pkg": "^1.0.0"
                }
            }"#,
        );
        let main = write_file(
            project.path(),
            "main.js",
            r#"import { greet } from "@acme/greeter"; if (greet() !== "locked-ok") throw new Error("bad");"#,
        );

        // Matching entry succeeds.
        Runtime::with_options(
            RuntimeOptions::new()
                .with_frozen_lockfile(true)
                .with_offline(true)
                .with_npm_cache_dir(cache.path()),
        )
        .unwrap()
        .run_file(&main)
        .expect("matching frozen lockfile succeeds");

        // Missing entry fails closed.
        let missing = write_file(project.path(), "missing.js", r#"import "extra-pkg";"#);
        let err = Runtime::with_options(
            RuntimeOptions::new()
                .with_frozen_lockfile(true)
                .with_offline(true)
                .with_npm_cache_dir(cache.path()),
        )
        .unwrap()
        .run_file(&missing)
        .expect_err("missing lockfile entry must fail");
        assert!(err.to_string().contains("neo.lock"), "{err}");

        // Tampered integrity in neo.lock fails closed.
        let mut tampered = lock;
        tampered
            .packages
            .get_mut("@acme/greeter@1.0.0")
            .unwrap()
            .integrity = format!("sha512-{}", BASE64_STANDARD.encode([0x11; 64]));
        tampered.write_atomic(&lock_path).unwrap();

        let err_tamper = Runtime::with_options(
            RuntimeOptions::new()
                .with_frozen_lockfile(true)
                .with_offline(true)
                .with_npm_cache_dir(cache.path()),
        )
        .unwrap()
        .run_file(&main)
        .expect_err("tampered integrity must fail closed");
        assert!(
            err_tamper.to_string().contains("offline")
                || err_tamper.to_string().contains("missing")
                || err_tamper.to_string().contains("mismatch"),
            "{err_tamper}"
        );
    }

    #[test]
    fn postinstall_script_is_never_executed_and_cjs_is_rejected_clearly() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let cas = CasStore::new(cache.path());
        let marker = project.path().join("pwned.txt");

        let evil_pkg = serde_json::json!({
            "name": "evil-scripts",
            "version": "1.0.0",
            "type": "module",
            "exports": "./index.mjs",
            "scripts": {
                "preinstall": format!("touch {}", marker.display()),
                "install": format!("touch {}", marker.display()),
                "postinstall": format!("touch {}", marker.display()),
                "prepare": format!("touch {}", marker.display()),
            }
        });
        let (evil_sri, evil_tree) = seed_cas_package(
            &cas,
            &evil_pkg.to_string(),
            &[("index.mjs", "export const safe = 'ok';\n")],
        );
        let (cjs_sri, cjs_tree) = seed_cas_package(
            &cas,
            r#"{"name":"cjs-only","version":"1.0.0","main":"./index.js"}"#,
            &[("index.js", "module.exports = require('fs');\n")],
        );

        let mut lock = Lockfile::new();
        lock.specifiers.insert(
            "evil-scripts@^1.0.0".to_owned(),
            "evil-scripts@1.0.0".to_owned(),
        );
        lock.specifiers
            .insert("cjs-only@^1.0.0".to_owned(), "cjs-only@1.0.0".to_owned());
        lock.packages.insert(
            "evil-scripts@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: evil_sri,
                tree_sha256: Some(evil_tree),
                tarball_url: "https://registry.npmjs.org/evil-scripts/-/1.0.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.packages.insert(
            "cjs-only@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: cjs_sri,
                tree_sha256: Some(cjs_tree),
                tarball_url: "https://registry.npmjs.org/cjs-only/-/1.0.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.write_atomic(&project.path().join("neo.lock")).unwrap();

        let main = write_file(
            project.path(),
            "main.js",
            r#"import { safe } from "npm:evil-scripts@^1.0.0"; if (safe !== "ok") throw new Error("bad");"#,
        );
        Runtime::with_options(RuntimeOptions::new().with_npm_cache_dir(cache.path()))
            .unwrap()
            .run_file(&main)
            .expect("package with ignored lifecycle scripts runs");
        assert!(
            !marker.exists(),
            "lifecycle scripts must NEVER be executed!"
        );

        let cjs_main = write_file(
            project.path(),
            "cjs.js",
            r#"import x from "npm:cjs-only@^1.0.0"; console.log(x);"#,
        );
        let err = Runtime::with_options(RuntimeOptions::new().with_npm_cache_dir(cache.path()))
            .unwrap()
            .run_file(&cjs_main)
            .expect_err("CJS-only package must fail cleanly");
        assert!(err.to_string().contains("CommonJS"), "{err}");
    }

    #[test]
    fn runtime_without_npm_option_rejects_npm_and_package_json() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let cas = CasStore::new(cache.path());
        let (sri, tree) = seed_cas_package(
            &cas,
            r#"{"name":"greeter","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
            &[("index.mjs", "export const ok = 1;\n")],
        );

        let mut lock = Lockfile::new();
        lock.specifiers
            .insert("greeter@^1.0.0".to_owned(), "greeter@1.0.0".to_owned());
        lock.packages.insert(
            "greeter@1.0.0".to_owned(),
            LockedPackage {
                version: "1.0.0".to_owned(),
                integrity: sri,
                tree_sha256: Some(tree),
                tarball_url: "https://registry.npmjs.org/greeter/-/1.0.0.tgz".to_owned(),
                dependencies: BTreeMap::new(),
            },
        );
        lock.write_atomic(&project.path().join("neo.lock")).unwrap();
        write_file(
            project.path(),
            "package.json",
            r#"{"name":"app","type":"module","dependencies":{"greeter":"^1.0.0"}}"#,
        );

        let main_npm = write_file(
            project.path(),
            "main_npm.js",
            r#"import "npm:greeter@^1.0.0";"#,
        );
        let err = Runtime::with_options(
            RuntimeOptions::new().with_npm(
                NpmOptions::new()
                    .with_enabled(false)
                    .with_cache_dir(cache.path()),
            ),
        )
        .unwrap()
        .run_file(&main_npm)
        .expect_err("--no-npm / without_npm must reject npm:");
        assert!(err.to_string().contains("disabled"), "{err}");

        let main_bare = write_file(project.path(), "main_bare.js", r#"import "greeter";"#);
        let err_bare = Runtime::with_options(RuntimeOptions::new().without_npm())
            .unwrap()
            .run_file(&main_bare)
            .expect_err("without_npm must not consult package.json");
        assert!(
            err_bare.to_string().contains("bare module specifiers"),
            "{err_bare}"
        );
    }
}

#[cfg(all(feature = "npm", feature = "test-support"))]
mod mock_registry_tests {
    use super::cas_and_lockfile_tests::build_package_tgz;
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use neojs_core::test_support::TestRemoteResponse;
    use neojs_core::{RemoteOptions, Runtime, RuntimeOptions};
    use neojs_npm::Lockfile;

    #[derive(Clone, Default)]
    struct MockNpmServer {
        routes: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl MockNpmServer {
        fn new() -> Self {
            Self::default()
        }

        fn publish(&self, name: &str, version: &str, pkg_json: &str, files: &[(&str, &str)]) {
            let mut all_files = vec![("package.json", pkg_json)];
            all_files.extend_from_slice(files);
            let (tgz, sri) = build_package_tgz(&all_files);

            let encoded_name = if let Some(rest) = name.strip_prefix('@') {
                let (scope, pkg) = rest.split_once('/').unwrap();
                format!("@{scope}%2f{pkg}")
            } else {
                name.to_owned()
            };
            let tarball_url = format!("https://registry.npmjs.org/{name}/-/{version}.tgz");
            let parsed_pkg: serde_json::Value = serde_json::from_str(pkg_json).unwrap();
            let deps = parsed_pkg
                .get("dependencies")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            let packument = serde_json::json!({
                "name": name,
                "dist-tags": { "latest": version },
                "versions": {
                    version: {
                        "version": version,
                        "dependencies": deps,
                        "dist": {
                            "tarball": tarball_url,
                            "integrity": sri
                        }
                    }
                }
            });

            let mut routes = self.routes.lock().unwrap();
            routes.insert(
                format!("https://registry.npmjs.org/{encoded_name}"),
                serde_json::to_vec(&packument).unwrap(),
            );
            routes.insert(tarball_url, tgz);
        }

        fn transport(
            &self,
        ) -> impl Fn(
            &str,
            std::time::Duration,
            Option<&str>,
            Option<&str>,
        ) -> Result<TestRemoteResponse, String>
        + Send
        + Sync
        + 'static {
            let routes = Arc::clone(&self.routes);
            let requests = Arc::clone(&self.requests);
            move |url, _timeout, _if_none_match, _if_modified_since| {
                requests.lock().unwrap().push(url.to_owned());
                if let Some(body) = routes.lock().unwrap().get(url).cloned() {
                    Ok(TestRemoteResponse {
                        status: 200,
                        location: None,
                        declared_length: Some(body.len() as u64),
                        etag: None,
                        last_modified: None,
                        cache_control: Some("max-age=60".to_owned()),
                        date: None,
                        age: None,
                        body,
                    })
                } else {
                    Ok(TestRemoteResponse {
                        status: 404,
                        location: None,
                        declared_length: None,
                        etag: None,
                        last_modified: None,
                        cache_control: None,
                        date: None,
                        age: None,
                        body: b"not found".to_vec(),
                    })
                }
            }
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    #[test]
    fn remote_transport_resolves_npm_writes_lockfile_then_frozen_offline_reuses_cas() {
        let project = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let server = MockNpmServer::new();

        server.publish(
            "@acme/greeter",
            "1.0.0",
            r#"{
                "name": "@acme/greeter",
                "version": "1.0.0",
                "type": "module",
                "exports": { ".": "./index.mjs", "./format": "./format.mjs" }
            }"#,
            &[
                (
                    "index.mjs",
                    "import { fmt } from './format.mjs'; export const greet = (s) => fmt(s);\n",
                ),
                ("format.mjs", "export const fmt = (s) => `hello ${s}`;\n"),
            ],
        );

        write_file(
            project.path(),
            "package.json",
            r#"{"name":"remote-app","type":"module","dependencies":{"@acme/greeter":"^1.0.0"}}"#,
        );
        let main = write_file(
            project.path(),
            "main.js",
            r#"
import { greet } from "@acme/greeter";
import { fmt } from "@acme/greeter/format";
if (greet("neo") !== "hello neo" || fmt("x") !== "hello x") throw new Error("bad");
"#,
        );

        let opts =
            RuntimeOptions::new().with_remote(RemoteOptions::new().with_cache_dir(cache.path()));
        Runtime::with_options_and_transport(opts, server.transport())
            .unwrap()
            .run_file(&main)
            .expect("mock registry fetch + CAS ingest succeeds");

        let lock_path = project.path().join("neo.lock");
        assert!(lock_path.exists());
        let lock = Lockfile::read_from_path(&lock_path).unwrap();
        assert_eq!(lock.version, 1);
        assert_eq!(
            lock.specifiers
                .get("@acme/greeter@^1.0.0")
                .map(String::as_str),
            Some("@acme/greeter@1.0.0")
        );

        let reqs_after_first = server.request_count();
        assert!(reqs_after_first >= 2);

        // Second run: frozen + offline, zero network requests.
        Runtime::with_options(
            RuntimeOptions::new()
                .with_frozen_lockfile(true)
                .with_offline(true)
                .with_npm_cache_dir(cache.path()),
        )
        .unwrap()
        .run_file(&main)
        .expect("frozen offline run succeeds");
        assert_eq!(server.request_count(), reqs_after_first);
    }
}

#[cfg(not(feature = "npm"))]
#[test]
fn compile_time_disabled_npm_feature_rejects_npm_and_package_json() {
    let project = TempDir::new().unwrap();
    write_file(
        project.path(),
        "package.json",
        r#"{"name":"app","type":"module","dependencies":{"greeter":"^1.0.0"}}"#,
    );
    let main_npm = write_file(
        project.path(),
        "main_npm.js",
        r#"import "npm:greeter@^1.0.0";"#,
    );
    let err = Runtime::new()
        .unwrap()
        .run_file(&main_npm)
        .expect_err("compile-time disabled npm feature must reject npm:");
    assert!(
        err.to_string().contains("npm: modules are not implemented"),
        "{err}"
    );

    let main_bare = write_file(project.path(), "main_bare.js", r#"import "greeter";"#);
    let err_bare = Runtime::new()
        .unwrap()
        .run_file(&main_bare)
        .expect_err("compile-time disabled npm feature must not consult package.json");
    assert!(
        err_bare.to_string().contains("bare module specifiers"),
        "{err_bare}"
    );
}
