//! Integration tests for bounded manifest discovery, workspace expansion,
//! and import-map / workspace resolution.

use std::fs;
use std::path::Path;

use neojs_manifest::{
    Error, ImportResolver, MANIFEST_FILE_NAME, MAX_DISCOVERY_DEPTH, MAX_MANIFEST_BYTES,
    MAX_WORKSPACE_MEMBERS, Manifest, PackageName, ResolvedImport, SpecifierKind, Support,
    Workspace, discover_manifest,
};

fn write_manifest(dir: &Path, body: &str) -> std::path::PathBuf {
    fs::create_dir_all(dir).expect("create manifest dir");
    let path = dir.join(MANIFEST_FILE_NAME);
    fs::write(&path, body).expect("write manifest");
    path
}

#[test]
fn discovers_manifest_in_current_and_ancestor_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("project");
    let nested = root.join("src").join("deep").join("mod");
    fs::create_dir_all(&nested).expect("create nested");
    let manifest_path = write_manifest(&root, r#"{"name":"@acme/app","exports":"./mod.ts"}"#);

    let found_from_root = discover_manifest(&root, None)
        .expect("discover from root")
        .expect("found");
    assert_eq!(
        found_from_root.path,
        fs::canonicalize(&manifest_path).expect("canonical manifest")
    );
    assert_eq!(
        found_from_root.dir,
        fs::canonicalize(&root).expect("canonical root")
    );
    assert_eq!(found_from_root.manifest.name.as_str(), "@acme/app");

    let found_from_nested = discover_manifest(&nested, None)
        .expect("discover from nested")
        .expect("found");
    assert_eq!(found_from_nested, found_from_root);
}

