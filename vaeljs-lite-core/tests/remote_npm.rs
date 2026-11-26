//! End-to-end integration tests for `https:`, `gh:`/`github:`, `jsr:`,
//! `npm:`/`package.json`/`.npmrc`, `vael.jsonc` remote/npm import maps,
//! and their security invariants in `vaeljs-lite-core`.

#[cfg(not(feature = "remote"))]
#[test]
fn remote_and_npm_are_disabled_when_features_are_off() {
    use vaeljs_lite_core::Runtime;

    for spec in [
        "https://example.test/mod.js",
        "gh:owner/repo@v1/mod.js",
        "jsr:@scope/pkg@1.0.0",
        "npm:lodash@4.17.21",
    ] {
        let mut rt = Runtime::new().expect("runtime");
        let err = rt
            .run_source("entry.js", &format!("import '{spec}';\n"))
            .expect_err("feature-disabled scheme must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("not enabled") || msg.contains("not implemented"),
            "unexpected error for {spec}: {msg}"
        );
    }
}

#[cfg(feature = "remote")]
mod remote_surface {
    use vaeljs_lite_core::{Error, RemoteOptions, Runtime};

    #[test]
    fn runtime_new_is_offline_by_default_and_rejects_run_url() {
        let mut rt = Runtime::new().expect("runtime");
        let err = rt
            .run_url("https://example.test/mod.js")
            .expect_err("default runtime must reject run_url");
        assert!(matches!(err, Error::RemoteDisabled), "{err:?}");

        // Failed preflight leaves runtime unused.
        rt.run_source("ok.js", "export const x = 1;\n")
            .expect("runtime remains unused after RemoteDisabled");
    }

    #[test]
    fn run_url_and_imports_reject_credentials_without_leaking_secrets() {
        let dir = tempfile::tempdir().expect("tempdir");
        for bad in [
            "http://example.test/mod.js",
            "https://user:secret@example.test/mod.js?token=leak",
            "gh:owner/repo/../../etc/passwd@main",
            "gh:user:secret@host/repo@v1/mod.js",
            "ftp://user:secret@host/mod.js?tok=leak",
        ] {
            let mut rt = Runtime::with_remote(RemoteOptions::new().with_cache_dir(dir.path()))
                .expect("runtime");
            let err = rt.run_url(bad).expect_err("bad URL must fail");
            let msg = err.to_string();
            assert!(!msg.contains("secret"), "{bad} -> {msg}");
            assert!(!msg.contains("leak"), "{bad} -> {msg}");
            assert!(!msg.contains("tok="), "{bad} -> {msg}");
            rt.run_source("still_usable.js", "export const ok = true;\n")
                .expect("failed URL preflight leaves runtime unused");
        }

        // Importing malformed `gh:` or credentialed non-HTTP specifiers must also
        // redact credentials in resolution error messages.
        for bad_spec in [
            "gh:user:secret@host/repo@v1/mod.js",
            "ftp://user:secret@host/mod.js?tok=leak",
        ] {
            let mut rt = Runtime::with_remote(RemoteOptions::new().with_cache_dir(dir.path()))
                .expect("runtime");
            let err = rt
                .run_source("entry.js", &format!("import '{bad_spec}';\n"))
                .expect_err("credentialed specifier import must fail");
            let msg = err.to_string();
            assert!(!msg.contains("secret"), "{bad_spec} -> {msg}");
            assert!(!msg.contains("leak"), "{bad_spec} -> {msg}");
            assert!(!msg.contains("tok="), "{bad_spec} -> {msg}");
        }
    }
}

