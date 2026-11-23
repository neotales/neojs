# NeoJS documentation site

Astro 7 + Starlight 0.43 documentation site for NeoJS, with Tailwind CSS 4 utilities and Starlight's
built-in Pagefind search.

The site is self-contained under `site/`. It does not depend on the Rust workspace, and building it
does not change the `neojs` runtime, the CLI, or any crate.

## Requirements

- Node.js `>=22.23.4`
- pnpm `12.18.0` (recorded in `package.json`'s `packageManager` field)

## Build

```sh
cd site
pnpm install --frozen-lockfile
pnpm check
pnpm build
pnpm test
```

`pnpm verify` runs `check`, `build`, and `test` in order.

- `pnpm dev` starts the development server on port `4421`.
- `pnpm build` writes the static site, including the Pagefind search index, to `dist/`.
- `pnpm test` validates the generated site with `node --test scripts/site.test.mjs`.

## Layout

```text
site/
  astro.config.mjs          Starlight config, sidebar, Tailwind Vite plugin
  src/content.config.ts     Starlight docs collection
  src/content/docs/         Markdown and MDX pages
  src/styles/global.css     Tailwind theme/utilities and Starlight accent
  scripts/site.test.mjs     Build output checks: search index, pages, links
  public/                   Static assets
```

## Dependency policy

Dependencies are pinned to exact versions in `package.json`, and `pnpm-lock.yaml` is committed.
Install with `--frozen-lockfile`. No build output or dependency tree is committed. The site has no
Svelte dependency and no Svelte-based documentation components.

## Content source

The pages summarise the repository `README.md`, `docs/`, crate READMEs, and the module sources.
Keep claims scoped to what the source implements, and keep the V8/Lite distinction explicit.

Use `neo:` import specifiers for Neo built-ins in every example. Bare built-in names are reserved
for future Node/package compatibility; `pnpm test` fails if a documented built-in import omits the
prefix.