#[test]
fn discovery_respects_stop_dir_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outer = temp.path().join("outer");
    let repo = outer.join("repo");
    let sub = repo.join("packages").join("app");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&sub).expect("create sub");
    fs::create_dir_all(&outside).expect("create outside");

    // Only outer has a manifest; stopping at `repo` must not see `outer`.
    write_manifest(&outer, r#"{"name":"@outer/root"}"#);
    assert_eq!(
        discover_manifest(&sub, Some(&repo)).expect("bounded search"),
        None
    );

    // If `repo` itself has a manifest, `stop_dir` is inclusive.
    let repo_manifest = write_manifest(&repo, r#"{"name":"@repo/root"}"#);
    let found = discover_manifest(&sub, Some(&repo))
        .expect("bounded search finds stop_dir")
        .expect("found at stop_dir");
    assert_eq!(
        found.path,
        fs::canonicalize(repo_manifest).expect("canonical")
    );

    // Starting outside `stop_dir` returns None without walking upward.
    assert_eq!(
        discover_manifest(&outside, Some(&repo)).expect("outside stop_dir"),
        None
    );
}

#[test]
fn discovery_enforces_max_depth() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("root");
    write_manifest(&root, r#"{"name":"@acme/deep"}"#);

    // Build 64 nested directories beneath `root` so `root` is 64 parents up
    // (65th directory inspected, outside MAX_DISCOVERY_DEPTH = 64).
    let mut deep = root.clone();
    for index in 0..MAX_DISCOVERY_DEPTH {
        deep = deep.join(format!("d{index}"));
    }
    fs::create_dir_all(&deep).expect("create deep dirs");

    assert_eq!(
        discover_manifest(&deep, None).expect("depth limit returns None"),
        None
    );

    // One directory shallower (64th directory inspected = `root`) succeeds.
    let within_limit = deep.parent().expect("parent");
    let found = discover_manifest(within_limit, None)
        .expect("within depth limit")
        .expect("found at 64th inspected dir");
    assert_eq!(found.manifest.name.as_str(), "@acme/deep");
}

#[test]
fn discovery_fails_closed_on_malformed_oversized_or_unreadable_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let parent = temp.path().join("parent");
    let child = parent.join("child");
    write_manifest(&parent, r#"{"name":"@acme/parent"}"#);

    // Malformed manifest in child must fail rather than falling back to parent.
    let child_manifest = write_manifest(&child, "{ not jsonc");
    let err = discover_manifest(&child, None).expect_err("malformed must fail closed");
    assert!(matches!(err, Error::Jsonc { .. }), "{err}");

    // Oversized manifest in child must fail closed.
    let mut oversized = String::from(r#"{"name":"@acme/child"}"#);
    oversized.push_str(&" ".repeat(MAX_MANIFEST_BYTES + 1 - oversized.len()));
    fs::write(&child_manifest, oversized).expect("write oversized");
    let err = discover_manifest(&child, None).expect_err("oversized must fail closed");
    assert!(matches!(err, Error::ManifestTooLarge { .. }), "{err}");

    // A directory occupying the manifest filename fails closed with Io.
    fs::remove_file(&child_manifest).expect("remove oversized");
    fs::create_dir(&child_manifest).expect("create dir at manifest path");
    let err = discover_manifest(&child, None).expect_err("directory candidate must fail closed");
    assert!(matches!(err, Error::Io { .. }), "{err}");
}

#[cfg(unix)]
#[test]
fn discovery_handles_symlink_loops_safely() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path().join("loop-test");
    fs::create_dir_all(&dir).expect("create dir");
    let self_link = dir.join("self");
    symlink(&dir, &self_link).expect("create self symlink");

    // Walking from a repeated self-symlink terminates cleanly with None.
    let repeated = self_link.join("self").join("self");
    assert_eq!(
        discover_manifest(&repeated, Some(temp.path())).expect("self symlink terminates"),
        None
    );

    // Mutual OS symlink loop fails closed with Error::Io without hanging.
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    symlink(&b, &a).expect("a -> b");
    symlink(&a, &b).expect("b -> a");
    let err = discover_manifest(&a, None).expect_err("mutual symlink loop fails closed");
    assert!(matches!(err, Error::Io { .. }), "{err}");

    // Dangling candidate manifest symlink fails closed rather than walking up.
    let parent = temp.path().join("parent");
    let child = parent.join("child");
    write_manifest(&parent, r#"{"name":"@acme/parent"}"#);
    fs::create_dir_all(&child).expect("create child");
    symlink(
        temp.path().join("does-not-exist"),
        child.join(MANIFEST_FILE_NAME),
    )
    .expect("dangling manifest symlink");
    let err = discover_manifest(&child, None).expect_err("dangling manifest fails closed");
    assert!(matches!(err, Error::Io { .. }), "{err}");
}

#[test]
fn workspace_expands_globs_deterministically_and_indexes_members() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Include glob metacharacters in the workspace directory name itself to
    // verify the root path is properly escaped.
    let root = temp.path().join("ws[1]");
    write_manifest(
        &root,
        r#"{
            "name": "@acme/root",
            "workspaces": ["packages/*", "tools/cli", "packages/beta"]
        }"#,
    );

    // Non-directory files matching `packages/*` are ignored.
    fs::create_dir_all(root.join("packages")).expect("create packages");
    fs::write(root.join("packages").join("README.md"), "docs").expect("write readme");

    write_manifest(
        &root.join("packages").join("zeta"),
        r#"{"name":"@acme/zeta","exports":"./mod.ts"}"#,
    );
    write_manifest(
        &root.join("packages").join("alpha"),
        r#"{"name":"@acme/alpha","exports":{".":"./index.ts","./sub":"./sub.ts"}}"#,
    );
    write_manifest(
        &root.join("packages").join("beta"),
        r#"{"name":"@acme/beta"}"#,
    );
    write_manifest(
        &root.join("tools").join("cli"),
        r#"{"name":"@acme/cli","exports":"./bin.ts"}"#,
    );

    let ws = Workspace::load(root.join(MANIFEST_FILE_NAME)).expect("load workspace");
    assert_eq!(ws.member_count(), 4);

    // Sorted deterministically by canonical directory path, deduplicating `beta`.
    let names: Vec<_> = ws.members().map(|m| m.name().as_str()).collect();
    assert_eq!(
        names,
        ["@acme/alpha", "@acme/beta", "@acme/zeta", "@acme/cli"]
    );

    let alpha_pkg = PackageName::parse("@acme/alpha").expect("pkg");
    let alpha = ws.member(&alpha_pkg).expect("alpha indexed");
    assert_eq!(
        alpha.dir,
        fs::canonicalize(root.join("packages").join("alpha")).expect("canonical alpha")
    );
    assert!(alpha.exports().is_some());

    // `member_for_path` resolves a file inside a member to that member.
    let alpha_src = root.join("packages").join("alpha").join("index.ts");
    fs::write(&alpha_src, "export const x = 1;\n").expect("write index.ts");
    let owner = ws
        .member_for_path(&alpha_src)
        .expect("member_for_path")
        .expect("owned by alpha");
    assert_eq!(owner.name(), &alpha_pkg);
}