#[cfg(all(feature = "test-support", feature = "remote"))]
mod remote_transport_tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use vaeljs_lite_core::{Error, RemoteOptions, Runtime, RuntimeOptions};
    use vaeljs_remote::TestRemoteResponse;

    fn write_file(root: &Path, relative: &str, content: &str) -> PathBuf {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, content).expect("write file");
        path
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(&mut out, "{b:02x}");
        }
        out
    }

    fn ok_response(body: impl Into<Vec<u8>>) -> TestRemoteResponse {
        let body = body.into();
        TestRemoteResponse {
            status: 200,
            declared_length: Some(body.len() as u64),
            cache_control: Some("max-age=3600".to_owned()),
            body,
            ..TestRemoteResponse::default()
        }
    }

    #[test]
    fn direct_https_gh_github_and_redirects_work_with_import_meta_url() {
        let cache = TempDir::new().expect("cache");
        let routes = Arc::new(BTreeMap::from([
            (
                "https://mods.test/entry.js".to_owned(),
                TestRemoteResponse {
                    status: 302,
                    location: Some("https://cdn.test/dir/real.js?token=secret".to_owned()),
                    ..TestRemoteResponse::default()
                },
            ),
            (
                "https://cdn.test/dir/real.js?token=secret".to_owned(),
                ok_response(
                    r#"
import { sub } from "./sub.mjs";
import { ghVal } from "gh:acme/lib@v1.0.0/mod.js";
import { githubVal } from "github:acme/lib@v1.0.0/mod.js";
const dyn = await import("./dyn.js");
if (!import.meta.url.startsWith("https://cdn.test/dir/real.js")) {
    throw new Error(`bad import.meta.url: ${import.meta.url}`);
}
if (sub !== "sub-ok" || ghVal !== "gh-ok" || githubVal !== "gh-ok" || dyn.dyn !== 42) {
    throw new Error("remote mismatch");
}
"#,
                ),
            ),
            (
                "https://cdn.test/dir/sub.mjs".to_owned(),
                ok_response("export const sub = 'sub-ok';\n"),
            ),
            (
                "https://cdn.test/dir/dyn.js".to_owned(),
                ok_response("export const dyn = 42;\n"),
            ),
            (
                "https://raw.githubusercontent.com/acme/lib/v1.0.0/mod.js".to_owned(),
                ok_response("export const ghVal = 'gh-ok'; export const githubVal = 'gh-ok';\n"),
            ),
        ]));

        let routes_ref = Arc::clone(&routes);
        let opts =
            RuntimeOptions::new().with_remote(RemoteOptions::new().with_cache_dir(cache.path()));
        let mut rt = Runtime::with_options_and_transport(opts, move |url, _timeout, _inm, _ims| {
            routes_ref
                .get(url)
                .cloned()
                .ok_or_else(|| format!("unexpected URL {url}"))
        })
        .expect("runtime");

        rt.run_url("https://mods.test/entry.js")
            .expect("redirected https entry with relative, gh, github, and dynamic imports");
    }

    #[cfg(feature = "typescript")]
    #[test]
    fn remote_and_jsr_entries_and_imports_strip_typescript_and_isolate_from_local() {
        let cache = TempDir::new().expect("cache");
        let dir = TempDir::new().expect("local dir");
        let local_secret = write_file(dir.path(), "secret.js", "export const secret = 'leak';\n");

        let mod_bytes =
            b"import { dep } from './helper.ts';\nexport const out: string = `jsr:${dep}`;\n";
        let helper_bytes = b"export const dep: string = 'verified';\n";

        let routes = Arc::new(BTreeMap::from([
            (
                "https://jsr.io/@scope/pkg/meta.json".to_owned(),
                ok_response(
                    br#"{"scope":"scope","name":"pkg","latest":"1.0.0","versions":{"1.0.0":{}}}"#
                        .to_vec(),
                ),
            ),
            (
                "https://jsr.io/@scope/pkg/1.0.0_meta.json".to_owned(),
                ok_response(
                    serde_json::to_vec(&serde_json::json!({
                        "exports": { ".": "./mod.ts" },
                        "manifest": {
                            "/mod.ts": {
                                "size": mod_bytes.len(),
                                "checksum": format!("sha256-{}", sha256_hex(mod_bytes))
                            },
                            "/helper.ts": {
                                "size": helper_bytes.len(),
                                "checksum": format!("sha256-{}", sha256_hex(helper_bytes))
                            }
                        }
                    }))
                    .expect("version meta"),
                ),
            ),
            (
                "https://jsr.io/@scope/pkg/1.0.0/mod.ts".to_owned(),
                ok_response(mod_bytes.to_vec()),
            ),
            (
                "https://jsr.io/@scope/pkg/1.0.0/helper.ts".to_owned(),
                ok_response(helper_bytes.to_vec()),
            ),
            (
                "https://evil.test/escape.js".to_owned(),
                ok_response(format!("import 'file://{}';\n", local_secret.display()).into_bytes()),
            ),
            (
                "https://evil.test/http.js".to_owned(),
                ok_response(b"import 'http://insecure.test/x.js';\n".to_vec()),
            ),
            (
                "https://jsr.io/@scope/bad/meta.json".to_owned(),
                ok_response(
                    br#"{"scope":"scope","name":"bad","latest":"1.0.0","versions":{"1.0.0":{}}}"#
                        .to_vec(),
                ),
            ),
            (
                "https://jsr.io/@scope/bad/1.0.0_meta.json".to_owned(),
                ok_response(
                    serde_json::to_vec(&serde_json::json!({
                        "exports": { ".": "./mod.js" },
                        "manifest": {
                            "/mod.js": {
                                "size": 10,
                                "checksum": format!("sha256-{}", sha256_hex(b"0123456789"))
                            }
                        }
                    }))
                    .expect("bad meta"),
                ),
            ),
            (
                "https://jsr.io/@scope/bad/1.0.0/mod.js".to_owned(),
                ok_response(b"tampered!!".to_vec()),
            ),
        ]));

        // Direct `jsr:` entry URL via `Runtime::run_url`.
        let routes_ref = Arc::clone(&routes);
        let opts =
            RuntimeOptions::new().with_remote(RemoteOptions::new().with_cache_dir(cache.path()));
        let mut rt = Runtime::with_options_and_transport(opts.clone(), move |url, _, _, _| {
            routes_ref
                .get(url)
                .cloned()
                .ok_or_else(|| format!("unexpected URL {url}"))
        })
        .expect("runtime");
        rt.run_url("jsr:@scope/pkg@1.0.0")
            .expect("jsr: entrypoint with relative TS succeeds");

        // Remote module cannot import `file:`.
        let routes_ref = Arc::clone(&routes);
        let mut rt_escape =
            Runtime::with_options_and_transport(opts.clone(), move |url, _, _, _| {
                routes_ref
                    .get(url)
                    .cloned()
                    .ok_or_else(|| format!("unexpected URL {url}"))
            })
            .expect("runtime");
        let escape_err = rt_escape
            .run_url("https://evil.test/escape.js")
            .expect_err("remote cannot import file:");
        assert!(
            escape_err
                .to_string()
                .contains("remote code may not read local files"),
            "{escape_err}"
        );

        // Remote module cannot import `http:`.
        let routes_ref = Arc::clone(&routes);
        let mut rt_http = Runtime::with_options_and_transport(opts.clone(), move |url, _, _, _| {
            routes_ref
                .get(url)
                .cloned()
                .ok_or_else(|| format!("unexpected URL {url}"))
        })
        .expect("runtime");
        let http_err = rt_http
            .run_url("https://evil.test/http.js")
            .expect_err("remote cannot import http:");
        assert!(
            http_err.to_string().contains("must use https:, not http:"),
            "{http_err}"
        );

        // JSR checksum mismatch is rejected.
        let routes_ref = Arc::clone(&routes);
        let mut rt_tamper = Runtime::with_options_and_transport(opts, move |url, _, _, _| {
            routes_ref
                .get(url)
                .cloned()
                .ok_or_else(|| format!("unexpected URL {url}"))
        })
        .expect("runtime");
        let tamper_err = rt_tamper
            .run_url("jsr:@scope/bad@1.0.0")
            .expect_err("tampered jsr checksum must fail");
        assert!(tamper_err.to_string().contains("checksum"), "{tamper_err}");
    }

    #[test]
    fn jsr_collision_and_redirect_security_invariants_hold() {
        let cache = TempDir::new().expect("cache");
        let mod_bytes = b"export const v = 1;\n";
        let routes = Arc::new(BTreeMap::from([
            (
                "https://jsr.io/@scope/pkg/meta.json".to_owned(),
                ok_response(
                    br#"{"scope":"scope","name":"pkg","latest":"1.0.0","versions":{"1.0.0":{}}}"#
                        .to_vec(),
                ),
            ),
            (
                "https://jsr.io/@scope/pkg/1.0.0_meta.json".to_owned(),
                ok_response(
                    serde_json::to_vec(&serde_json::json!({
                        "exports": { ".": "./mod.js" },
                        "manifest": {
                            "/mod.js": {
                                "size": mod_bytes.len(),
                                "checksum": format!("sha256-{}", sha256_hex(mod_bytes))
                            }
                        }
                    }))
                    .expect("version meta"),
                ),
            ),
            (
                "https://jsr.io/@scope/pkg/1.0.0/mod.js".to_owned(),
                ok_response(mod_bytes.to_vec()),
            ),
            (
                "https://evil.test/redirect-to-jsr".to_owned(),
                TestRemoteResponse {
                    status: 302,
                    location: Some("https://jsr.io/@scope/pkg/1.0.0/mod.js".to_owned()),
                    ..TestRemoteResponse::default()
                },
            ),
        ]));

        // 1. Verified `jsr:` followed by direct `https://jsr.io/...` reuses verified module;
        //    an ordinary redirect onto that verified JSR identity is refused.
        let routes_ref = Arc::clone(&routes);
        let opts =
            RuntimeOptions::new().with_remote(RemoteOptions::new().with_cache_dir(cache.path()));
        let mut rt = Runtime::with_options_and_transport(opts.clone(), move |url, _, _, _| {
            routes_ref
                .get(url)
                .cloned()
                .ok_or_else(|| format!("unexpected URL {url}"))
        })
        .expect("runtime");
        rt.run_source(
            "main.js",
            r#"
const a = await import("jsr:@scope/pkg@1.0.0");
const b = await import("https://jsr.io/@scope/pkg/1.0.0/mod.js");
if (a !== b || a.v !== 1) throw new Error("module identity mismatch");
let redirErr = null;
try {
    await import("https://evil.test/redirect-to-jsr");
} catch (e) {
    redirErr = String(e);
}
if (!redirErr || !redirErr.includes("already a verified JSR module")) {
    throw new Error(`expected JSR collision error, got: ${redirErr}`);
}
"#,
        )
        .expect("JSR identity reuse and redirect guard");

        // 2. Ordinary `https://jsr.io/...` loaded first cannot later be reused by `jsr:`.
        let routes_ref = Arc::clone(&routes);
        let mut rt2 = Runtime::with_options_and_transport(opts, move |url, _, _, _| {
            routes_ref
                .get(url)
                .cloned()
                .ok_or_else(|| format!("unexpected URL {url}"))
        })
        .expect("runtime");
        let err = rt2
            .run_source(
                "main2.js",
                r#"
await import("https://jsr.io/@scope/pkg/1.0.0/mod.js");
await import("jsr:@scope/pkg@1.0.0");
"#,
            )
            .expect_err("jsr: after ordinary load must fail");
        assert!(
            err.to_string()
                .contains("already loaded as an ordinary HTTPS module"),
            "{err}"
        );
    }

    #[test]
    fn remote_error_redaction_strips_query_secrets_while_preserving_user_errors() {
        let cache = TempDir::new().expect("cache");
        let routes = Arc::new(BTreeMap::from([
            (
                "https://mods.test/throws.js?token=secret".to_owned(),
                ok_response("throw new Error('boom');\n"),
            ),
            (
                "https://mods.test/user_msg.js?token=secret".to_owned(),
                ok_response(
                    "throw new Error('keep https://mods.test/user_msg.js?token=secret');\n",
                ),
            ),
            (
                "https://mods.test/link_main.js".to_owned(),
                ok_response("import { missing } from './mod.js?token=secret';\n"),
            ),
            (
                "https://mods.test/mod.js?token=secret".to_owned(),
                ok_response("export const present = 1;\n"),
            ),
        ]));

        // Stack trace + module identity redact query secrets.
        let r1 = Arc::clone(&routes);
        let opts =
            RuntimeOptions::new().with_remote(RemoteOptions::new().with_cache_dir(cache.path()));
        let mut rt1 = Runtime::with_options_and_transport(opts.clone(), move |url, _, _, _| {
            r1.get(url).cloned().ok_or_else(|| format!("404 {url}"))
        })
        .expect("runtime");
        let err1 = rt1
            .run_url("https://mods.test/throws.js?token=secret")
            .expect_err("throws");
        let msg1 = err1.to_string();
        assert!(msg1.contains("boom"), "{msg1}");
        assert!(!msg1.contains("secret"), "{msg1}");
        assert!(!msg1.contains("token="), "{msg1}");

        // User-thrown Error message is preserved while stack location is redacted.
        let r2 = Arc::clone(&routes);
        let mut rt2 = Runtime::with_options_and_transport(opts.clone(), move |url, _, _, _| {
            r2.get(url).cloned().ok_or_else(|| format!("404 {url}"))
        })
        .expect("runtime");
        let err2 = rt2
            .run_url("https://mods.test/user_msg.js?token=secret")
            .expect_err("throws");
        let msg2 = err2.to_string();
        assert!(
            msg2.contains("keep https://mods.test/user_msg.js?token=secret"),
            "{msg2}"
        );

        // Link error (`Could not find export`) redacts the query secret.
        let r3 = Arc::clone(&routes);
        let mut rt3 = Runtime::with_options_and_transport(opts, move |url, _, _, _| {
            r3.get(url).cloned().ok_or_else(|| format!("404 {url}"))
        })
        .expect("runtime");
        let err3 = rt3
            .run_url("https://mods.test/link_main.js")
            .expect_err("link error");
        let msg3 = err3.to_string();
        assert!(!msg3.contains("secret"), "{msg3}");
        assert!(!msg3.contains("token="), "{msg3}");

        // TypeScript diagnostic on entry leaves runtime unused and redacts query.
        #[cfg(feature = "typescript")]
        {
            let routes_ts = Arc::new(BTreeMap::from([(
                "https://mods.test/bad.ts?token=secret".to_owned(),
                ok_response("enum E { A }\n"),
            )]));
            let mut rt4 = Runtime::with_options_and_transport(
                RuntimeOptions::new()
                    .with_remote(RemoteOptions::new().with_cache_dir(cache.path())),
                move |url, _, _, _| {
                    routes_ts
                        .get(url)
                        .cloned()
                        .ok_or_else(|| format!("404 {url}"))
                },
            )
            .expect("runtime");
            let err4 = rt4
                .run_url("https://mods.test/bad.ts?token=secret")
                .expect_err("non-erasable TS");
            assert!(matches!(err4, Error::RemoteTypeScript { .. }), "{err4:?}");
            assert!(!err4.to_string().contains("secret"), "{err4}");
        }
    }

    #[test]
    fn failed_jsr_load_cleans_up_pending_state_and_allows_unrelated_https_imports() {
        let cache = TempDir::new().expect("cache");
        let routes = Arc::new(BTreeMap::from([
            (
                "https://jsr.io/@scope/bad/meta.json".to_owned(),
                ok_response(r#"{"latest":"1.0.0","versions":{"1.0.0":{}}}"#),
            ),
            (
                "https://jsr.io/@scope/bad/1.0.0_meta.json".to_owned(),
                // Valid export so `resolve_jsr` marks `1.0.0/index.js` pending,
                // then `load_jsr` fails due to checksum mismatch on `index.js`.
                ok_response(
                    r#"{"manifest":{"/index.js":{"size":24,"checksum":"sha256-0000000000000000000000000000000000000000000000000000000000000000"}},"exports":{".":"./index.js"}}"#,
                ),
            ),
            (
                "https://jsr.io/@scope/bad/1.0.0/index.js".to_owned(),
                ok_response("export const bad = 1;\n"),
            ),
            (
                "https://mods.test/ok.js".to_owned(),
                ok_response("export const value = 42;\n"),
            ),
        ]));
        let dir = TempDir::new().expect("project");
        let manifest_check = dir.path().join("check.js");
        fs::write(
            &manifest_check,
            r#"
let sawErr = false;
try {
    await import("jsr:@scope/bad@1.0.0");
} catch (_) {
    sawErr = true;
}
// Direct HTTPS import of the same URL should now go through `load_https`
// (not stale `jsr_pending` checksum verification) and succeed cleanly.
const direct = await import("https://jsr.io/@scope/bad/1.0.0/index.js");
const ok = await import("https://mods.test/ok.js");
if (!sawErr || direct.bad !== 1 || ok.value !== 42) throw new Error("unexpected state");
"#,
        )
        .expect("write");
        let mut rt = Runtime::with_options_and_transport(
            RuntimeOptions::new().with_remote(RemoteOptions::new().with_cache_dir(cache.path())),
            move |url, _, _, _| {
                routes
                    .get(url)
                    .cloned()
                    .ok_or_else(|| format!("unexpected fetch: {url}"))
            },
        )
        .expect("runtime");
        rt.run_file(&manifest_check)
            .expect("failed JSR load must clean up jsr_pending and not poison runtime");
    }

    #[cfg(all(feature = "npm", feature = "typescript"))]
    mod npm_and_manifest_tests {
        use super::*;
        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use sha2::Sha512;
        use std::io::Write as _;
        use std::sync::Mutex;
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
            origin: &str,
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
            let tarball_url = format!("{origin}/{name}/-/{version}.tgz");
            let deps_val = serde_json::from_str::<serde_json::Value>(pkg_json)
                .ok()
                .and_then(|v| v.get("dependencies").cloned())
                .unwrap_or_else(|| serde_json::json!({}));
            let packument = serde_json::json!({
                "name": name,
                "dist-tags": { "latest": version },
                "versions": {
                    version: {
                        "version": version,
                        "dependencies": deps_val,
                        "dist": {
                            "tarball": tarball_url,
                            "integrity": sri
                        }
                    }
                }
            });
            routes.insert(
                format!("{origin}/{encoded_name}"),
                serde_json::to_vec(&packument).expect("packument json"),
            );
            routes.insert(tarball_url, tgz);
        }

        #[test]
        fn package_json_imports_transitive_npm_npmrc_auth_and_isolation_work() {
            let dir = TempDir::new().expect("project dir");
            let cache = TempDir::new().expect("cache dir");
            let marker = dir.path().join("LIFECYCLE_RAN");

            write_file(
                dir.path(),
                "package.json",
                r##"{
                    "name": "@demo/app",
                    "type": "module",
                    "exports": { ".": "./src/self_entry.js" },
                    "imports": {
                        "#internal": "./src/internal.ts",
                        "#ext": "public-pkg"
                    },
                    "dependencies": {
                        "public-pkg": "^1.0.0",
                        "@priv/lib": "^1.0.0"
                    }
                }"##,
            );
            write_file(
                dir.path(),
                ".npmrc",
                "@priv:registry=https://priv.example.test/npm/\n//priv.example.test/npm/:_authToken=super-secret-token\n",
            );
            write_file(
                dir.path(),
                "src/internal.ts",
                "export const internal: string = 'hash-ok';\n",
            );
            write_file(
                dir.path(),
                "src/self_entry.js",
                "export const selfVal = 'self-ok';\n",
            );
            let main = write_file(
                dir.path(),
                "src/main.ts",
                r##"
import { internal } from "#internal";
import { pub } from "#ext";
import { selfVal } from "@demo/app";
import { privVal } from "@priv/lib";
import { pub as depVal } from "public-pkg";
const dyn = await import("npm:public-pkg@^1.0.0/sub");
if (
    internal !== "hash-ok" ||
    pub !== "pub-ok" ||
    selfVal !== "self-ok" ||
    privVal !== "priv-ok" ||
    depVal !== "pub-ok" ||
    dyn.sub !== "sub-ok"
) {
    throw new Error("package.json/npm mismatch");
}
"##,
            );

            let mut routes = BTreeMap::new();
            publish_npm(
                &mut routes,
                "https://registry.npmjs.org",
                "public-pkg",
                "1.0.0",
                r#"{
                    "name": "public-pkg",
                    "version": "1.0.0",
                    "type": "module",
                    "scripts": { "postinstall": "touch LIFECYCLE_RAN" },
                    "dependencies": { "leaf-dep": "^1.0.0" },
                    "exports": { ".": "./index.js", "./sub": "./sub.ts" }
                }"#,
                &[
                    (
                        "index.js",
                        "import { leaf } from 'leaf-dep'; export const pub = leaf;\n",
                    ),
                    ("sub.ts", "export const sub: string = 'sub-ok';\n"),
                ],
            );
            publish_npm(
                &mut routes,
                "https://registry.npmjs.org",
                "leaf-dep",
                "1.0.0",
                r#"{ "name": "leaf-dep", "version": "1.0.0", "type": "module", "exports": "./index.js" }"#,
                &[("index.js", "export const leaf = 'pub-ok';\n")],
            );
            publish_npm(
                &mut routes,
                "https://priv.example.test/npm",
                "@priv/lib",
                "1.0.0",
                r#"{ "name": "@priv/lib", "version": "1.0.0", "type": "module", "exports": "./index.js" }"#,
                &[("index.js", "export const privVal = 'priv-ok';\n")],
            );

            let routes = Arc::new(routes);
            let seen_auth = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
            let routes_ref = Arc::clone(&routes);
            let auth_ref = Arc::clone(&seen_auth);

            let opts = RuntimeOptions::new()
                .with_remote(RemoteOptions::new().with_cache_dir(cache.path()));
            let mut rt = Runtime::with_options_and_auth_transport(
                opts,
                move |url, _timeout, _inm, _ims, auth| {
                    auth_ref
                        .lock()
                        .expect("lock")
                        .push((url.to_owned(), auth.map(str::to_owned)));
                    let body = routes_ref
                        .get(url)
                        .cloned()
                        .ok_or_else(|| format!("unexpected URL {url}"))?;
                    Ok(ok_response(body))
                },
            )
            .expect("runtime");

            rt.run_file(&main)
                .expect("package.json + .npmrc + npm works");
            assert!(!marker.exists(), "lifecycle scripts must never execute");
            assert!(!dir.path().join("node_modules").exists());
            assert!(dir.path().join("vael.lock").exists());

            // Verify Bearer auth was sent ONLY to priv.example.test and NEVER to registry.npmjs.org.
            let recorded = seen_auth.lock().expect("lock");
            assert!(recorded.iter().any(|(u, a)| {
                u.starts_with("https://priv.example.test/")
                    && a.as_deref() == Some("Bearer super-secret-token")
            }));
            assert!(
                recorded
                    .iter()
                    .all(|(u, a)| { !u.starts_with("https://registry.npmjs.org/") || a.is_none() })
            );

            // Offline replay with --frozen-lockfile and no transport succeeds.
            let offline_opts = RuntimeOptions::new()
                .with_offline(true)
                .with_frozen_lockfile(true)
                .with_npm_cache_dir(cache.path());
            Runtime::with_options(offline_opts)
                .expect("offline runtime")
                .run_file(&main)
                .expect("offline frozen-lockfile replay succeeds");

            // CJS package must be rejected cleanly, and npm:// cannot escape to local file:.
            let bad_dir = TempDir::new().expect("bad dir");
            let mut bad_routes = BTreeMap::new();
            publish_npm(
                &mut bad_routes,
                "https://registry.npmjs.org",
                "cjs-pkg",
                "1.0.0",
                r#"{ "name": "cjs-pkg", "version": "1.0.0", "main": "./index.cjs" }"#,
                &[("index.cjs", "module.exports = 1;\n")],
            );
            publish_npm(
                &mut bad_routes,
                "https://registry.npmjs.org",
                "escape-pkg",
                "1.0.0",
                r#"{ "name": "escape-pkg", "version": "1.0.0", "type": "module", "exports": "./index.js" }"#,
                &[(
                    "index.js",
                    &format!("import 'file://{}';\n", main.display()),
                )],
            );
            let bad_routes = Arc::new(bad_routes);

            let cjs_entry = write_file(bad_dir.path(), "cjs.js", "import 'npm:cjs-pkg@^1.0.0';\n");
            let br1 = Arc::clone(&bad_routes);
            let cjs_err = Runtime::with_options_and_transport(
                RuntimeOptions::new()
                    .with_remote(RemoteOptions::new().with_cache_dir(cache.path())),
                move |url, _, _, _| {
                    br1.get(url)
                        .cloned()
                        .map(ok_response)
                        .ok_or_else(|| format!("404 {url}"))
                },
            )
            .expect("rt")
            .run_file(&cjs_entry)
            .expect_err("CJS npm package must fail");
            assert!(
                cjs_err.to_string().contains(".cjs") || cjs_err.to_string().contains("CommonJS"),
                "{cjs_err}"
            );

            let esc_entry = write_file(
                bad_dir.path(),
                "esc.js",
                "import 'npm:escape-pkg@^1.0.0';\n",
            );
            let br2 = Arc::clone(&bad_routes);
            let esc_err = Runtime::with_options_and_transport(
                RuntimeOptions::new()
                    .with_remote(RemoteOptions::new().with_cache_dir(cache.path())),
                move |url, _, _, _| {
                    br2.get(url)
                        .cloned()
                        .map(ok_response)
                        .ok_or_else(|| format!("404 {url}"))
                },
            )
            .expect("rt")
            .run_file(&esc_entry)
            .expect_err("npm:// cannot import file:");
            assert!(
                esc_err.to_string().contains("npm") || esc_err.to_string().contains("outside"),
                "{esc_err}"
            );
        }

        #[cfg(feature = "manifest")]
        #[test]
        fn ancestor_vael_jsonc_import_map_resolves_npm_jsr_gh_and_https_and_replays_offline() {
            use vaeljs_lite_core::ImportResolver;
            use vaeljs_manifest::discover_manifest;

            let dir = TempDir::new().expect("temp dir");
            let cache = TempDir::new().expect("cache dir");

            write_file(
                dir.path(),
                "vael.jsonc",
                r#"{
                    "name": "@demo/ancestor-lite",
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
            for forbidden in ["npm:", "jsr:", "gh:", "github:", "https:"] {
                assert!(!main_source.contains(forbidden));
            }
            let main = write_file(dir.path(), "apps/nested/sub/main.ts", main_source);

            let mut routes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
            publish_npm(
                &mut routes,
                "https://registry.npmjs.org",
                "demo-pkg",
                "1.0.0",
                r#"{
                    "name": "demo-pkg",
                    "version": "1.0.0",
                    "type": "module",
                    "exports": { ".": "./index.ts", "./sub": "./sub.ts" }
                }"#,
                &[
                    ("index.ts", "export const npmVal: string = 'npm-root';\n"),
                    ("sub.ts", "export const npmSubVal: string = 'npm-sub';\n"),
                ],
            );
            publish_npm(
                &mut routes,
                "https://registry.npmjs.org",
                "@acme/lib",
                "1.0.0",
                r#"{ "name": "@acme/lib", "version": "1.0.0", "type": "module", "exports": "./index.ts" }"#,
                &[(
                    "index.ts",
                    "export const scopedVal: string = 'npm-scoped';\n",
                )],
            );

            let jsr_mod_bytes = b"export const jsrVal: string = 'jsr-root';\n";
            let jsr_sub_bytes = b"export const jsrSubVal: string = 'jsr-sub';\n";
            routes.insert(
                "https://jsr.io/@demo/pkg/meta.json".to_owned(),
                br#"{"scope":"demo","name":"pkg","latest":"1.0.0","versions":{"1.0.0":{}}}"#
                    .to_vec(),
            );
            routes.insert(
                "https://jsr.io/@demo/pkg/1.0.0_meta.json".to_owned(),
                serde_json::to_vec(&serde_json::json!({
                    "exports": { ".": "./mod.ts", "./sub": "./sub.ts" },
                    "manifest": {
                        "/mod.ts": { "size": jsr_mod_bytes.len(), "checksum": format!("sha256-{}", sha256_hex(jsr_mod_bytes)) },
                        "/sub.ts": { "size": jsr_sub_bytes.len(), "checksum": format!("sha256-{}", sha256_hex(jsr_sub_bytes)) }
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
            let discovered = discover_manifest(main.parent().expect("nested dir"), None)
                .expect("discover_manifest")
                .expect("ancestor vael.jsonc");
            let resolver = ImportResolver::from_manifest(&discovered);

            let routes_ref = Arc::clone(&routes);
            let opts = RuntimeOptions::new()
                .with_remote(RemoteOptions::new().with_cache_dir(cache.path()))
                .with_import_resolver(resolver);
            let mut rt = Runtime::with_options_and_transport(opts, move |url, _, _, _| {
                let body = routes_ref
                    .get(url)
                    .cloned()
                    .ok_or_else(|| format!("unexpected URL {url}"))?;
                Ok(ok_response(body))
            })
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

            let lock = Lockfile::read_from_path(&root_lock_path).expect("read vael.lock");
            assert_eq!(
                lock.specifiers.get("demo-pkg@^1.0.0").map(String::as_str),
                Some("demo-pkg@1.0.0")
            );
            assert_eq!(
                lock.specifiers.get("@acme/lib@^1.0.0").map(String::as_str),
                Some("@acme/lib@1.0.0")
            );

            // Pass 2: offline + frozen-lockfile with NO remote transport.
            let npm_offline = write_file(
                dir.path(),
                "apps/nested/sub/npm_offline.ts",
                r#"
import { npmSubVal } from "my-npm/sub";
import { scopedVal } from "my-scoped";
const dynNpm = await import("my-npm");
if (npmSubVal !== "npm-sub" || scopedVal !== "npm-scoped" || dynNpm.npmVal !== "npm-root") {
    throw new Error("offline npm import-map mismatch");
}
"#,
            );
            let rediscovered = discover_manifest(npm_offline.parent().expect("nested dir"), None)
                .expect("discover")
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
                .expect("offline frozen-lockfile replay succeeds");

            // Pass 3: cold dynamic import("my-jsr") from cache with failing network transport.
            let jsr_dyn = write_file(
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
            Runtime::with_options_and_transport(jsr_opts, |_url, _, _, _| {
                Err("unexpected network call".to_owned())
            })
            .expect("cached jsr runtime")
            .run_file(&jsr_dyn)
            .expect("cold dynamic import('my-jsr') succeeds from cache");
        }
    }
}
