# neojs-template-core

Engine-neutral Go `text/template` and deterministic Sprig core shared by the V8 (`neojs-core`) and QuickJS (`neojs-lite-core`) runtimes.

This crate has `#![forbid(unsafe_code)]`, zero JavaScript engine dependencies (`v8`, `deno_core`, `rquickjs`), and zero `tokio` dependencies. It wraps [`neotales-gotmpl`](https://crates.io/crates/neotales-gotmpl) and [`neotales-sprig`](https://crates.io/crates/neotales-sprig).

## Features

- **Go template syntax**: dot paths, `if`/`else`, `range`, `with`, pipelines, variables, `define`/`template`/`block`, `len`, and `index`.
- **Safe Sprig helpers**: full deterministic `neotales-sprig` helper set (`sprig: true`), or `hermetic: true` to omit clock- and environment-reading helpers (`now`, `ago`, `env`, `expandenv`, etc.).
- **XSS-safe HTML rendering**: `escape_html`, always-available `escapeHtml`/`htmlEscape`/`safeHtml` template helpers, and `html_escape: true` auto-escaping mode.
- **Layouts & partials**: `TemplateSet` composes `{{define}}`, `{{template}}`, and `{{block}}` across registered templates.
- **Bounded execution**: finite defaults for source length, nesting depth, output length, loop iterations, and function calls.
