//! Zero-network integration tests for `neojs-npm` covering real `.tgz`
//! ingestion, CAS tamper detection, peer-context isolation, cycles,
//! `exports`/`#imports`/fallback ESM policy, security rejections, and project
//! `package.json` bare-import resolution.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use neojs_module_policy::{ModuleFormat, PackageType};
use neojs_npm::{
    CasError, CasStore, ExportsError, NpmGraphResolver, NpmLimits, NpmReferrerResolution,
    NpmRegistry, NpmSpecifier, PackageName, PackageTree, Packument, ProjectPackageResolver,
    ProjectResolution, ResolverError, TarballError, TarballIngestor, TargetPlatform,
};
use sha2::{Digest, Sha512};

// ---------------------------------------------------------------------------
// Test tar + gzip builder helpers (self-contained, zero external tar crate)
// ---------------------------------------------------------------------------

struct TarEntrySpec {
    path: String,
    typeflag: u8,
    bytes: Vec<u8>,
    use_pax_path: bool,
}

impl TarEntrySpec {
    fn file(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            typeflag: b'0',
            bytes: bytes.into(),
            use_pax_path: false,
        }
    }

    fn pax_file(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            typeflag: b'0',
            bytes: bytes.into(),
            use_pax_path: true,
        }
    }

    fn dir(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            typeflag: b'5',
            bytes: Vec::new(),
            use_pax_path: false,
        }
    }

    fn symlink(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            typeflag: b'2',
            bytes: Vec::new(),
            use_pax_path: false,
        }
    }

    fn hardlink(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            typeflag: b'1',
            bytes: Vec::new(),
            use_pax_path: false,
        }
    }
}

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

fn encode_pax_record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut len = body.len() + 1;
    loop {
        let candidate = format!("{len}{body}");
        if candidate.len() == len {
            return candidate.into_bytes();
        }
        len = candidate.len();
    }
}

fn build_tgz(entries: &[TarEntrySpec]) -> (Vec<u8>, String) {
    let mut tar_buf = Vec::new();
    for entry in entries {
        if entry.use_pax_path {
            let pax_payload = encode_pax_record("path", &entry.path);
            let pax_hdr = write_tar_header("PaxHeader/entry", b'x', pax_payload.len());
            tar_buf.extend_from_slice(&pax_hdr);
            tar_buf.extend_from_slice(&pax_payload);
            let pad = (512 - (pax_payload.len() % 512)) % 512;
            tar_buf.resize(tar_buf.len() + pad, 0);

            let hdr = write_tar_header("truncated-name", entry.typeflag, entry.bytes.len());
            tar_buf.extend_from_slice(&hdr);
        } else {
            let hdr = write_tar_header(&entry.path, entry.typeflag, entry.bytes.len());
            tar_buf.extend_from_slice(&hdr);
        }

        if !entry.bytes.is_empty() {
            tar_buf.extend_from_slice(&entry.bytes);
            let pad = (512 - (entry.bytes.len() % 512)) % 512;
            tar_buf.resize(tar_buf.len() + pad, 0);
        }
    }
    // Two 512-byte EOF zero blocks.
    tar_buf.resize(tar_buf.len() + 1024, 0);

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_buf).unwrap();
    let tgz = gz.finish().unwrap();
    let sri = format!("sha512-{}", BASE64_STANDARD.encode(Sha512::digest(&tgz)));
    (tgz, sri)
}

// ---------------------------------------------------------------------------
// Mock NpmRegistry
// ---------------------------------------------------------------------------

struct MockPackageVersion {
    version: String,
    tgz: Vec<u8>,
    integrity: Option<String>,
    dependencies: BTreeMap<String, String>,
    optional_dependencies: BTreeMap<String, String>,
    peer_dependencies: BTreeMap<String, String>,
    peer_dependencies_meta: BTreeMap<String, bool>,
    os: Option<Vec<String>>,
    cpu: Option<Vec<String>>,
    libc: Option<Vec<String>>,
}

struct MockRegistry {
    origin: String,
    packages: BTreeMap<String, (BTreeMap<String, String>, Vec<MockPackageVersion>)>,
    tarballs: BTreeMap<String, Vec<u8>>,
}

impl MockRegistry {
    fn new() -> Self {
        Self {
            origin: "https://registry.npmjs.org".to_owned(),
            packages: BTreeMap::new(),
            tarballs: BTreeMap::new(),
        }
    }

    fn publish(
        &mut self,
        name: &str,
        version: &str,
        dist_tags: &[(&str, &str)],
        pkg: MockPackageVersion,
    ) {
        let tarball_url = format!("{}/{}/-/{}-{}.tgz", self.origin, name, name, version);
        self.tarballs.insert(tarball_url, pkg.tgz.clone());

        let entry = self
            .packages
            .entry(name.to_owned())
            .or_insert_with(|| (BTreeMap::new(), Vec::new()));
        for (tag, ver) in dist_tags {
            entry.0.insert((*tag).to_owned(), (*ver).to_owned());
        }
        entry.1.push(pkg);
    }

    fn publish_simple(
        &mut self,
        name: &str,
        version: &str,
        tag_latest: bool,
        pkg_json: &str,
        extra_files: &[(&str, &str)],
    ) {
        let mut entries = vec![
            TarEntrySpec::dir("package/"),
            TarEntrySpec::file("package/package.json", pkg_json.as_bytes().to_vec()),
        ];
        for (rel, body) in extra_files {
            entries.push(TarEntrySpec::file(
                format!("package/{rel}"),
                body.as_bytes().to_vec(),
            ));
        }
        let (tgz, sri) = build_tgz(&entries);
        let parsed: serde_json::Value = serde_json::from_str(pkg_json).unwrap();
        let to_map = |field: &str| -> BTreeMap<String, String> {
            parsed
                .get(field)
                .and_then(|v| v.as_object())
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let peer_meta = parsed
            .get("peerDependenciesMeta")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.get("optional").and_then(|o| o.as_bool()).unwrap_or(false),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let to_vec = |field: &str| -> Option<Vec<String>> {
            parsed
                .get(field)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
        };

        let tags: Vec<(&str, &str)> = if tag_latest {
            vec![("latest", version)]
        } else {
            vec![]
        };
        self.publish(
            name,
            version,
            &tags,
            MockPackageVersion {
                version: version.to_owned(),
                tgz,
                integrity: Some(sri),
                dependencies: to_map("dependencies"),
                optional_dependencies: to_map("optionalDependencies"),
                peer_dependencies: to_map("peerDependencies"),
                peer_dependencies_meta: peer_meta,
                os: to_vec("os"),
                cpu: to_vec("cpu"),
                libc: to_vec("libc"),
            },
        );
    }
}

impl NpmRegistry for MockRegistry {
    fn registry_origin(&self) -> &str {
        &self.origin
    }

    fn fetch_packument(&self, package_name: &PackageName) -> Result<Packument, ResolverError> {
        let (tags, versions) =
            self.packages
                .get(package_name.as_str())
                .ok_or_else(|| ResolverError::Registry {
                    package: package_name.to_string(),
                    reason: "404 package not found".to_owned(),
                })?;

        let mut versions_json = serde_json::Map::new();
        for v in versions {
            let tarball_url = format!(
                "{}/{}/-/{}-{}.tgz",
                self.origin, package_name, package_name, v.version
            );
            let peer_meta_json: serde_json::Map<String, serde_json::Value> = v
                .peer_dependencies_meta
                .iter()
                .map(|(k, opt)| (k.clone(), serde_json::json!({ "optional": opt })))
                .collect();
            let mut dist = serde_json::Map::new();
            dist.insert("tarball".to_owned(), serde_json::json!(tarball_url));
            if let Some(sri) = &v.integrity {
                dist.insert("integrity".to_owned(), serde_json::json!(sri));
            }
            let mut ver_obj = serde_json::json!({
                "version": v.version,
                "dist": dist,
                "dependencies": v.dependencies,
                "optionalDependencies": v.optional_dependencies,
                "peerDependencies": v.peer_dependencies,
                "peerDependenciesMeta": peer_meta_json,
            });
            if let Some(os) = &v.os {
                ver_obj["os"] = serde_json::json!(os);
            }
            if let Some(cpu) = &v.cpu {
                ver_obj["cpu"] = serde_json::json!(cpu);
            }
            if let Some(libc) = &v.libc {
                ver_obj["libc"] = serde_json::json!(libc);
            }
            versions_json.insert(v.version.clone(), ver_obj);
        }

        let pack_bytes = serde_json::to_vec(&serde_json::json!({
            "name": package_name.as_str(),
            "dist-tags": tags,
            "versions": versions_json,
        }))
        .unwrap();
        Packument::parse_bytes(&pack_bytes, 16 * 1024 * 1024)
    }

