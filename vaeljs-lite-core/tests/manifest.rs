//! End-to-end tests for `vael.jsonc` `ImportResolver` integration in `vaeljs-lite-core`.

#![cfg(feature = "manifest")]

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use vaeljs_lite_core::{ImportResolver, Runtime};
use vaeljs_manifest::{DiscoveredManifest, Manifest, PackageName, Workspace};

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
        r#"{ "name": "@lite/app", "imports": { "greet": "./greet.js" } }"#,
    );
    write(
        dir.path(),
        "greet.js",
        "export const hello = (n) => `hi ${n}`;\n",
    );
    let main = write(
        dir.path(),
        "main.js",
        "import { hello } from 'greet';\nconsole.log(hello('world'));\n",
    );

    let err = Runtime::new()
        .expect("runtime")
        .run_file(&main)
        .expect_err("Runtime::new must not discover vael.jsonc implicitly");
    assert!(err.to_string().contains("bare module specifiers"), "{err}");
}

#[test]
fn resolves_exact_and_prefix_local_imports_and_workspace_packages() {
    let dir = TempDir::new().expect("temp dir");
    let root_manifest = write(
        dir.path(),
        "vael.jsonc",
        r#"{
            "name": "@lite/root",
            "workspaces": ["packages/*"],
            "imports": {
                "greet": "./shared/greet.js",
                "lib/": "./shared/lib/"
            }
        }"#,
    );
    write(
        dir.path(),
        "shared/greet.js",
        "export const greet = (name) => `hello ${name}`;\n",
    );
    write(
        dir.path(),
        "shared/lib/math.mjs",
        "export const add = (a, b) => a + b;\n",
    );
    write(
        dir.path(),
        "packages/a/vael.jsonc",
        r#"{
            "name": "@lite/a",
            "exports": "./mod.js",
            "imports": { "local-a": "./local.js" }
        }"#,
    );
    write(
        dir.path(),
        "packages/a/local.js",
        "export const fromA = 'member-a';\n",
    );
    write(
        dir.path(),
        "packages/b/vael.jsonc",
        r#"{
            "name": "@lite/b",
            "exports": {
                ".": "./index.js",
                "./sub": "./sub.mjs"
            }
        }"#,
    );
    write(
        dir.path(),
        "packages/b/index.js",
        "export const bName = 'pkg-b';\n",
    );
    write(
        dir.path(),
        "packages/b/sub.mjs",
        "export const subVal = 77;\n",
    );
    write(
        dir.path(),
        "packages/b/secret.js",
        "export const secret = 'hidden';\n",
    );

    let entry = write(
        dir.path(),
        "packages/a/mod.js",
        r#"
import { greet } from "greet";
import { add } from "lib/math.mjs";
import { fromA } from "local-a";
import { bName } from "@lite/b";
import { subVal } from "@lite/b/sub";
if (greet("lite") !== "hello lite" || add(3, 4) !== 7 || fromA !== "member-a" || bName !== "pkg-b" || subVal !== 77) {
    throw new Error("lite manifest resolution mismatch");
}
"#,
    );

    let workspace = Workspace::load(&root_manifest).expect("workspace");
    let active = PackageName::parse("@lite/a").expect("pkg name");
    let resolver = ImportResolver::from_workspace(&workspace, Some(&active)).expect("resolver");

    Runtime::with_import_resolver(resolver.clone())
        .expect("runtime")
        .run_file(&entry)
        .expect("workspace imports succeed");

    // Unexported subpath must be rejected.
    let bad_entry = write(
        dir.path(),
        "packages/a/bad.js",
        r#"import { secret } from "@lite/b/secret"; console.log(secret);"#,
    );
    let err = Runtime::with_import_resolver(resolver)
        .expect("runtime")
        .run_file(&bad_entry)
        .expect_err("unexported subpath must fail");
    assert!(err.to_string().contains("not exported"), "{err}");
}

