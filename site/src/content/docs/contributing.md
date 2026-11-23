---
title: Building and contributing
description: Build the docs site reproducibly, verify its search index and internal links, and update it from source.
---

This documentation site is a standalone Astro + Starlight frontend under `site/`. It has no
dependency on the Rust workspace and does not change product behaviour.

## Requirements

- Node.js 22.23.4 or newer (the version Astro requires).
- pnpm 12.18.0, the version recorded in `packageManager`.

Dependencies are pinned exactly in `package.json`, and `pnpm-lock.yaml` is committed. Do not add a
range specifier or a vendored build artifact.

## Install and verify

```sh
cd site
pnpm install --frozen-lockfile
pnpm check
pnpm build
pnpm test
```

`pnpm verify` runs the three gates in order:

```sh
pnpm verify
```

- `pnpm check` runs `astro check` over the Astro, TypeScript, and content files.
- `pnpm build` produces `dist/`.
- `pnpm test` runs `node --test scripts/site.test.mjs`, which checks that the Pagefind search index
  was generated, that every expected page exists, and that internal links resolve inside `dist/`.

Do not report a gate as passing without running it. A build that succeeds while the site has a dead
navigation link is not verified.

## Development server

```sh
pnpm dev
```

The server uses port `4421`. Pagefind builds the search index during `pnpm build`, so a production
build is the authoritative check for search output, not the development server.

## Where the content comes from

| Topic | Source of truth |
| --- | --- |
| CLI, built-ins, limits, remote loading | repository `README.md` |
| V8 embedding | `neojs-core/src`, `neojs/src` |
| NeoJS Lite | `neojs-lite-core/README.md`, `neojs-lite/README.md` |
| Database API | `neojs-dbapi/README.md`, `docs/db-api.md` |
| Module exports | `neojs-core/src/builtins/*.esm.js` |
| Module semantics and errors | `neojs-core/src/builtins/neo.js` and the Rust modules |
| License and notices | `LICENSE.md`, `neojs-core/THIRD_PARTY_NOTICES.md` |

The site deliberately summarises rather than duplicating the whole README. When behaviour changes,
update the page that states it and keep the "not implemented" claims honest. The pages were checked
against `dev` at commit `420402e` on 2026-11-22.

## Content rules

- State the runtime: **V8 only**, **Lite only**, or **both**. Most built-ins are V8 only.
- Distinguish **implemented**, **opt-in**, and **not implemented**. Do not describe a planned
  feature as available.
- Use error codes and limits from the source, not from memory.
- Mark illustrative code as illustrative. Do not present an example as verified unless it was run.
- Keep the V8/Lite comparison in [V8 and Lite compared](/v8-vs-lite/) in step with
  [Limits and unsupported features](/limits/).

## Search and navigation

Starlight generates the search index with Pagefind at build time. The sidebar is defined in
`astro.config.mjs`; a page that is not in the sidebar is still built, but the navigation check
should be updated when a page is added.

`scripts/site.test.mjs` reads the generated `dist/` output. It fails when:

- `dist/pagefind/pagefind.js` or a Pagefind index is missing.
- An expected page is missing.
- An internal `href` points at a path with no generated page or asset.

## Repository boundaries

- Do not edit Rust product behaviour from this site.
- Do not commit `node_modules/`, `dist/`, `.astro/`, or package-manager caches.
- Do not vendor dependencies or generated search assets into the source tree.
- Keep changes on the site branch and commit them in small pieces.