    fn fetch_tarball(&self, tarball_url: &str, _integrity: &str) -> Result<Vec<u8>, ResolverError> {
        self.tarballs
            .get(tarball_url)
            .cloned()
            .ok_or_else(|| ResolverError::Registry {
                package: tarball_url.to_owned(),
                reason: "404 tarball not found".to_owned(),
            })
    }
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[test]
fn resolves_scoped_unscoped_ranges_dist_tags_cycles_and_multi_version_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let mut reg = MockRegistry::new();

    // Two versions of `leaf`, plus `cycle-a <-> cycle-b`, plus scoped `@org/mid`.
    reg.publish_simple(
        "leaf",
        "1.0.0",
        false,
        r#"{"name":"leaf","version":"1.0.0","type":"module","exports":"./index.js"}"#,
        &[("index.js", "export const v = 1;\n")],
    );
    reg.publish_simple(
        "leaf",
        "2.0.0",
        true,
        r#"{"name":"leaf","version":"2.0.0","type":"module","exports":"./index.js"}"#,
        &[("index.js", "export const v = 2;\n")],
    );
    reg.publish_simple(
        "@org/mid",
        "1.2.3",
        true,
        r#"{"name":"@org/mid","version":"1.2.3","type":"module","exports":"./mod.mjs","dependencies":{"leaf":"^1.0.0"}}"#,
        &[("mod.mjs", "import { v } from 'leaf'; export const mid = v;\n")],
    );
    reg.publish_simple(
        "cycle-a",
        "1.0.0",
        true,
        r#"{"name":"cycle-a","version":"1.0.0","type":"module","exports":"./a.js","dependencies":{"cycle-b":"1.0.0"}}"#,
        &[("a.js", "import './b'; export const a = 1;\n")],
    );
    reg.publish_simple(
        "cycle-b",
        "1.0.0",
        true,
        r#"{"name":"cycle-b","version":"1.0.0","type":"module","exports":"./b.js","dependencies":{"cycle-a":"1.0.0"}}"#,
        &[("b.js", "export const b = 2;\n")],
    );

    let resolver = NpmGraphResolver::new(&reg, cas);
    let specs = vec![
        NpmSpecifier::parse("npm:@org/mid@^1.2.0").unwrap(),
        NpmSpecifier::parse("npm:leaf").unwrap(), // defaults to dist-tag "latest" -> 2.0.0
        NpmSpecifier::parse("npm:cycle-a@1.0.0").unwrap(),
    ];
    let graph = resolver.resolve_specifiers(&specs).unwrap();

    // Verify no node_modules directory exists anywhere under cache root.
    assert!(!tmp.path().join("node_modules").exists());

    let mid_url = graph.resolve_root_specifier(&specs[0]).unwrap();
    let leaf2_url = graph.resolve_root_specifier(&specs[1]).unwrap();
    let cycle_a_url = graph.resolve_root_specifier(&specs[2]).unwrap();

    assert_eq!(
        mid_url.as_str(),
        "npm://registry.npmjs.org/@org/mid/1.2.3/0/mod.mjs"
    );
    assert_eq!(
        leaf2_url.as_str(),
        "npm://registry.npmjs.org/leaf/2.0.0/0/index.js"
    );
    assert_eq!(
        cycle_a_url.as_str(),
        "npm://registry.npmjs.org/cycle-a/1.0.0/0/a.js"
    );

    // `@org/mid` resolves `leaf` to 1.0.0 (separate from root `leaf@2.0.0`).
    let mid_leaf = graph.resolve_from_npm_referrer(&mid_url, "leaf").unwrap();
    assert_eq!(
        mid_leaf,
        NpmReferrerResolution::NpmUrl(
            "npm://registry.npmjs.org/leaf/1.0.0/0/index.js"
                .parse()
                .unwrap()
        )
    );

    // Cycle edges terminate cleanly and resolve back and forth.
    let NpmReferrerResolution::NpmUrl(cycle_b_url) = graph
        .resolve_from_npm_referrer(&cycle_a_url, "cycle-b")
        .unwrap()
    else {
        panic!("expected NpmUrl");
    };
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&cycle_b_url, "cycle-a")
            .unwrap(),
        NpmReferrerResolution::NpmUrl(cycle_a_url.clone())
    );

    // Content-verified load returns exact owned bytes and ModuleFormat.
    let loaded = graph.load_module_verified(&leaf2_url).unwrap();
    assert_eq!(loaded.bytes, b"export const v = 2;\n");
    assert_eq!(loaded.format, ModuleFormat::JavaScript);
    assert_eq!(loaded.package_type, PackageType::Module);
}

#[test]
fn peer_context_isolation_produces_distinct_urls_and_handles_sibling_order() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let mut reg = MockRegistry::new();

    reg.publish_simple(
        "host",
        "1.0.0",
        false,
        r#"{"name":"host","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const host = 1;\n")],
    );
    reg.publish_simple(
        "host",
        "2.0.0",
        true,
        r#"{"name":"host","version":"2.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const host = 2;\n")],
    );
    reg.publish_simple(
        "@scope/plug",
        "2.1.0",
        true,
        r#"{"name":"@scope/plug","version":"2.1.0","type":"module","exports":"./dist/index.mjs","peerDependencies":{"host":"^1.0.0 || ^2.0.0"}}"#,
        &[("dist/index.mjs", "import { host } from 'host'; export const p = host;\n")],
    );
    reg.publish_simple(
        "app-one",
        "1.0.0",
        true,
        r#"{"name":"app-one","version":"1.0.0","type":"module","exports":"./index.mjs","dependencies":{"@scope/plug":"2.1.0","host":"1.0.0"}}"#,
        &[("index.mjs", "export { p } from '@scope/plug';\n")],
    );
    reg.publish_simple(
        "app-two",
        "1.0.0",
        true,
        r#"{"name":"app-two","version":"1.0.0","type":"module","exports":"./index.mjs","dependencies":{"@scope/plug":"2.1.0","host":"2.0.0"}}"#,
        &[("index.mjs", "export { p } from '@scope/plug';\n")],
    );

    // Same `@scope/plug@2.1.0` artifact under two different peer contexts yields
    // two distinct virtual URLs with different `p<32-hex>` context tokens.
    let resolver = NpmGraphResolver::new(&reg, cas.clone());
    let app1_spec = NpmSpecifier::parse("npm:app-one@1.0.0").unwrap();
    let app2_spec = NpmSpecifier::parse("npm:app-two@1.0.0").unwrap();
    let graph = resolver
        .resolve_specifiers(&[app1_spec.clone(), app2_spec.clone()])
        .unwrap();

    let app1_url = graph.resolve_root_specifier(&app1_spec).unwrap();
    let app2_url = graph.resolve_root_specifier(&app2_spec).unwrap();

    let NpmReferrerResolution::NpmUrl(plug_url_1) = graph
        .resolve_from_npm_referrer(&app1_url, "@scope/plug")
        .unwrap()
    else {
        panic!("expected NpmUrl");
    };
    let NpmReferrerResolution::NpmUrl(plug_url_2) = graph
        .resolve_from_npm_referrer(&app2_url, "@scope/plug")
        .unwrap()
    else {
        panic!("expected NpmUrl");
    };

    assert_ne!(plug_url_1, plug_url_2);
    assert!(plug_url_1.as_str().contains("/@scope/plug/2.1.0/p"));
    assert!(plug_url_2.as_str().contains("/@scope/plug/2.1.0/p"));

    // Each peer-context instance resolves `host` to its own bound version.
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&plug_url_1, "host")
            .unwrap(),
        NpmReferrerResolution::NpmUrl(
            "npm://registry.npmjs.org/host/1.0.0/0/index.mjs"
                .parse()
                .unwrap()
        )
    );
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&plug_url_2, "host")
            .unwrap(),
        NpmReferrerResolution::NpmUrl(
            "npm://registry.npmjs.org/host/2.0.0/0/index.mjs"
                .parse()
                .unwrap()
        )
    );

    // Sibling root peer visibility regardless of import order (`@scope/plug` listed BEFORE `host`).
    let sibling_specs = vec![
        NpmSpecifier::parse("npm:@scope/plug@2.1.0").unwrap(),
        NpmSpecifier::parse("npm:host@1.0.0").unwrap(),
    ];
    let sibling_graph = resolver.resolve_specifiers(&sibling_specs).unwrap();
    let sibling_plug = sibling_graph
        .resolve_root_specifier(&sibling_specs[0])
        .unwrap();
    assert_eq!(sibling_plug, plug_url_1);
}

