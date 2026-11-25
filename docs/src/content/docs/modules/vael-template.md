---
title: vael:template
description: Bounded Go text/template and Sprig rendering engine with HTML auto-escaping, safeHtml, hermetic mode, and template sets.
---

`vael:template` renders Go `text/template` syntax with Sprig helpers (`neotales-gotmpl` + `neotales-sprig`), optional HTML auto-escaping, hermetic mode, and strict resource limits.

:::note[Availability]
`vael:template` is available in **both `vjs` (V8) and `vjs-lite` (QuickJS)** when built with the `vael-template` Cargo feature (enabled by default in both binaries).
:::

## Exports

The module also provides a `default` export mirroring the four named exports.

| Export | Signature | Description |
| --- | --- | --- |
| `render` | `(source: string, data?: unknown, options?: TemplateOptions) => string` | Compile and render a template once against optional JSON data. |
| `compile` | `(source: string, options?: TemplateOptions) => Template` | Pre-compile a template for repeated `.render(data?)` calls. |
| `createTemplateSet` | `(options?: TemplateOptions) => TemplateSet` | Create a named template collection supporting `define`, `template`, and `block`. |
| `escapeHtml` | `(input: string) => string` | Escape `&`, `<`, `>`, `"`, and `'` in `input`. |

### `Template` & `TemplateSet`

- **`Template.render(data?: unknown): string`**: Render the compiled template against an optional JSON object (defaults to `{}`).
- **`TemplateSet.add(name: string, source: string): TemplateSet`**: Validate and register a named template, replacing any prior entry with the same name. Returns `this` for chaining.
- **`TemplateSet.render(name: string, data?: unknown): string`**: Render a registered template by name.
- **`TemplateSet.names(): string[]`**: Return registered template names in sorted order.

## HTML Auto-Escaping & `safeHtml`

When `htmlEscape: true` is passed in `TemplateOptions`, all interpolated values are automatically HTML-escaped unless explicitly marked inside the template with the `safeHtml` helper (for example `{{ .trusted | safeHtml }}`).

Because `safeHtml` attaches an internal trusted marker during template execution, untrusted JSON input cannot forge `safeHtml` status. `escapeHtml` and `htmlEscape` are also registered as template pipeline helpers regardless of the `htmlEscape` option.

## `TemplateOptions` & Resource Limits

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `sprig` | `boolean` | `true` | Enable deterministic Sprig template functions. Set `false` to disable Sprig helpers. |
| `hermetic` | `boolean` | `false` | Omit clock- and environment-reading Sprig helpers (such as `now` and `ago`). |
| `htmlEscape` | `boolean` | `false` | Auto-escape interpolated values unless marked with `safeHtml`. |
| `maxSourceLength` | `number` | `1_048_576` | Maximum template source length in UTF-8 bytes (`1 MiB`). |
| `maxNestingDepth` | `number` | `100` | Maximum control-structure nesting depth. |
| `maxOutputLength` | `number` | `4_194_304` | Maximum rendered output length in UTF-8 bytes (`4 MiB`). |
| `maxIterations` | `number` | `100_000` | Maximum `range` loop iterations per render. |
| `maxFunctionCalls` | `number` | `100_000` | Maximum helper function calls per render. |

## Error Codes (`TemplateErrorCode`)

Failures throw an `Error` with `code` set to one of:

| Code | Meaning |
| --- | --- |
| `"PARSE"` | Invalid Go template syntax. |
| `"RENDER"` | Runtime failure during template evaluation. |
| `"LIMIT_EXCEEDED"` | Exceeded source length, nesting depth, output length, loop iterations, or function calls. |
| `"TEMPLATE_NOT_FOUND"` | `TemplateSet.render(name)` referenced an unregistered template name. |
| `"INVALID_NAME"` | Blank or invalid template name passed to `TemplateSet.add()` or `render()`. |
| `"INVALID_DATA"` | Invalid options, non-string input to `escapeHtml()`, or non-object data context. |

## Example

```ts
import { createTemplateSet, escapeHtml, render } from "vael:template";

// Auto-escaped one-shot render with Sprig helper + safeHtml
const page = render(
  `<h1>{{ .title | upper }}</h1><div>{{ .body | safeHtml }}</div>`,
  { title: "<Admin>", body: "<em>Ready</em>" },
  { htmlEscape: true, hermetic: true },
);
console.log(page); // <h1>&lt;ADMIN&gt;</h1><div><em>Ready</em></div>

// Composable layout + partial set
const templates = createTemplateSet({ htmlEscape: true })
  .add("layout", `<html><body>{{ block "content" . }}{{ end }}</body></html>`)
  .add("home", `{{ template "layout" . }}{{ define "content" }}<p>{{ .msg }}</p>{{ end }}`);

console.log(templates.render("home", { msg: "Hello <world>" }));
console.log(escapeHtml(`"a" & 'b'`));
```
