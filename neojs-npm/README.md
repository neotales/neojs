# neojs-npm

Engine-independent `npm:` package resolution, verification, and virtual graph core for `neojs`.

`neojs-npm` has zero JavaScript engine dependencies (`deno_core`, `deno_ast`, `v8`, and `rquickjs` are absent) and is **offline by default** (`default = []`). Enabling the optional `network` feature activates `neojs-remote/network` (`RemoteNpmRegistry::new`).

```sh
cargo test --locked -p neojs-npm
cargo test --locked -p neojs-npm --all-features
cargo tree --locked -p neojs-npm -e normal
```

## Architecture invariants

- **Zero `node_modules`:** Packages never materialize onto the workspace or host filesystem by package name. Resolved modules receive peer-context-isolated virtual URLs of the form `npm://<registry_host>/<pkg>/<version>/<ctx>/<rel_path>`.
- **Content-verified CAS execution:** Extracted files live in `cas/sha256/<aa>/<bb>/<digest>` (`0444` on Unix), indexed by canonical `.tree` manifests (`npm/<origin>/trees/sha512-<hex>.tree`). Every read opens with `O_NOFOLLOW` on Unix, verifies `fstat` regular-file metadata and exact byte length, verifies SHA-256 in memory, and returns owned `Vec<u8>`.
- **Zero auto-install scripts & native addons:** Lifecycle hooks (`preinstall`, `install`, `postinstall`, `prepare`, etc.) are audited into `ScriptAudit` and never executed. Native binaries (`.node`, `.dll`, `.so`, `.dylib`) are excluded from CAS.
- **Strict ESM only:** `exports` and `#imports` preserve JSON manifest key insertion order with active conditions fixed to `["import", "default"]`. Every module file is classified via `neojs_module_policy::package_module_format` using the nearest `package.json` `"type"` inside `.tree`.
- **Project `package.json` bare imports:** `ProjectPackageResolver` maps declared `dependencies`, `optionalDependencies`, `#imports`, and package self-reference into `npm:` graph roots or local project files without `node_modules` and without intercepting `neo:` or `node:` built-ins.

## Modules

- `specifier`: `PackageName`, `NpmSpecifier`, `split_bare_package_specifier`, and virtual `npm://` URL helpers.
- `semver` (also aliased `npm_semver`): bounded `node-semver`-compatible `NpmVersion`, `NpmRange`, and `VersionConstraint`.
- `package_json`: bounded `PackageJson` parser (`MAX_PACKAGE_JSON_BYTES = 1 MiB`), ordered `ExportsField`, `ScriptAudit`, and `TargetPlatform`.
- `exports`: `ExportsResolver` for `exports`, `#imports`, self-reference, restricted `main` fallback, and nearest-`package.json` ESM classification.
- `cas` & `tarball`: `CasStore`, `TarballIngestor` (SRI `sha512` check + bounded POSIX/ustar/PAX tar reader directly over `flate2::read::GzDecoder`), and `PackageTree`.
- `npmrc`: bounded `.npmrc` parser (`MAX_NPMRC_BYTES = 256 KiB`), `@scope:registry` routing, HTTPS-only `_authToken`/`_auth` matching with lazy `${ENV_VAR}` resolution, redaction, and `ZeroizeOnDrop`.
- `resolver`: `NpmRegistry` trait, `RemoteNpmRegistry`, `NpmGraphResolver`, `ResolvedNpmGraph`, and `ProjectPackageResolver`.