#[test]
fn exports_key_order_wildcards_null_imports_self_ref_fallback_and_cjs_rejection() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let mut reg = MockRegistry::new();

    // 1. Key-order sensitivity: `default` before `import` -> `default.mjs` wins!
    reg.publish_simple(
        "order-pkg",
        "1.0.0",
        true,
        r##"{
            "name": "order-pkg",
            "version": "1.0.0",
            "type": "module",
            "exports": {
                ".": {
                    "require": "./cjs.cjs",
                    "default": "./default.mjs",
                    "import": "./import.mjs"
                },
                "./*": "./dist/*.mjs",
                "./secret": null,
                "./null-import": {
                    "import": null,
                    "default": "./default.mjs"
                }
            },
            "imports": {
                "#internal": "./src/internal.ts",
                "#ext": "helper"
            },
            "dependencies": {
                "helper": "1.0.0"
            }
        }"##,
        &[
            ("default.mjs", "export const which = 'default';\n"),
            ("import.mjs", "export const which = 'import';\n"),
            ("dist/feat.mjs", "export const feat = true;\n"),
            ("src/internal.ts", "export const x: number = 1;\n"),
            ("cjs.cjs", "module.exports = {};\n"),
        ],
    );
    reg.publish_simple(
        "helper",
        "1.0.0",
        true,
        r#"{"name":"helper","version":"1.0.0","type":"module","exports":"./h.mjs"}"#,
        &[("h.mjs", "export const h = 1;\n")],
    );

    // 2. Fallback `main` without `exports`, plus nested `type: "commonjs"` rejection.
    reg.publish_simple(
        "fallback-pkg",
        "1.0.0",
        true,
        r#"{"name":"fallback-pkg","version":"1.0.0","type":"module","main":"./lib/entry.js"}"#,
        &[
            ("lib/entry.js", "export const ok = 1;\n"),
            ("legacy/package.json", r#"{"type":"commonjs"}"#),
            ("legacy/old.js", "module.exports = 1;\n"),
            ("legacy/modern.mjs", "export const m = 1;\n"),
        ],
    );

    // 3. Untyped package (`type` omitted) -> `.js` rejected, `.mjs` allowed.
    reg.publish_simple(
        "untyped-pkg",
        "1.0.0",
        true,
        r#"{"name":"untyped-pkg","version":"1.0.0","main":"./index.js"}"#,
        &[
            ("index.js", "module.exports = 1;\n"),
            ("esm.mjs", "export const e = 1;\n"),
        ],
    );

    let resolver = NpmGraphResolver::new(&reg, cas);
    let specs = vec![
        NpmSpecifier::parse("npm:order-pkg@1.0.0").unwrap(),
        NpmSpecifier::parse("npm:fallback-pkg@1.0.0").unwrap(),
        NpmSpecifier::parse("npm:untyped-pkg@1.0.0/esm.mjs").unwrap(),
    ];
    let graph = resolver.resolve_specifiers(&specs).unwrap();

    // Condition key order: `default` preceded `import`, so `default.mjs` won.
    let order_root = graph.resolve_root_specifier(&specs[0]).unwrap();
    assert_eq!(
        order_root.as_str(),
        "npm://registry.npmjs.org/order-pkg/1.0.0/0/default.mjs"
    );

    // Wildcard export `./feat` -> `./dist/feat.mjs`.
    let feat_url = graph
        .resolve_root_specifier(&NpmSpecifier::parse("npm:order-pkg@1.0.0/feat").unwrap())
        .unwrap();
    assert_eq!(
        feat_url.as_str(),
        "npm://registry.npmjs.org/order-pkg/1.0.0/0/dist/feat.mjs"
    );

    // `null` exclusions fail closed.
    assert!(matches!(
        graph.resolve_root_specifier(&NpmSpecifier::parse("npm:order-pkg@1.0.0/secret").unwrap()),
        Err(ResolverError::Exports(
            ExportsError::PackagePathNotExported { .. }
        ))
    ));
    assert!(matches!(
        graph.resolve_root_specifier(
            &NpmSpecifier::parse("npm:order-pkg@1.0.0/null-import").unwrap()
        ),
        Err(ResolverError::Exports(
            ExportsError::PackagePathNotExported { .. }
        ))
    ));

    // `#imports` internal `.ts` (allowed under `type: "module"`) & external `helper`.
    let NpmReferrerResolution::NpmUrl(internal_ts) = graph
        .resolve_from_npm_referrer(&order_root, "#internal")
        .unwrap()
    else {
        panic!("expected NpmUrl");
    };
    let loaded_ts = graph.load_module_verified(&internal_ts).unwrap();
    assert_eq!(loaded_ts.format, ModuleFormat::TypeScript);

    assert_eq!(
        graph
            .resolve_from_npm_referrer(&order_root, "#ext")
            .unwrap(),
        NpmReferrerResolution::NpmUrl(
            "npm://registry.npmjs.org/helper/1.0.0/0/h.mjs"
                .parse()
                .unwrap()
        )
    );

    // Self-reference `order-pkg/feat` from inside `order-pkg`.
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&order_root, "order-pkg/feat")
            .unwrap(),
        NpmReferrerResolution::NpmUrl(feat_url)
    );

    // Same-instance `npm://` URL allowed; cross-package `npm://` URL rejected; `https:`/`file:` rejected; `neo:`/`node:` passed through.
    assert!(
        graph
            .resolve_from_npm_referrer(
                &order_root,
                "npm://registry.npmjs.org/order-pkg/1.0.0/0/dist/feat.mjs"
            )
            .is_ok()
    );
    assert!(
        graph
            .resolve_from_npm_referrer(&order_root, "npm://registry.npmjs.org/helper/1.0.0/0/h.mjs")
            .is_err()
    );
    assert!(
        graph
            .resolve_from_npm_referrer(&order_root, "https://example.com/evil.js")
            .is_err()
    );
    assert!(
        graph
            .resolve_from_npm_referrer(&order_root, "../escape.mjs")
            .is_err()
    );
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&order_root, "neo:assert")
            .unwrap(),
        NpmReferrerResolution::NeoBuiltin("neo:assert".to_owned())
    );
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&order_root, "node:path")
            .unwrap(),
        NpmReferrerResolution::NodeBuiltin("node:path".to_owned())
    );

    // Fallback `main` works; nested `legacy/old.js` under `"type": "commonjs"` fails closed;
    // nested `legacy/modern.mjs` succeeds.
    let fb_root = graph.resolve_root_specifier(&specs[1]).unwrap();
    assert_eq!(
        fb_root.as_str(),
        "npm://registry.npmjs.org/fallback-pkg/1.0.0/0/lib/entry.js"
    );
    assert!(matches!(
        graph.resolve_from_npm_referrer(&fb_root, "../legacy/old.js"),
        Err(ResolverError::Exports(
            ExportsError::UnsupportedModuleFormat { .. }
        ))
    ));
    assert!(
        graph
            .resolve_from_npm_referrer(&fb_root, "../legacy/modern.mjs")
            .is_ok()
    );

    // Untyped package `.js` root fails closed even offline.
    let untyped_mjs = graph.resolve_root_specifier(&specs[2]).unwrap();
    assert!(
        graph
            .resolve_from_npm_referrer(&untyped_mjs, "./index.js")
            .is_err()
    );
}

#[test]
fn tampered_cas_blob_fails_closed_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let mut reg = MockRegistry::new();

    reg.publish_simple(
        "tamper-test",
        "1.0.0",
        true,
        r#"{"name":"tamper-test","version":"1.0.0","type":"module","exports":"./mod.mjs"}"#,
        &[("mod.mjs", "export const clean = true;\n")],
    );

    let resolver = NpmGraphResolver::new(&reg, cas.clone());
    let spec = NpmSpecifier::parse("npm:tamper-test@1.0.0").unwrap();
    let graph = resolver
        .resolve_specifiers(std::slice::from_ref(&spec))
        .unwrap();
    let url = graph.resolve_root_specifier(&spec).unwrap();

    // First load succeeds.
    assert!(graph.load_module_verified(&url).is_ok());

    // Locate the CAS blob on disk and tamper with its bytes.
    let inst = graph.instances().values().next().unwrap();
    let entry = inst.tree.get_file("mod.mjs").unwrap();
    let blob_path = cas.blob_path(&entry.sha256).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&blob_path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    fs::write(&blob_path, b"export const clean = HACK;\n").unwrap();

    // Subsequent offline load fails closed with IntegrityMismatch.
    let err = graph.load_module_verified(&url).unwrap_err();
    assert!(
        matches!(err, ResolverError::Cas(CasError::IntegrityMismatch { .. })),
        "expected CasError::IntegrityMismatch, got {err:?}"
    );
}

