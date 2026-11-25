---
title: npm & Remote Loading
description: "Zero-node_modules npm package loading via CAS and vael.lock, .npmrc scoped registries, and opt-in jsr:, gh:, and https: imports in vjs."
---

`vjs` supports `npm:` packages, project `package.json` dependencies, and remote `jsr:`, `gh:`, and `https:` imports—without creating `node_modules` or running lifecycle scripts. `vjs-lite` rejects all remote and `npm:` imports.

## Zero-`node_modules` `npm:` Packages

Import npm ESM packages directly with `npm:` specifiers or through the nearest `package.json`:

```ts
import { z } from "npm:zod@^3.23.0";
import { helper } from "@my-org/internal-esm"; // via package.json dependencies or #imports
```

Key architectural guarantees in `vjs` (`vaeljs-npm`):

- **No `node_modules` directory:** Packages are extracted into a shared content-addressed store (CAS) under `$XDG_CACHE_HOME/vaeljs/packages` (or `$HOME/.cache/vaeljs/packages`).
- **Content verification:** Every tarball is checked against its `sha512` SRI digest, extracted files are stored read-only (`0444` on Unix) by SHA-256, and reads verify `O_NOFOLLOW`, file length, and SHA-256 in memory.
- **Zero install scripts or native addons:** `preinstall`, `install`, `postinstall`, and `prepare` scripts are never executed. Native binaries (`.node`, `.dll`, `.so`, `.dylib`) are excluded from CAS.
- **Strict ESM only:** `exports` and `#imports` resolve with active conditions `["import", "default"]`. CommonJS (`.cjs` or `"type": "commonjs"` `.js`) is rejected.
- **Offline by default:** Missing packages or uncached `vael.lock` entries require `--allow-remote` (or `VAELJS_ALLOW_REMOTE=1`) to fetch from the registry.

### Policy Flags for `npm:`

| Flag | Effect |
| --- | --- |
| `--allow-remote` | Allow fetching missing npm packuments/tarballs from HTTPS registries. |
| `--frozen-lockfile` | Refuse any resolution not pinned in `vael.lock` v1 or failing integrity checks. |
| `--offline` | Forbid all network requests; serve strictly from `vael.lock` and the verified CAS cache. |
| `--no-npm` | Disable `npm:` and `package.json` resolution completely for the run. |

## `.npmrc` Scoped Registries & Auth

`vjs` discovers `.npmrc` upward from the referrer or project directory (max **256 KiB**, opened with `O_NOFOLLOW` on Unix so symlinks fail closed):

```ini
registry=https://registry.npmjs.org
@internal:registry=https://npm.internal.example/
//npm.internal.example/:_authToken=${INTERNAL_NPM_TOKEN}
```

- **HTTPS-only:** Only `https:` registries are accepted for `registry=` and `@scope:registry=`.
- **Lazy `${ENV_VAR}` interpolation:** Tokens are read only from `[A-Za-z_][A-Za-z0-9_]*` environment variables when a request matches that rule (no shell evaluation).
- **Zero secret leakage:** Credentials are stored with `ZeroizeOnDrop`, print as `<redacted>` in errors/debug output, and are stripped on cross-origin or out-of-scope redirects.

## Remote `jsr:`, `gh:`, and `https:` Imports

Remote module loading is **disabled by default** in `vjs`. Enable it explicitly per invocation with `--allow-remote` or `VAELJS_ALLOW_REMOTE=1` (and override it off with `--no-allow-remote`):

```sh
vjs --allow-remote https://example.com/script.ts
vjs --allow-remote gh:owner/repo@v1.2.3/mod.ts
```

- **`jsr:@scope/name[@constraint][/subpath]`:** Resolves via the public JSR registry (`jsr.io`), checking every fetched file's length and SHA-256 against the version manifest before type-stripping and execution.
- **`gh:owner/repo@ref/path` / `github:owner/repo@ref/path`:** Normalizes to a credential-free `https://raw.githubusercontent.com/...` URL (the legacy `gh:owner/repo/path/@ref` form is also accepted). `ref` must be a single path segment (tag, branch without `/`, or commit SHA).
- **`https://...`:** Plain `http:` is always rejected. The HTTPS client sends no ambient credentials and reads no proxy environment variables.

:::caution[Not a Sandbox]
`--allow-remote` is an explicit network opt-in, not SSRF protection, a host allowlist, or a sandbox. Remote modules run with the host process's full filesystem and network authority.
:::