#[test]
fn workspace_rejects_missing_duplicate_nested_and_over_limit_members() {
    let temp = tempfile::tempdir().expect("tempdir");

    // 1. Missing member manifest (explicit directory without neo.jsonc).
    let missing_ws = temp.path().join("missing-manifest");
    write_manifest(
        &missing_ws,
        r#"{"name":"@acme/root","workspaces":["packages/*"]}"#,
    );
    fs::create_dir_all(missing_ws.join("packages").join("empty")).expect("empty dir");
    let err = Workspace::load(missing_ws.join(MANIFEST_FILE_NAME)).expect_err("missing manifest");
    assert!(matches!(err, Error::MissingMemberManifest), "{err}");

    // 2. Missing explicit non-glob member directory.
    let missing_dir_ws = temp.path().join("missing-dir");
    write_manifest(
        &missing_dir_ws,
        r#"{"name":"@acme/root","workspaces":["packages/absent"]}"#,
    );
    let err =
        Workspace::load(missing_dir_ws.join(MANIFEST_FILE_NAME)).expect_err("missing member dir");
    assert!(matches!(err, Error::MissingMemberManifest), "{err}");

    // 3. Duplicate package name across root and member.
    let dup_root_ws = temp.path().join("dup-root");
    write_manifest(
        &dup_root_ws,
        r#"{"name":"@secret-scope/dup-name","workspaces":["packages/*"]}"#,
    );
    write_manifest(
        &dup_root_ws.join("packages").join("a"),
        r#"{"name":"@secret-scope/dup-name"}"#,
    );
    let err = Workspace::load(dup_root_ws.join(MANIFEST_FILE_NAME)).expect_err("dup root name");
    assert!(matches!(err, Error::DuplicateWorkspacePackage), "{err}");
    assert!(!err.to_string().contains("secret-scope"), "{err}");

    // 4. Duplicate package name across two members.
    let dup_members_ws = temp.path().join("dup-members");
    write_manifest(
        &dup_members_ws,
        r#"{"name":"@acme/root","workspaces":["packages/*"]}"#,
    );
    write_manifest(
        &dup_members_ws.join("packages").join("a"),
        r#"{"name":"@acme/shared"}"#,
    );
    write_manifest(
        &dup_members_ws.join("packages").join("b"),
        r#"{"name":"@acme/shared"}"#,
    );
    let err =
        Workspace::load(dup_members_ws.join(MANIFEST_FILE_NAME)).expect_err("dup member names");
    assert!(matches!(err, Error::DuplicateWorkspacePackage), "{err}");

    // 5. Nested workspace declaration rejected.
    let nested_ws = temp.path().join("nested");
    write_manifest(
        &nested_ws,
        r#"{"name":"@acme/root","workspaces":["packages/*"]}"#,
    );
    write_manifest(
        &nested_ws.join("packages").join("sub"),
        r#"{"name":"@acme/sub","workspaces":["inner/*"]}"#,
    );
    let err = Workspace::load(nested_ws.join(MANIFEST_FILE_NAME)).expect_err("nested workspace");
    assert!(matches!(err, Error::NestedWorkspace), "{err}");

    // 6. Member count over MAX_WORKSPACE_MEMBERS rejected.
    let huge_ws = temp.path().join("huge");
    write_manifest(
        &huge_ws,
        r#"{"name":"@acme/root","workspaces":["packages/*"]}"#,
    );
    for idx in 0..=MAX_WORKSPACE_MEMBERS {
        write_manifest(
            &huge_ws.join("packages").join(format!("p{idx:03}")),
            &format!(r#"{{"name":"@acme/p{idx:03}"}}"#),
        );
    }
    let err = Workspace::load(huge_ws.join(MANIFEST_FILE_NAME)).expect_err("too many members");
    assert!(
        matches!(err, Error::WorkspaceTooManyMembers { limit } if limit == MAX_WORKSPACE_MEMBERS),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn workspace_rejects_root_escapes_via_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside-secret-dir");
    write_manifest(&outside, r#"{"name":"@evil/outside"}"#);

    // Symlink member directory escaping root.
    let ws1 = temp.path().join("ws-escape-dir");
    write_manifest(&ws1, r#"{"name":"@acme/root","workspaces":["packages/*"]}"#);
    fs::create_dir_all(ws1.join("packages")).expect("create packages");
    symlink(&outside, ws1.join("packages").join("escaped")).expect("create escape symlink");
    let err = Workspace::load(ws1.join(MANIFEST_FILE_NAME)).expect_err("escaped member dir");
    assert!(matches!(err, Error::WorkspaceEscape), "{err}");
    assert!(!err.to_string().contains("outside-secret-dir"), "{err}");

    // Symlink member directory pointing back to the workspace root itself.
    let ws2 = temp.path().join("ws-self-link");
    write_manifest(&ws2, r#"{"name":"@acme/root","workspaces":["packages/*"]}"#);
    fs::create_dir_all(ws2.join("packages")).expect("create packages");
    symlink(&ws2, ws2.join("packages").join("self")).expect("create self link");
    let err = Workspace::load(ws2.join(MANIFEST_FILE_NAME)).expect_err("self-pointing member");
    assert!(matches!(err, Error::WorkspaceEscape), "{err}");

    // Symlinked member manifest file pointing outside the workspace root.
    let ws3 = temp.path().join("ws-escape-manifest");
    write_manifest(
        &ws3,
        r#"{"name":"@acme/root","workspaces":["packages/pkg"]}"#,
    );
    fs::create_dir_all(ws3.join("packages").join("pkg")).expect("create pkg");
    symlink(
        outside.join(MANIFEST_FILE_NAME),
        ws3.join("packages").join("pkg").join(MANIFEST_FILE_NAME),
    )
    .expect("create manifest escape symlink");
    let err = Workspace::load(ws3.join(MANIFEST_FILE_NAME)).expect_err("escaped manifest file");
    assert!(matches!(err, Error::WorkspaceEscape), "{err}");
}

#[test]
fn resolver_enforces_precedence_and_non_interception() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("ws");

    // Root manifest with exact, prefix, and deceptive builtin keys in imports.
    write_manifest(
        &root,
        r#"{
            "name": "@acme/root",
            "workspaces": ["packages/*"],
            "imports": {
                "neo:crypto": "./hijack-neo.ts",
                "node:fs/promises": "./hijack-node.ts",
                "neo:": "./hijack-neo-exact.ts",
                "node:": "./hijack-node-exact.ts",
                "neo:/": "./hijack-neo-prefix/",
                "node:/": "./hijack-node-prefix/",
                "@shared": "./shared/root-exact.ts",
                "@lib/": "./root-lib/",
                "@lib/deep/": "./root-deep/",
                "@acme/core": "./override-core-from-root.ts",
                "@web/": "https://example.com/root-web/",
                "@gh/": "gh:owner/repo@v1.2.3/src/",
                "@jsr/": "jsr:@scope/std@1.0.0/",
                "@npm/": "npm:lodash@4.17.21/",
                "gh-exact": "github:owner/repo@v2.0.0/mod.ts",
                "jsr-exact": "jsr:@scope/kit@0.1.0/entry",
                "npm-exact": "npm:@scope/tool@3.0.0/cli",
                "web-exact": "https://example.com/mod.ts"
            }
        }"#,
    );

    let app_dir = root.join("packages").join("app");
    write_manifest(
        &app_dir,
        r#"{
            "name": "@acme/app",
            "imports": {
                "@shared": "./member-exact.ts",
                "@lib/": "./member-lib/",
                "@local-only": "./src/local.ts"
            }
        }"#,
    );

    let core_dir = root.join("packages").join("core");
    write_manifest(
        &core_dir,
        r#"{
            "name": "@acme/core",
            "exports": {
                ".": "./src/mod.ts",
                "./utils": "./src/utils.ts"
            }
        }"#,
    );

    let helper_dir = root.join("packages").join("helper");
    write_manifest(
        &helper_dir,
        r#"{
            "name": "@acme/helper",
            "exports": "./index.ts"
        }"#,
    );

    let private_dir = root.join("packages").join("private");
    write_manifest(&private_dir, r#"{"name":"@acme/private"}"#);

    // Create some local target files on disk to verify canonical resolution.
    fs::create_dir_all(app_dir.join("src")).expect("create app/src");
    fs::write(app_dir.join("src").join("local.ts"), "export {};\n").expect("write local.ts");
    fs::create_dir_all(core_dir.join("src")).expect("create core/src");
    fs::write(core_dir.join("src").join("mod.ts"), "export {};\n").expect("write mod.ts");
    fs::write(core_dir.join("src").join("utils.ts"), "export {};\n").expect("write utils.ts");
    fs::write(helper_dir.join("index.ts"), "export {};\n").expect("write index.ts");

    let ws = Workspace::load(root.join(MANIFEST_FILE_NAME)).expect("load workspace");
    let canonical_root = ws.root_dir().to_path_buf();
    let app_pkg = PackageName::parse("@acme/app").expect("app");
    let canonical_app = ws.member(&app_pkg).expect("app").dir.clone();
    let canonical_core = ws
        .member(&PackageName::parse("@acme/core").expect("core"))
        .expect("core")
        .dir
        .clone();
    let canonical_helper = ws
        .member(&PackageName::parse("@acme/helper").expect("helper"))
        .expect("helper")
        .dir
        .clone();

    let resolver =
        ImportResolver::from_workspace(&ws, Some(&app_pkg)).expect("build member resolver");

    // 1. Built-ins are never intercepted, even when imports tries to shadow them.
    assert_eq!(
        resolver.resolve("neo:crypto").expect("neo builtin"),
        ResolvedImport::Builtin("neo:crypto".to_owned())
    );
    assert_eq!(
        resolver.resolve("neo:uuid").expect("neo prefix builtin"),
        ResolvedImport::Builtin("neo:uuid".to_owned())
    );
    assert_eq!(
        resolver.resolve("neo:/foo").expect("neo slash builtin"),
        ResolvedImport::Builtin("neo:/foo".to_owned())
    );
    assert_eq!(
        resolver
            .resolve("node:fs/promises")
            .expect("node exact builtin"),
        ResolvedImport::Builtin("node:fs/promises".to_owned())
    );
    assert_eq!(
        resolver.resolve("node:path").expect("node prefix builtin"),
        ResolvedImport::Builtin("node:path".to_owned())
    );
    assert_eq!(
        resolver.resolve("node:/foo").expect("node slash builtin"),
        ResolvedImport::Builtin("node:/foo".to_owned())
    );

    // 2. Exact match: member overrides root, and resolves relative to member dir.
    assert_eq!(
        resolver.resolve("@shared").expect("member overrides root"),
        ResolvedImport::Local(canonical_app.join("member-exact.ts"))
    );
    assert_eq!(
        resolver.resolve("@local-only").expect("member local"),
        ResolvedImport::Local(canonical_app.join("src").join("local.ts"))
    );

    // Exact match in root resolves relative to root (declaring manifest dir),
    // and wins over workspace package match (`@acme/core`).
    assert_eq!(
        resolver
            .resolve("@acme/core")
            .expect("root import shadows workspace pkg"),
        ResolvedImport::Local(canonical_root.join("override-core-from-root.ts"))
    );

    // 3. Longest prefix match: active member overrides root for `@lib/`.
    assert_eq!(
        resolver.resolve("@lib/a.ts").expect("member prefix"),
        ResolvedImport::Local(canonical_app.join("member-lib").join("a.ts"))
    );
    // Root resolver (no active member) sees root's longest prefix `@lib/deep/`.
    let root_resolver = ImportResolver::from_workspace(&ws, None).expect("root resolver");
    assert_eq!(
        root_resolver
            .resolve("@lib/deep/x.ts")
            .expect("longest root prefix"),
        ResolvedImport::Local(canonical_root.join("root-deep").join("x.ts"))
    );
    assert_eq!(
        root_resolver
            .resolve("@lib/other.ts")
            .expect("shorter root prefix"),
        ResolvedImport::Local(canonical_root.join("root-lib").join("other.ts"))
    );

    // Prefix traversal attempts must be rejected cleanly.
    for bad in [
        "@lib/",
        "@lib/../secret.ts",
        "@lib/sub/../../secret.ts",
        "@lib/./mod.ts",
        "@lib/a\\b.ts",
        "@web/../secret.ts",
        "@web/%2e%2e/secret.ts",
        "@web/sub/%2E%2E/secret.ts",
    ] {
        let err = resolver.resolve(bad).expect_err(bad);
        assert!(
            matches!(err, Error::InvalidSpecifier { .. }),
            "{bad}: {err}"
        );
    }

    // 4. Workspace package resolution (when not shadowed by `imports`).
    assert_eq!(
        resolver
            .resolve("@acme/core/utils")
            .expect("workspace subpath export"),
        ResolvedImport::Local(canonical_core.join("src").join("utils.ts"))
    );
    assert_eq!(
        resolver
            .resolve("@acme/helper")
            .expect("workspace single export"),
        ResolvedImport::Local(canonical_helper.join("index.ts"))
    );

    // Unexported workspace subpaths and no-exports packages are rejected clearly.
    for unexported in [
        "@acme/core/src/secret.ts",
        "@acme/core/../secret.ts",
        "@acme/core/",
        "@acme/helper/sub",
        "@acme/private",
        "@acme/private/mod",
    ] {
        let err = resolver.resolve(unexported).expect_err(unexported);
        assert!(
            matches!(err, Error::UnexportedSubpath),
            "{unexported}: {err}"
        );
    }

    // 5. Remote and registry targets preserve `NormalizedSpecifier` and `Support`.
    let gh = match resolver.resolve("gh-exact").expect("gh exact") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(gh.kind(), SpecifierKind::Gh);
    assert_eq!(gh.support(), Support::NormalizedOnly);
    assert_eq!(
        gh.canonical(),
        "https://raw.githubusercontent.com/owner/repo/v2.0.0/mod.ts"
    );

    let gh_pref = match resolver.resolve("@gh/sub/file.ts").expect("gh prefix") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(gh_pref.kind(), SpecifierKind::Gh);
    assert_eq!(gh_pref.support(), Support::NormalizedOnly);
    assert_eq!(
        gh_pref.canonical(),
        "https://raw.githubusercontent.com/owner/repo/v1.2.3/src/sub/file.ts"
    );

    let jsr = match resolver.resolve("jsr-exact").expect("jsr exact") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(jsr.kind(), SpecifierKind::Jsr);
    assert!(matches!(jsr.support(), Support::Unsupported(_)));
    assert_eq!(jsr.canonical(), "jsr:@scope/kit@0.1.0/entry");

    let jsr_pref = match resolver.resolve("@jsr/path/mod").expect("jsr prefix") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(jsr_pref.kind(), SpecifierKind::Jsr);
    assert!(matches!(jsr_pref.support(), Support::Unsupported(_)));
    assert_eq!(jsr_pref.canonical(), "jsr:@scope/std@1.0.0/path/mod");

    let npm = match resolver.resolve("npm-exact").expect("npm exact") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(npm.kind(), SpecifierKind::Npm);
    assert!(matches!(npm.support(), Support::Unsupported(_)));
    assert_eq!(npm.canonical(), "npm:@scope/tool@3.0.0/cli");

    let npm_pref = match resolver.resolve("@npm/debounce").expect("npm prefix") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(npm_pref.kind(), SpecifierKind::Npm);
    assert!(matches!(npm_pref.support(), Support::Unsupported(_)));
    assert_eq!(npm_pref.canonical(), "npm:lodash@4.17.21/debounce");

    let web = match resolver.resolve("web-exact").expect("web exact") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(web.kind(), SpecifierKind::Https);
    assert_eq!(web.support(), Support::NormalizedOnly);
    assert_eq!(web.canonical(), "https://example.com/mod.ts");

    let web_pref = match resolver.resolve("@web/util.ts?v=1").expect("web prefix") {
        ResolvedImport::Specifier(spec) => spec,
        other => panic!("expected Specifier, got {other:?}"),
    };
    assert_eq!(web_pref.kind(), SpecifierKind::Https);
    assert_eq!(web_pref.support(), Support::NormalizedOnly);
    assert_eq!(
        web_pref.canonical(),
        "https://example.com/root-web/util.ts?v=1"
    );

    // 6. Unmapped specifiers pass through as Unmapped.
    for unmapped in [
        "./relative.ts",
        "../parent.ts",
        "https://other.example/mod.ts",
        "@external/pkg",
        "bare-pkg",
    ] {
        assert_eq!(
            resolver.resolve(unmapped).expect(unmapped),
            ResolvedImport::Unmapped
        );
    }

    // 7. Unknown active member fails loudly.
    let unknown = PackageName::parse("@acme/unknown").expect("pkg");
    let err = ImportResolver::from_workspace(&ws, Some(&unknown)).expect_err("unknown member");
    assert!(matches!(err, Error::UnknownWorkspaceMember), "{err}");
}

#[test]
fn standalone_discovered_manifest_builds_resolver() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("pkg");
    let sub = root.join("src");
    fs::create_dir_all(&sub).expect("create src");
    fs::write(sub.join("util.ts"), "export const v = 1;\n").expect("write util.ts");

    write_manifest(
        &root,
        r#"{
            "name": "@solo/pkg",
            "imports": {
                "@/": "./src/",
                "std": "jsr:@scope/std@1.0.0"
            }
        }"#,
    );

    let discovered = discover_manifest(&sub, None)
        .expect("discover")
        .expect("found");
    let resolver = ImportResolver::from_manifest(&discovered);

    assert_eq!(
        resolver.resolve("@/util.ts").expect("prefix local"),
        ResolvedImport::Local(fs::canonicalize(sub.join("util.ts")).expect("canonical"))
    );
    assert_eq!(
        resolver.resolve("neo:assert").expect("builtin"),
        ResolvedImport::Builtin("neo:assert".to_owned())
    );
    assert!(matches!(
        resolver.resolve("std").expect("jsr"),
        ResolvedImport::Specifier(spec) if spec.kind() == SpecifierKind::Jsr
    ));
}

#[test]
fn prefix_import_keys_require_trailing_slash_targets() {
    let err = Manifest::parse(r#"{"name":"@a/b","imports":{"@lib/":"./src/lib"}}"#)
        .expect_err("prefix key without trailing slash target must fail");
    assert!(matches!(err, Error::InvalidSpecifier { .. }), "{err}");
}