#[test]
fn tarball_security_rejects_bad_sri_sha1_traversal_links_node_modules_bombs_and_audits_scripts() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let ingestor = TarballIngestor::new(cas.clone(), NpmLimits::default());
    let origin = "https://registry.npmjs.org";

    let (valid_tgz, valid_sri) = build_tgz(&[
        TarEntrySpec::dir("package/"),
        TarEntrySpec::file(
            "package/package.json",
            br#"{"name":"sec","version":"1.0.0","type":"module","scripts":{"postinstall":"rm -rf /","prepare":"echo hi"},"gypfile":true}"#,
        ),
        TarEntrySpec::pax_file("package/sub/pax-long.mjs", b"export const pax = 1;\n".to_vec()),
        TarEntrySpec::file("package/build/addon.node", b"\x7fELF".to_vec()),
    ]);

    // 1. Bad SRI
    let wrong_sri = format!("sha512-{}", BASE64_STANDARD.encode([0u8; 64]));
    assert!(matches!(
        ingestor.ingest(origin, &wrong_sri, &valid_tgz),
        Err(TarballError::IntegrityMismatch { .. })
    ));

    // 2. Missing / sha1-only SRI
    assert!(matches!(
        ingestor.ingest(origin, "", &valid_tgz),
        Err(TarballError::MissingIntegrity)
    ));
    assert!(matches!(
        ingestor.ingest(origin, "sha1-2jmj7l5rSw0yVb/vlWAYkK/YBwk=", &valid_tgz),
        Err(TarballError::WeakIntegrity { .. })
    ));

    // 3. Valid ingestion: PAX entry retained, `.node` native binary excluded from CAS, scripts audited!
    let tree = ingestor.ingest(origin, &valid_sri, &valid_tgz).unwrap();
    assert!(tree.get_file("sub/pax-long.mjs").is_some());
    assert!(tree.get_file("build/addon.node").is_none());
    assert!(tree.is_skipped_native("build/addon.node"));

    let pkg_entry = tree.get_file("package.json").unwrap();
    let pkg = neojs_npm::PackageJson::parse_bytes(
        &cas.read_verified(&pkg_entry.sha256, pkg_entry.size)
            .unwrap(),
    )
    .unwrap();
    let audit = pkg.build_script_audit(
        "sec",
        "1.0.0",
        tree.has_binding_gyp,
        tree.skipped_native.clone(),
    );
    assert_eq!(audit.ignored_scripts, vec!["postinstall", "prepare"]);
    assert!(audit.has_gypfile);
    assert_eq!(audit.skipped_native_binaries, vec!["build/addon.node"]);

    // Verify `.tree` reload verifies tree_sha256 and detects tampering.
    let reloaded =
        PackageTree::load_verified(tmp.path(), origin, &valid_sri, Some(&tree.tree_sha256))
            .unwrap()
            .unwrap();
    assert_eq!(reloaded.tree_sha256, tree.tree_sha256);

    // 4. Zip-slip `../` rejection
    let (zip_slip_tgz, zip_slip_sri) =
        build_tgz(&[TarEntrySpec::file("package/../evil.js", b"evil".to_vec())]);
    assert!(matches!(
        ingestor.ingest(origin, &zip_slip_sri, &zip_slip_tgz),
        Err(TarballError::UnsafeEntryPath { .. })
    ));

    // 5. Symlink & hardlink rejection
    let (sym_tgz, sym_sri) = build_tgz(&[TarEntrySpec::symlink("package/link")]);
    assert!(matches!(
        ingestor.ingest(origin, &sym_sri, &sym_tgz),
        Err(TarballError::UnsupportedEntryType { .. })
    ));
    let (hard_tgz, hard_sri) = build_tgz(&[TarEntrySpec::hardlink("package/hard")]);
    assert!(matches!(
        ingestor.ingest(origin, &hard_sri, &hard_tgz),
        Err(TarballError::UnsupportedEntryType { .. })
    ));

    // 6. `node_modules/` segment rejection
    let (nm_tgz, nm_sri) = build_tgz(&[TarEntrySpec::file(
        "package/node_modules/nested/index.js",
        b"1".to_vec(),
    )]);
    assert!(matches!(
        ingestor.ingest(origin, &nm_sri, &nm_tgz),
        Err(TarballError::UnsafeEntryPath { .. })
    ));

    // 7. Duplicate path rejection
    let (dup_tgz, dup_sri) = build_tgz(&[
        TarEntrySpec::file(
            "package/package.json",
            br#"{"name":"d","version":"1.0.0"}"#.to_vec(),
        ),
        TarEntrySpec::file("package/index.mjs", b"export const a = 1;".to_vec()),
        TarEntrySpec::file("package/index.mjs", b"export const a = 2;".to_vec()),
    ]);
    assert!(matches!(
        ingestor.ingest(origin, &dup_sri, &dup_tgz),
        Err(TarballError::DuplicatePath { .. })
    ));

    // 8. Decompression-bomb size limit
    let bomb_limits = NpmLimits {
        max_tarball_uncompressed_bytes: 2048,
        ..NpmLimits::default()
    };
    let bomb_ingestor = TarballIngestor::new(cas, bomb_limits);
    let (bomb_tgz, bomb_sri) = build_tgz(&[
        TarEntrySpec::file(
            "package/package.json",
            br#"{"name":"b","version":"1.0.0"}"#.to_vec(),
        ),
        TarEntrySpec::file("package/big.mjs", vec![b'a'; 8192]),
    ]);
    assert!(matches!(
        bomb_ingestor.ingest(origin, &bomb_sri, &bomb_tgz),
        Err(TarballError::UncompressedLimitExceeded { .. })
    ));
}

#[test]
fn platform_checks_and_optional_omissions() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let mut reg = MockRegistry::new();

    reg.publish_simple(
        "win-only",
        "1.0.0",
        true,
        r#"{"name":"win-only","version":"1.0.0","type":"module","os":["win32"],"exports":"./win.mjs"}"#,
        &[("win.mjs", "export const w = 1;\n")],
    );
    reg.publish_simple(
        "cross",
        "1.0.0",
        true,
        r#"{"name":"cross","version":"1.0.0","type":"module","exports":"./index.mjs","optionalDependencies":{"win-only":"^1.0.0"}}"#,
        &[("index.mjs", "export const c = 1;\n")],
    );

    let linux_x64 = TargetPlatform {
        os: "linux".to_owned(),
        cpu: "x64".to_owned(),
        libc: Some("glibc".to_owned()),
    };
    let resolver = NpmGraphResolver::new(&reg, cas).with_platform(linux_x64);

    // Optional mismatch is omitted cleanly; required mismatch fails closed.
    let cross_spec = NpmSpecifier::parse("npm:cross@1.0.0").unwrap();
    let graph = resolver
        .resolve_specifiers(std::slice::from_ref(&cross_spec))
        .unwrap();
    let inst = graph.instances().get("cross@1.0.0~0").unwrap();
    assert_eq!(
        inst.optional_dependencies
            .get(&PackageName::parse("win-only").unwrap()),
        Some(&None)
    );

    let win_spec = NpmSpecifier::parse("npm:win-only@1.0.0").unwrap();
    assert!(matches!(
        resolver.resolve_specifiers(&[win_spec]),
        Err(ResolverError::UnsupportedPlatform { .. })
    ));
}