#[test]
fn resolves_manifest_targets_rejects_remote_and_preserves_builtins() {
    let dir = TempDir::new().expect("temp dir");
    let manifest_path = write(
        dir.path(),
        "vael.jsonc",
        r#"{
            "name": "@lite/policy",
            "imports": {
                "vael:os": "./fake-os.js",
                "node:fs": "./fake-fs.js",
                "https-dep": "https://example.test/mod.js",
                "gh-dep": "gh:owner/repo@v1/mod.js",
                "jsr-dep": "jsr:@scope/pkg@1.0.0",
                "npm-dep": "npm:lodash@4.17.21",
                "ts-dep": "./typed.ts"
            }
        }"#,
    );
    write(
        dir.path(),
        "fake-os.js",
        "export const platform = 'fake';\n",
    );
    write(dir.path(), "fake-fs.js", "export const x = 1;\n");
    write(dir.path(), "typed.ts", "export const x: number = 1;\n");
    let resolver = load_standalone_resolver(&manifest_path);

    // Built-in `vael:os` is never intercepted.
    let os_entry = write(
        dir.path(),
        "os.js",
        "import { platform } from 'vael:os';\nif (platform === 'fake') throw new Error('shadowed');\n",
    );
    Runtime::with_import_resolver(resolver.clone())
        .expect("runtime")
        .run_file(&os_entry)
        .expect("vael:os succeeds");

    // `node:fs` must use the built-in (never `./fake-fs.js`), while unsupported
    // `node:fs/promises` and disabled `node-compat` stay rejected.
    #[cfg(feature = "node-compat")]
    {
        let node_entry = write(
            dir.path(),
            "node.js",
            "import { existsSync } from 'node:fs';\nif (typeof existsSync !== 'function') throw new Error('shadowed');\n",
        );
        Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&node_entry)
            .expect("node:fs succeeds and is not shadowed");

        let node_disabled_err = Runtime::with_options(
            vaeljs_lite_core::RuntimeOptions::new()
                .with_import_resolver(resolver.clone())
                .with_node_compat(false),
        )
        .expect("runtime")
        .run_file(&node_entry)
        .expect_err("node:fs with node_compat disabled must fail");
        assert!(
            node_disabled_err.to_string().contains("node:"),
            "{node_disabled_err}"
        );
    }

    #[cfg(not(feature = "node-compat"))]
    {
        let node_entry = write(dir.path(), "node.js", "import 'node:fs';\n");
        let node_err = Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&node_entry)
            .expect_err("node:fs without node-compat feature must fail");
        assert!(node_err.to_string().contains("node:"), "{node_err}");
    }

    let unsupported_entry = write(
        dir.path(),
        "node_promises.js",
        "import 'node:fs/promises';\n",
    );
    let node_err = Runtime::with_import_resolver(resolver.clone())
        .expect("runtime")
        .run_file(&unsupported_entry)
        .expect_err("node:fs/promises must fail on lite");
    assert!(node_err.to_string().contains("node:"), "{node_err}");

    // Remote `https:`, `gh:`, `jsr:`, `npm:` targets fail with clear error stating remote loading is not supported.
    for (file, spec) in [
        ("https.js", "https-dep"),
        ("gh.js", "gh-dep"),
        ("jsr.js", "jsr-dep"),
        ("npm.js", "npm-dep"),
    ] {
        let entry = write(dir.path(), file, &format!("import '{spec}';\n"));
        let err = Runtime::with_import_resolver(resolver.clone())
            .expect("runtime")
            .run_file(&entry)
            .expect_err("remote target must be rejected in vaeljs-lite");
        let msg = err.to_string();
        assert!(
            msg.contains("remote module loading is not supported in vaeljs-lite"),
            "unexpected error for {spec}: {msg}"
        );
    }

    // TypeScript target runs when `typescript` feature is enabled, and fails clearly when disabled.
    let ts_entry = write(
        dir.path(),
        "ts.js",
        "import { x } from 'ts-dep';\nif (x !== 1) throw new Error('bad ts-dep');\n",
    );
    #[cfg(feature = "typescript")]
    Runtime::with_import_resolver(resolver)
        .expect("runtime")
        .run_file(&ts_entry)
        .expect(".ts target succeeds when typescript feature is enabled");
    #[cfg(not(feature = "typescript"))]
    {
        let ts_err = Runtime::with_import_resolver(resolver)
            .expect("runtime")
            .run_file(&ts_entry)
            .expect_err(".ts target must fail when typescript feature is disabled");
        assert!(
            ts_err
                .to_string()
                .contains("TypeScript is not enabled in this build"),
            "{ts_err}"
        );
    }
}
