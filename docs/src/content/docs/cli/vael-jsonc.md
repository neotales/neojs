---
title: vael.jsonc & Workspaces
description: Manifest schema, strict JSONC dialect, bounded discovery, workspace resolution, and vael.lock v1 format.
---

VaelJS projects use **`vael.jsonc`** for package metadata, `imports` maps, and monorepo `workspaces`. The schema is intentionally strict: unknown keys, duplicate keys, and explicit `null` fields are rejected rather than ignored.

## Example `vael.jsonc`

```jsonc
{
  // Required: scoped package name (@scope/name)
  "name": "@neotales/automation",
  "version": "0.1.0",
  "description": "Shared operational scripts and libraries.",
  "authors": ["Ada", "Grace"],

  // Exported entrypoints for workspace consumers (no "main" fallback)
  "exports": {
    ".": "./src/mod.ts",
    "./ssh": "./src/ssh.ts",
  },

  // Specifier map applied to local file: modules
  "imports": {
    "@lib/": "./src/lib/",
    "std/assert": "jsr:@std/assert@1.0.0",
  },

  // Expand workspace member directories deterministically
  "workspaces": ["packages/*"],
}
```

## Schema Fields

| Field | Required | Rules & Limits |
| --- | --- | --- |
| `name` | **Yes** | `@scope/name` format, max 214 bytes, lowercase ASCII letters, digits, `-`, `_`, `.`. |
| `version` | No | Exact SemVer string (e.g., `1.2.3-alpha.1`). Ranges and tags are rejected. |
| `description` | No | String up to 4,096 bytes. |
| `authors` | No | Array of up to 64 non-empty strings (max 256 bytes each, trimmed). |
| `exports` | No | String or object mapping `.` / `./subpath` to `./...` (`.ts`, `.mts`, `.js`, `.mjs`), max 1,024 entries. `"main"` is rejected. |
| `imports` | No | Object mapping exact keys or trailing-`/` prefixes to local paths or canonical schemes, max 1,024 entries. Bare targets are rejected. |
| `workspaces` | No | Array of member directory patterns expanded deterministically by `Workspace::load`. |
| `include` / `exclude` | No | Arrays of up to 256 relative glob strings (syntax-checked only; not applied at runtime). |

### JSONC Dialect Bounds

- Supports `//` line comments, `/* ... */` block comments (non-nesting), one trailing comma per object/array, and an optional UTF-8 BOM.
- Maximum file size: **1 MiB** (`1,048,576` bytes); maximum nesting depth: **128**.
- Duplicate keys are rejected at every object depth.

## Manifest & Workspace Discovery

When running a local file, `vjs` and `vjs-lite` search upward from the entry file's directory for `vael.jsonc` (bounded to **`MAX_DISCOVERY_DEPTH = 64`** parent directories):

1. If a `vael.jsonc` is found and its directory sits inside an enclosing workspace root (`workspaces` non-empty), the workspace resolver loads all members and resolves `@scope/pkg` imports across workspace packages.
2. Pass `--config <path>` to load an explicit manifest (exits `2` if missing or invalid), or `--no-config` to skip discovery.
3. In Rust embedding (`Runtime::new()`), automatic discovery is **off**—opt in explicitly with `ImportResolver` (`vaeljs-manifest`).

### Resolution Boundaries

- Only local `file:` modules consult `vael.jsonc` `imports` and `workspaces`. Remote (`https:`, `jsr:`) referrers cannot use a local manifest to access local files.
- `vael:*` and `node:*` specifiers are never intercepted by `imports` maps.
- In `vjs`, remote targets inside `imports` (`https:`, `gh:`, `jsr:`) still require `--allow-remote` or `VAELJS_ALLOW_REMOTE=1`. In `vjs-lite`, remote and `npm:` targets are rejected.

## `vael.lock` v1

When `vjs` resolves `npm:` specifiers or `package.json` dependencies, it pins exact versions and integrity digests in **`vael.lock`** (`version: 1`, max **16 MiB**, canonical sorted JSON):

- Maps each requested specifier (e.g., `"pkg@^1.0.0"`) to its resolved package key (`"pkg@1.2.3"`).
- Records per-package `version`, `integrity` (`sha512-...` SRI or SHA-256), optional `treeSha256`, `tarballUrl`, and resolved `dependencies`.
- Use `--frozen-lockfile` in CI to fail if any dependency is missing from `vael.lock` or fails verification.

:::note[Design Specifications]
For deeper implementation notes, see the repository architecture specs: [`docs/vael-jsonc.md`](https://github.com/neotales/vaeljs/blob/dev/docs/vael-jsonc.md).
:::