#[test]
fn project_package_json_maps_bare_imports_without_node_modules_and_never_intercepts_builtins() {
    let proj_dir = tempfile::tempdir().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let cas = CasStore::new(cache_dir.path());
    let mut reg = MockRegistry::new();

    reg.publish_simple(
        "pkg",
        "1.2.3",
        true,
        r#"{"name":"pkg","version":"1.2.3","type":"module","exports":{".":"./index.mjs","./sub":"./sub.mjs"}}"#,
        &[
            ("index.mjs", "export const main = 1;\n"),
            ("sub.mjs", "export const sub = 2;\n"),
        ],
    );
    reg.publish_simple(
        "@scope/pkg",
        "2.0.0",
        true,
        r#"{"name":"@scope/pkg","version":"2.0.0","type":"module","exports":"./scoped.mjs"}"#,
        &[("scoped.mjs", "export const scoped = 3;\n")],
    );

    fs::create_dir_all(proj_dir.path().join("src/nested")).unwrap();
    fs::write(
        proj_dir.path().join("src/internal.mjs"),
        "export const i = 1;\n",
    )
    .unwrap();
    fs::write(
        proj_dir.path().join("package.json"),
        r##"{
            "name": "my-app",
            "version": "0.1.0",
            "type": "module",
            "exports": {
                "./self": "./src/internal.mjs"
            },
            "imports": {
                "#internal": "./src/internal.mjs",
                "#dep": "pkg"
            },
            "dependencies": {
                "pkg": "^1.0.0",
                "@scope/pkg": ">=2.0.0 <3.0.0"
            },
            "scripts": {
                "postinstall": "should-never-run"
            }
        }"##,
    )
    .unwrap();

    let project = ProjectPackageResolver::discover(&proj_dir.path().join("src/nested"))
        .unwrap()
        .expect("should discover package.json upward");
    assert!(!proj_dir.path().join("node_modules").exists());

    let requests = project.root_requests().unwrap();
    let graph = NpmGraphResolver::new(&reg, cas)
        .resolve_requests(&requests)
        .unwrap();

    // Bare package, subpath, and scoped bare package map into virtual `npm://` URLs.
    assert_eq!(
        project.resolve_specifier("pkg", &graph).unwrap(),
        ProjectResolution::NpmModule(
            "npm://registry.npmjs.org/pkg/1.2.3/0/index.mjs"
                .parse()
                .unwrap()
        )
    );
    assert_eq!(
        project.resolve_specifier("pkg/sub", &graph).unwrap(),
        ProjectResolution::NpmModule(
            "npm://registry.npmjs.org/pkg/1.2.3/0/sub.mjs"
                .parse()
                .unwrap()
        )
    );
    assert_eq!(
        project.resolve_specifier("@scope/pkg", &graph).unwrap(),
        ProjectResolution::NpmModule(
            "npm://registry.npmjs.org/@scope/pkg/2.0.0/0/scoped.mjs"
                .parse()
                .unwrap()
        )
    );

    // `#imports` internal and external targets + self-reference.
    let canon_proj_dir = fs::canonicalize(proj_dir.path()).unwrap();
    assert_eq!(
        project.resolve_specifier("#internal", &graph).unwrap(),
        ProjectResolution::LocalFile {
            path: canon_proj_dir.join("src/internal.mjs"),
            format: ModuleFormat::JavaScript,
        }
    );
    assert_eq!(
        project.resolve_specifier("#dep", &graph).unwrap(),
        ProjectResolution::NpmModule(
            "npm://registry.npmjs.org/pkg/1.2.3/0/index.mjs"
                .parse()
                .unwrap()
        )
    );
    assert_eq!(
        project.resolve_specifier("my-app/self", &graph).unwrap(),
        ProjectResolution::LocalFile {
            path: canon_proj_dir.join("src/internal.mjs"),
            format: ModuleFormat::JavaScript,
        }
    );

    // Never intercept `neo:` or `node:` built-ins (including uppercase scheme variants).
    assert_eq!(
        project.resolve_specifier("neo:crypto", &graph).unwrap(),
        ProjectResolution::NotIntercepted
    );
    assert_eq!(
        project.resolve_specifier("NODE:fs", &graph).unwrap(),
        ProjectResolution::NotIntercepted
    );
    assert_eq!(
        project.resolve_specifier("NEO:assert", &graph).unwrap(),
        ProjectResolution::NotIntercepted
    );
    assert_eq!(
        project.resolve_specifier("node:fs", &graph).unwrap(),
        ProjectResolution::NotIntercepted
    );

    // Undeclared bare package fails closed.
    assert!(project.resolve_specifier("undeclared-pkg", &graph).is_err());
}

#[test]
fn regression_tarball_hardening_and_windows_drive_and_case_insensitive_builtins() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let ingestor = TarballIngestor::new(cas.clone(), NpmLimits::default());
    let origin = "https://registry.npmjs.org";

    // 1. Base-256 tar size overflow (`0x81` + trailing zeroes) -> MalformedTar.
    let mut overflow_hdr = write_tar_header("package/overflow.mjs", b'0', 0);
    overflow_hdr[124] = 0x81;
    let sum: u32 = overflow_hdr
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                u32::from(b' ')
            } else {
                u32::from(b)
            }
        })
        .sum();
    let csum = format!("{sum:06o}\0 ");
    overflow_hdr[148..156].copy_from_slice(csum.as_bytes());
    let mut overflow_tar = overflow_hdr.to_vec();
    overflow_tar.resize(overflow_tar.len() + 1024, 0);
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&overflow_tar).unwrap();
    let overflow_tgz = gz.finish().unwrap();
    let overflow_sri = format!(
        "sha512-{}",
        BASE64_STANDARD.encode(Sha512::digest(&overflow_tgz))
    );
    assert!(matches!(
        ingestor.ingest(origin, &overflow_sri, &overflow_tgz),
        Err(TarballError::MalformedTar { .. })
    ));

    // 2. PAX size override + GNU `L` longname + both `L` and `x` before one entry
    //    (verifying GNU `L` does NOT leak onto the subsequent entry).
    let mut pax_gnu_tar = Vec::new();
    let pkg_json = br#"{"name":"paxgnu","version":"1.0.0","type":"module"}"#;
    let pkg_hdr = write_tar_header("package/package.json", b'0', pkg_json.len());
    pax_gnu_tar.extend_from_slice(&pkg_hdr);
    pax_gnu_tar.extend_from_slice(pkg_json);
    pax_gnu_tar.resize(pax_gnu_tar.len() + (512 - (pkg_json.len() % 512)) % 512, 0);

    // Entry with GNU `L` followed by PAX `x` (both path and size).
    let gnu_name = b"package/leaked-gnu-name.mjs\0";
    let gnu_hdr = write_tar_header("././@LongLink", b'L', gnu_name.len());
    pax_gnu_tar.extend_from_slice(&gnu_hdr);
    pax_gnu_tar.extend_from_slice(gnu_name);
    pax_gnu_tar.resize(pax_gnu_tar.len() + (512 - (gnu_name.len() % 512)) % 512, 0);

    let body_a = b"export const a = 1;\n";
    let mut pax_payload = encode_pax_record("path", "package/pax-wins.mjs");
    pax_payload.extend_from_slice(&encode_pax_record("size", &body_a.len().to_string()));
    let pax_hdr = write_tar_header("PaxHeader/a", b'x', pax_payload.len());
    pax_gnu_tar.extend_from_slice(&pax_hdr);
    pax_gnu_tar.extend_from_slice(&pax_payload);
    pax_gnu_tar.resize(
        pax_gnu_tar.len() + (512 - (pax_payload.len() % 512)) % 512,
        0,
    );
    let hdr_a = write_tar_header("package/ignored.mjs", b'0', 0);
    pax_gnu_tar.extend_from_slice(&hdr_a);
    pax_gnu_tar.extend_from_slice(body_a);
    pax_gnu_tar.resize(pax_gnu_tar.len() + (512 - (body_a.len() % 512)) % 512, 0);

    // Subsequent normal entry MUST use its own header path, not `leaked-gnu-name.mjs`.
    let body_b = b"export const b = 2;\n";
    let hdr_b = write_tar_header("package/next.mjs", b'0', body_b.len());
    pax_gnu_tar.extend_from_slice(&hdr_b);
    pax_gnu_tar.extend_from_slice(body_b);
    pax_gnu_tar.resize(pax_gnu_tar.len() + (512 - (body_b.len() % 512)) % 512, 0);
    pax_gnu_tar.resize(pax_gnu_tar.len() + 1024, 0);

    let mut gz2 = GzEncoder::new(Vec::new(), Compression::default());
    gz2.write_all(&pax_gnu_tar).unwrap();
    let pax_gnu_tgz = gz2.finish().unwrap();
    let pax_gnu_sri = format!(
        "sha512-{}",
        BASE64_STANDARD.encode(Sha512::digest(&pax_gnu_tgz))
    );
    let tree = ingestor.ingest(origin, &pax_gnu_sri, &pax_gnu_tgz).unwrap();
    assert!(tree.get_file("pax-wins.mjs").is_some());
    assert!(tree.get_file("next.mjs").is_some());
    assert!(tree.get_file("leaked-gnu-name.mjs").is_none());

    // 3. Re-ingesting when on-disk `.tree` was tampered must fail closed.
    let sha512_hex = {
        let mut h = String::new();
        for b in Sha512::digest(&pax_gnu_tgz) {
            use std::fmt::Write as _;
            let _ = write!(h, "{b:02x}");
        }
        h
    };
    let tree_disk_path = cas
        .npm_origin_dir(origin)
        .join("trees")
        .join(format!("sha512-{sha512_hex}.tree"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tree_disk_path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    fs::write(
        &tree_disk_path,
        b"{\"version\":1,\"integrity\":\"sha512-x\"}",
    )
    .unwrap();
    assert!(matches!(
        ingestor.ingest(origin, &pax_gnu_sri, &pax_gnu_tgz),
        Err(TarballError::Cas(
            CasError::IntegrityMismatch { .. } | CasError::SizeMismatch { .. }
        ))
    ));

    // 4. Truncated tar / orphan PAX header at EOF -> MalformedTar.
    let mut orphan_tar = Vec::new();
    let orphan_pax = encode_pax_record("path", "package/orphan.mjs");
    orphan_tar.extend_from_slice(&write_tar_header(
        "PaxHeader/orphan",
        b'x',
        orphan_pax.len(),
    ));
    orphan_tar.extend_from_slice(&orphan_pax);
    orphan_tar.resize(orphan_tar.len() + (512 - (orphan_pax.len() % 512)) % 512, 0);
    let mut gz3 = GzEncoder::new(Vec::new(), Compression::default());
    gz3.write_all(&orphan_tar).unwrap();
    let orphan_tgz = gz3.finish().unwrap();
    let orphan_sri = format!(
        "sha512-{}",
        BASE64_STANDARD.encode(Sha512::digest(&orphan_tgz))
    );
    assert!(matches!(
        ingestor.ingest(origin, &orphan_sri, &orphan_tgz),
        Err(TarballError::MalformedTar { .. })
    ));

    // 5. Windows drive-path injection rejected in tarballs, specifiers, and exports.
    let (drv_tgz, drv_sri) = build_tgz(&[
        TarEntrySpec::file(
            "package/package.json",
            br#"{"name":"drv","version":"1.0.0"}"#.to_vec(),
        ),
        TarEntrySpec::file("package/C:/evil.mjs", b"export const bad = 1;".to_vec()),
    ]);
    assert!(matches!(
        ingestor.ingest(origin, &drv_sri, &drv_tgz),
        Err(TarballError::UnsafeEntryPath { .. })
    ));
    assert!(NpmSpecifier::parse("npm:pkg/C:/evil.mjs").is_err());

    let bad_exports_pkg = neojs_npm::PackageJson::parse_str(
        r##"{
            "name": "win-drive-test",
            "type": "module",
            "exports": { ".": "./C:/Windows/System32/evil.mjs" },
            "imports": { "#bad": "./C:/Windows/System32/evil.mjs" }
        }"##,
    )
    .unwrap();
    assert!(matches!(
        neojs_npm::ExportsResolver::resolve_exports_target(
            bad_exports_pkg.exports.as_ref().unwrap(),
            "win-drive-test",
            None
        ),
        Err(ExportsError::InvalidTarget { .. })
    ));
    assert!(matches!(
        neojs_npm::ExportsResolver::resolve_project_import(
            &bad_exports_pkg,
            tmp.path(),
            "win-drive-test",
            "#bad"
        ),
        Err(ExportsError::InvalidTarget { .. })
    ));

    // 6. Project `#imports` local ESM classification + case-insensitive `NEO:`/`NODE:` in npm referrers.
    let ts_proj = neojs_npm::PackageJson::parse_str(
        r##"{
            "name": "ts-proj",
            "type": "module",
            "imports": { "#mod": "./src/mod.ts", "#cjs": "./src/bad.cjs" }
        }"##,
    )
    .unwrap();
    let ts_res =
        neojs_npm::ExportsResolver::resolve_project_import(&ts_proj, tmp.path(), "ts-proj", "#mod")
            .unwrap();
    assert!(matches!(
        ts_res,
        neojs_npm::PackageImportTarget::InternalFile(f) if f.format == ModuleFormat::TypeScript
    ));
    assert!(matches!(
        neojs_npm::ExportsResolver::resolve_project_import(&ts_proj, tmp.path(), "ts-proj", "#cjs"),
        Err(ExportsError::UnsupportedModuleFormat { .. })
    ));

    let mut reg = MockRegistry::new();
    reg.publish_simple(
        "builtin-caller",
        "1.0.0",
        true,
        r#"{"name":"builtin-caller","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const x = 1;\n")],
    );
    let graph = NpmGraphResolver::new(&reg, cas)
        .resolve_specifiers(&[NpmSpecifier::parse("npm:builtin-caller@1.0.0").unwrap()])
        .unwrap();
    let caller_url: url::Url = "npm://registry.npmjs.org/builtin-caller/1.0.0/0/index.mjs"
        .parse()
        .unwrap();
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&caller_url, "NEO:crypto")
            .unwrap(),
        NpmReferrerResolution::NeoBuiltin("NEO:crypto".to_owned())
    );
    assert_eq!(
        graph
            .resolve_from_npm_referrer(&caller_url, "NODE:path")
            .unwrap(),
        NpmReferrerResolution::NodeBuiltin("NODE:path".to_owned())
    );
}

#[test]
fn lockfile_resolution_enforces_tree_hash_package_json_constraints_peers_and_ctx() {
    use neojs_npm::{
        DEFAULT_REGISTRY_ORIGIN, LockedPackage, Lockfile, NpmRootRequest, RemoteNpmRegistry,
        VersionConstraint, resolve_requests_with_lockfile,
    };

    let tmp = tempfile::tempdir().unwrap();
    let cas = CasStore::new(tmp.path());
    let mut reg = MockRegistry::new();

    reg.publish_simple(
        "host",
        "1.0.0",
        true,
        r#"{"name":"host","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const v = 1;\n")],
    );
    reg.publish_simple(
        "host",
        "2.0.0",
        true,
        r#"{"name":"host","version":"2.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const v = 2;\n")],
    );
    reg.publish_simple(
        "plug",
        "1.0.0",
        true,
        r#"{"name":"plug","version":"1.0.0","type":"module","exports":"./index.mjs","peerDependencies":{"host":"^1.0.0 || ^2.0.0"}}"#,
        &[("index.mjs", "export const p = 1;\n")],
    );
    reg.publish_simple(
        "leaf",
        "1.0.0",
        true,
        r#"{"name":"leaf","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const l = 1;\n")],
    );
    reg.publish_simple(
        "leaf",
        "2.0.0",
        true,
        r#"{"name":"leaf","version":"2.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const l = 2;\n")],
    );
    reg.publish_simple(
        "parent",
        "1.0.0",
        true,
        r#"{"name":"parent","version":"1.0.0","type":"module","exports":"./index.mjs","dependencies":{"leaf":"^1.0.0"},"devDependencies":{"dev-only":"^1.0.0"}}"#,
        &[("index.mjs", "export const p = 1;\n")],
    );
    reg.publish_simple(
        "dev-only",
        "1.0.0",
        true,
        r#"{"name":"dev-only","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        &[("index.mjs", "export const d = 1;\n")],
    );

    // 1. Incremental Lockfile::record_graph preserves `~<ctx>` whenever ctx != "0".
    let resolver = NpmGraphResolver::new(&reg, cas.clone());
    let g1 = resolver
        .resolve_specifiers(&[
            NpmSpecifier::parse("npm:host@1.0.0").unwrap(),
            NpmSpecifier::parse("npm:plug@1.0.0").unwrap(),
        ])
        .unwrap();
    let g2 = resolver
        .resolve_specifiers(&[
            NpmSpecifier::parse("npm:host@2.0.0").unwrap(),
            NpmSpecifier::parse("npm:plug@1.0.0").unwrap(),
        ])
        .unwrap();
    let mut lock = Lockfile::new();
    assert!(lock.record_graph(&g1));
    assert!(lock.record_graph(&g2));
    let plug_keys: Vec<_> = lock
        .packages
        .keys()
        .filter(|k| k.starts_with("plug@1.0.0~p"))
        .collect();
    assert_eq!(
        plug_keys.len(),
        2,
        "distinct ~<ctx> entries preserved: {plug_keys:?}"
    );

    // 2. Cold-cache treeSha256 mismatch fails closed with CasError::IntegrityMismatch.
    let cold_cache = tempfile::tempdir().unwrap();
    let cold_cas = CasStore::new(cold_cache.path());
    let parent_meta = reg
        .fetch_packument(&PackageName::parse("parent").unwrap())
        .unwrap();
    let parent_v = &parent_meta.versions["1.0.0"];
    let mut cold_lock = Lockfile::new();
    cold_lock
        .specifiers
        .insert("parent@^1.0.0".to_owned(), "parent@1.0.0".to_owned());
    cold_lock.packages.insert(
        "parent@1.0.0".to_owned(),
        LockedPackage {
            version: "1.0.0".to_owned(),
            integrity: parent_v.dist.integrity.clone().unwrap(),
            tree_sha256: Some("0".repeat(64)),
            tarball_url: parent_v.dist.tarball.clone(),
            dependencies: BTreeMap::new(),
        },
    );
    let req_parent = vec![NpmRootRequest {
        name: PackageName::parse("parent").unwrap(),
        constraint: VersionConstraint::parse("^1.0.0").unwrap(),
        optional: false,
    }];
    let err = resolve_requests_with_lockfile(
        &req_parent,
        &mut cold_lock,
        &cold_cas,
        Some(&reg),
        DEFAULT_REGISTRY_ORIGIN,
        true,
        false,
        NpmLimits::default(),
        &TargetPlatform::current(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ResolverError::Cas(CasError::IntegrityMismatch { .. })),
        "{err}"
    );

    // 3. Frozen lockfile requires treeSha256.
    cold_lock
        .packages
        .get_mut("parent@1.0.0")
        .unwrap()
        .tree_sha256 = None;
    let err = resolve_requests_with_lockfile(
        &req_parent,
        &mut cold_lock,
        &cold_cas,
        Some(&reg),
        DEFAULT_REGISTRY_ORIGIN,
        true,
        false,
        NpmLimits::default(),
        &TargetPlatform::current(),
    )
    .unwrap_err();
    assert!(matches!(err, ResolverError::FrozenLockfile { .. }), "{err}");

    // Seed valid parent + leaf@1.0.0 + leaf@2.0.0 + dev-only@1.0.0 into `cas`.
    let base_graph = resolver
        .resolve_specifiers(&[
            NpmSpecifier::parse("npm:parent@^1.0.0").unwrap(),
            NpmSpecifier::parse("npm:leaf@2.0.0").unwrap(),
            NpmSpecifier::parse("npm:dev-only@1.0.0").unwrap(),
        ])
        .unwrap();
    let mut base_lock = Lockfile::new();
    base_lock.record_graph(&base_graph);

    // 4. Reject locked dependency not declared in package.json (e.g. injected devDependencies).
    let mut dev_inject = base_lock.clone();
    dev_inject
        .packages
        .get_mut("parent@1.0.0")
        .unwrap()
        .dependencies
        .insert("dev-only".to_owned(), "dev-only@1.0.0".to_owned());
    let err = resolve_requests_with_lockfile(
        &req_parent,
        &mut dev_inject,
        &cas,
        None::<&MockRegistry>,
        DEFAULT_REGISTRY_ORIGIN,
        true,
        true,
        NpmLimits::default(),
        &TargetPlatform::current(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ResolverError::DisallowedImport { .. }),
        "{err}"
    );

    // 5. Reject locked child version violating parent's SemVer constraint (`leaf@^1.0.0` vs `2.0.0`).
    let mut bad_child_ver = base_lock.clone();
    bad_child_ver
        .packages
        .get_mut("parent@1.0.0")
        .unwrap()
        .dependencies
        .insert("leaf".to_owned(), "leaf@2.0.0".to_owned());
    let err = resolve_requests_with_lockfile(
        &req_parent,
        &mut bad_child_ver,
        &cas,
        None::<&MockRegistry>,
        DEFAULT_REGISTRY_ORIGIN,
        true,
        true,
        NpmLimits::default(),
        &TargetPlatform::current(),
    )
    .unwrap_err();
    assert!(matches!(err, ResolverError::FrozenLockfile { .. }), "{err}");

    // 6. Reject locked package.json version mismatch.
    let mut bad_pkg_ver = base_lock.clone();
    let parent_pkg = bad_pkg_ver.packages.remove("parent@1.0.0").unwrap();
    bad_pkg_ver
        .specifiers
        .insert("parent@^1.0.0".to_owned(), "parent@1.0.1".to_owned());
    bad_pkg_ver.packages.insert(
        "parent@1.0.1".to_owned(),
        LockedPackage {
            version: "1.0.1".to_owned(),
            ..parent_pkg
        },
    );
    let err = resolve_requests_with_lockfile(
        &req_parent,
        &mut bad_pkg_ver,
        &cas,
        None::<&MockRegistry>,
        DEFAULT_REGISTRY_ORIGIN,
        true,
        true,
        NpmLimits::default(),
        &TargetPlatform::current(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ResolverError::PackageVersionMismatch { .. }),
        "{err}"
    );

    // 7. Reject missing required peerDependency in frozen mode.
    let mut missing_peer_lock = Lockfile::new();
    missing_peer_lock.record_graph(&g1);
    let plug_key = missing_peer_lock
        .packages
        .keys()
        .find(|k| k.starts_with("plug@"))
        .unwrap()
        .clone();
    missing_peer_lock
        .packages
        .get_mut(&plug_key)
        .unwrap()
        .dependencies
        .clear();
    let req_plug = vec![NpmRootRequest {
        name: PackageName::parse("plug").unwrap(),
        constraint: VersionConstraint::parse("1.0.0").unwrap(),
        optional: false,
    }];
    let err = resolve_requests_with_lockfile(
        &req_plug,
        &mut missing_peer_lock,
        &cas,
        None::<&MockRegistry>,
        DEFAULT_REGISTRY_ORIGIN,
        true,
        true,
        NpmLimits::default(),
        &TargetPlatform::current(),
    )
    .unwrap_err();
    assert!(matches!(err, ResolverError::FrozenLockfile { .. }), "{err}");

    // 8. RemoteNpmRegistry::fetch_tarball redacts credentials and query tokens from errors.
    struct FailingTransport;
    impl neojs_remote::RemoteTransport for FailingTransport {
        fn get(
            &self,
            _request: &neojs_remote::RemoteRequest<'_>,
        ) -> Result<neojs_remote::RemoteResponse, String> {
            Err("simulated transport failure".to_owned())
        }
    }
    let client = neojs_remote::RemoteClient::with_transport(
        neojs_remote::RemoteOptions::new().without_cache(),
        std::sync::Arc::new(FailingTransport),
    );
    let remote_reg = RemoteNpmRegistry::with_client(DEFAULT_REGISTRY_ORIGIN, client);
    let err = remote_reg
        .fetch_tarball(
            "https://user:supersecret@registry.npmjs.org/pkg.tgz?token=leak123",
            "sha512-x",
        )
        .unwrap_err();
    let msg = err.to_string();
    assert!(!msg.contains("supersecret"), "credential leaked: {msg}");
    assert!(!msg.contains("leak123"), "query token leaked: {msg}");
    let bad_err = remote_reg
        .fetch_tarball("not a url ?secret=abc", "sha512-x")
        .unwrap_err();
    let bad_msg = bad_err.to_string();
    assert!(
        !bad_msg.contains("secret=abc"),
        "malformed URL leaked: {bad_msg}"
    );
}

#[test]
fn npmrc_scoped_routing_auth_env_redirect_stripping_and_offline_replay() {
    use std::sync::{Arc, Mutex};

    use neojs_npm::{
        DEFAULT_REGISTRY_ORIGIN, Lockfile, NpmRootRequest, NpmrcConfig, NpmrcError,
        RemoteNpmRegistry, VersionConstraint, resolve_requests_with_lockfile_and_npmrc,
    };
    use neojs_remote::{
        RemoteClient, RemoteOptions, RemoteRequest, RemoteResponse, RemoteTransport,
    };

    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let cas = CasStore::new(cache_dir.path());

    // User ~/.npmrc defines a default mirror and fallback auth; project .npmrc
    // overrides default registry back to npmjs, adds @private + @basic scopes,
    // configures path-specific auth, and leaves ${MISSING_TOKEN} unset for @unset.
    fs::write(
        home.path().join(".npmrc"),
        r#"
            registry=https://user-mirror.example.test/
            //priv.example.test/:_authToken=user_stale_token
        "#,
    )
    .unwrap();
    fs::write(
        project.path().join(".npmrc"),
        r#"
            # Project overrides user settings
            registry=https://registry.npmjs.org/
            @Private:registry=https://priv.example.test/team/
            @Basic:registry=https://basic.example.test/
            @Unset:registry=https://unset.example.test/
            //priv.example.test/:_authToken=root_should_not_win
            //priv.example.test/team/:_authToken=${TEAM_TOKEN}
            //basic.example.test/:_auth=${BASIC_CREDS}
            //unset.example.test/:_authToken=${MISSING_TOKEN}
        "#,
    )
    .unwrap();

    let npmrc =
        NpmrcConfig::load_for_project(project.path(), Some(home.path()), &|name| match name {
            "TEAM_TOKEN" => Some("team_secret_token".to_owned()),
            "BASIC_CREDS" => Some("dXNlcjpwYXNz".to_owned()),
            _ => None,
        })
        .unwrap();

    // Verify Debug output never leaks secrets.
    let dbg = format!("{npmrc:?}");
    assert!(!dbg.contains("team_secret_token"));
    assert!(!dbg.contains("dXNlcjpwYXNz"));
    assert!(!dbg.contains("user_stale_token"));

    let (pub_tgz, pub_sri) = build_tgz(&[
        TarEntrySpec::file(
            "package/package.json",
            r#"{"name":"pub-lib","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        ),
        TarEntrySpec::file("package/index.mjs", "export const pub = 'public';\n"),
    ]);
    let (priv_tgz, priv_sri) = build_tgz(&[
        TarEntrySpec::file(
            "package/package.json",
            r#"{"name":"@private/lib","version":"1.0.0","type":"module","exports":"./index.mjs","dependencies":{"pub-lib":"^1.0.0"}}"#,
        ),
        TarEntrySpec::file(
            "package/index.mjs",
            "import { pub } from 'pub-lib'; export const val = `priv:${pub}`;\n",
        ),
    ]);
    let (basic_tgz, basic_sri) = build_tgz(&[
        TarEntrySpec::file(
            "package/package.json",
            r#"{"name":"@basic/lib","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        ),
        TarEntrySpec::file("package/index.mjs", "export const b = 'basic-ok';\n"),
    ]);

    type RouteEntry = (u16, Option<String>, Vec<u8>);
    type RequestLog = Vec<(String, Option<String>)>;

    #[derive(Clone, Default)]
    struct MockAuthTransport {
        routes: Arc<Mutex<BTreeMap<String, RouteEntry>>>,
        log: Arc<Mutex<RequestLog>>,
    }
    impl RemoteTransport for MockAuthTransport {
        fn get(&self, request: &RemoteRequest<'_>) -> Result<RemoteResponse, String> {
            self.log.lock().unwrap().push((
                request.url.as_str().to_owned(),
                request.authorization.map(str::to_owned),
            ));
            let guard = self.routes.lock().unwrap();
            if let Some((status, location, body)) = guard.get(request.url.as_str()) {
                Ok(RemoteResponse {
                    status: *status,
                    location: location.clone(),
                    declared_length: Some(body.len() as u64),
                    etag: None,
                    last_modified: None,
                    cache_control: Some("max-age=60".to_owned()),
                    date: None,
                    age: None,
                    body: Box::new(std::io::Cursor::new(body.clone())),
                })
            } else {
                Ok(RemoteResponse {
                    status: 404,
                    location: None,
                    declared_length: None,
                    etag: None,
                    last_modified: None,
                    cache_control: None,
                    date: None,
                    age: None,
                    body: Box::new(std::io::Cursor::new(b"not found".to_vec())),
                })
            }
        }
    }

    let transport = MockAuthTransport::default();
    {
        let mut r = transport.routes.lock().unwrap();
        // Public package on registry.npmjs.org
        let pub_pack = serde_json::json!({
            "name": "pub-lib",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "version": "1.0.0",
                    "dist": {
                        "tarball": "https://registry.npmjs.org/pub-lib/-/1.0.0.tgz",
                        "integrity": pub_sri
                    }
                }
            }
        });
        r.insert(
            "https://registry.npmjs.org/pub-lib".to_owned(),
            (200, None, serde_json::to_vec(&pub_pack).unwrap()),
        );
        r.insert(
            "https://registry.npmjs.org/pub-lib/-/1.0.0.tgz".to_owned(),
            (200, None, pub_tgz),
        );

        // @private/lib on https://priv.example.test/team/, tarball redirects cross-origin to CDN!
        let priv_pack = serde_json::json!({
            "name": "@private/lib",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "version": "1.0.0",
                    "dependencies": { "pub-lib": "^1.0.0" },
                    "dist": {
                        "tarball": "https://priv.example.test/team/@private/lib/-/1.0.0.tgz",
                        "integrity": priv_sri
                    }
                }
            }
        });
        r.insert(
            "https://priv.example.test/team/@private%2flib".to_owned(),
            (200, None, serde_json::to_vec(&priv_pack).unwrap()),
        );
        r.insert(
            "https://priv.example.test/team/@private/lib/-/1.0.0.tgz".to_owned(),
            (
                302,
                Some("https://cdn.unrelated.test/blobs/priv-1.0.0.tgz".to_owned()),
                Vec::new(),
            ),
        );
        r.insert(
            "https://cdn.unrelated.test/blobs/priv-1.0.0.tgz".to_owned(),
            (200, None, priv_tgz),
        );

        // @basic/lib on https://basic.example.test/
        let basic_pack = serde_json::json!({
            "name": "@basic/lib",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "version": "1.0.0",
                    "dist": {
                        "tarball": "https://basic.example.test/@basic/lib/-/1.0.0.tgz",
                        "integrity": basic_sri
                    }
                }
            }
        });
        r.insert(
            "https://basic.example.test/@basic%2flib".to_owned(),
            (200, None, serde_json::to_vec(&basic_pack).unwrap()),
        );
        r.insert(
            "https://basic.example.test/@basic/lib/-/1.0.0.tgz".to_owned(),
            (200, None, basic_tgz),
        );
    }

    let client = RemoteClient::with_transport(
        RemoteOptions::new().without_cache(),
        Arc::new(transport.clone()),
    );
    let remote_reg =
        RemoteNpmRegistry::with_client(DEFAULT_REGISTRY_ORIGIN, client).with_npmrc(npmrc.clone());

    let mut lockfile = Lockfile::new();
    let reqs = vec![
        NpmRootRequest {
            name: PackageName::parse("@private/lib").unwrap(),
            constraint: VersionConstraint::parse("^1.0.0").unwrap(),
            optional: false,
        },
        NpmRootRequest {
            name: PackageName::parse("@basic/lib").unwrap(),
            constraint: VersionConstraint::parse("^1.0.0").unwrap(),
            optional: false,
        },
    ];
    let (graph, modified) = resolve_requests_with_lockfile_and_npmrc(
        &reqs,
        &mut lockfile,
        &cas,
        Some(&remote_reg),
        DEFAULT_REGISTRY_ORIGIN,
        false,
        false,
        NpmLimits::default(),
        &TargetPlatform::current(),
        Some(&npmrc),
        false,
    )
    .unwrap();
    assert!(modified);

    let log = transport.log.lock().unwrap().clone();
    assert!(
        log.contains(&(
            "https://priv.example.test/team/@private%2flib".to_owned(),
            Some("Bearer team_secret_token".to_owned())
        )),
        "expected Bearer team_secret_token for @private/lib packument: {log:?}"
    );
    assert!(
        log.contains(&(
            "https://priv.example.test/team/@private/lib/-/1.0.0.tgz".to_owned(),
            Some("Bearer team_secret_token".to_owned())
        )),
        "expected Bearer team_secret_token for @private/lib tarball: {log:?}"
    );
    assert!(
        log.contains(&(
            "https://cdn.unrelated.test/blobs/priv-1.0.0.tgz".to_owned(),
            None
        )),
        "cross-origin redirect MUST strip Authorization header: {log:?}"
    );
    assert!(
        log.contains(&("https://registry.npmjs.org/pub-lib".to_owned(), None)),
        "public package must NOT receive private Authorization header: {log:?}"
    );
    assert!(
        log.contains(&(
            "https://basic.example.test/@basic%2flib".to_owned(),
            Some("Basic dXNlcjpwYXNz".to_owned())
        )),
        "expected Basic auth for @basic/lib: {log:?}"
    );

    // Virtual URL of @private/lib uses priv.example.test host, while pub-lib uses registry.npmjs.org.
    let priv_spec = NpmSpecifier::parse("npm:@private/lib@^1.0.0").unwrap();
    let priv_url = graph.resolve_root_specifier(&priv_spec).unwrap();
    assert_eq!(priv_url.host_str(), Some("priv.example.test"));
    let pub_dep_res = graph
        .resolve_from_npm_referrer(&priv_url, "pub-lib")
        .unwrap();
    match pub_dep_res {
        NpmReferrerResolution::NpmUrl(url) => {
            assert_eq!(url.host_str(), Some("registry.npmjs.org"));
        }
        other => panic!("expected NpmUrl, got {other:?}"),
    }

    // Offline replay with lockfile + CAS + npmrc succeeds with zero network calls!
    let count_before = transport.log.lock().unwrap().len();
    let (offline_graph, offline_modified) = resolve_requests_with_lockfile_and_npmrc(
        &reqs,
        &mut lockfile,
        &cas,
        None::<&RemoteNpmRegistry>,
        DEFAULT_REGISTRY_ORIGIN,
        true,
        true,
        NpmLimits::default(),
        &TargetPlatform::current(),
        Some(&npmrc),
        false,
    )
    .unwrap();
    assert!(!offline_modified);
    assert_eq!(transport.log.lock().unwrap().len(), count_before);
    assert!(offline_graph.resolve_root_specifier(&priv_spec).is_ok());

    // Requesting @unset/pkg (whose ${MISSING_TOKEN} is unset) fails closed with MissingAuthEnvVar.
    let unset_err = remote_reg
        .fetch_packument(&PackageName::parse("@unset/pkg").unwrap())
        .unwrap_err();
    assert!(
        matches!(
            unset_err,
            ResolverError::Npmrc(NpmrcError::MissingAuthEnvVar { ref var_name })
                if var_name == "MISSING_TOKEN"
        ),
        "unexpected error: {unset_err}"
    );

    // Plaintext http:// tarball URL matching an auth rule fails closed with InsecureAuthUrl.
    let http_err = remote_reg
        .fetch_tarball("http://priv.example.test/team/pkg.tgz", "sha512-x")
        .unwrap_err();
    assert!(
        matches!(http_err, ResolverError::Npmrc(NpmrcError::InsecureAuthUrl)),
        "unexpected error: {http_err}"
    );
}
