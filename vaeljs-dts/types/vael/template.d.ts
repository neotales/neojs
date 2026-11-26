/**
 * Go `text/template` engine with Sprig helpers, template sets, and HTML escaping.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Supports one-shot rendering, pre-compiled `Template` objects, multi-template
 * `TemplateSet` layouts (`define`/`block`/`template`), bounded execution
 * limits, and optional automatic HTML escaping.
 *
 * @example
 * ```ts
 * import { createTemplateSet, render } from "vael:template";
 *
 * console.log(render("Hello, {{ upper .name }}!", { name: "vael" }));
 *
 * const set = createTemplateSet({ htmlEscape: true })
 *   .add("layout", "<html><body>{{ block \"content\" . }}{{ end }}</body></html>")
 *   .add("page", "{{ template \"layout\" . }}{{ define \"content\" }}<p>{{ .msg }}</p>{{ end }}");
 * console.log(set.render("page", { msg: "<safe>" }));
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:template" {
  /**
   * Error codes reported on `Error.code` by `vael:template` operations.
   *
   * @since 0.0.0-alpha.0
   */
  export type TemplateErrorCode =
    | "PARSE"
    | "RENDER"
    | "LIMIT_EXCEEDED"
    | "TEMPLATE_NOT_FOUND"
    | "INVALID_NAME"
    | "INVALID_DATA";

  /**
   * Options controlling Sprig helpers, HTML auto-escaping, and safety bounds.
   *
   * @since 0.0.0-alpha.0
   */
  export interface TemplateOptions {
    /** Enable safe deterministic Sprig template helpers (default: `true`). @since 0.0.0-alpha.0 */
    sprig?: boolean;

    /**
     * Restrict Sprig helpers to hermetic functions, omitting clock- and
     * environment-reading helpers such as `now` and `ago` (default: `false`).
     *
     * @since 0.0.0-alpha.0
     */
    hermetic?: boolean;

    /**
     * Automatically HTML-escape interpolated values unless marked with `safeHtml`
     * (default: `false`).
     *
     * @since 0.0.0-alpha.0
     */
    htmlEscape?: boolean;

    /** Maximum template source length in UTF-8 bytes (default: `1_048_576`). @since 0.0.0-alpha.0 */
    maxSourceLength?: number;

    /** Maximum control-structure nesting depth (default: `100`). @since 0.0.0-alpha.0 */
    maxNestingDepth?: number;

    /** Maximum rendered output length in UTF-8 bytes (default: `4_194_304`). @since 0.0.0-alpha.0 */
    maxOutputLength?: number;

    /** Maximum `range` loop iterations per render (default: `100_000`). @since 0.0.0-alpha.0 */
    maxIterations?: number;

    /** Maximum helper function calls per render (default: `100_000`). @since 0.0.0-alpha.0 */
    maxFunctionCalls?: number;
  }

  /**
   * A pre-compiled Go `text/template` ready for repeated rendering.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Template {
    /**
     * Render the compiled template against an optional JSON data object.
     *
     * @param data Optional JSON object context (defaults to `{}`).
     * @returns Rendered template output string.
     * @throws {Error} with `code: TemplateErrorCode`.
     * @example
     * ```ts
     * import { compile } from "vael:template";
     *
     * const tpl = compile("Hi {{ .user }}");
     * console.log(tpl.render({ user: "Ada" }));
     * ```
     * @since 0.0.0-alpha.0
     */
    render(data?: unknown): string;
  }

  /**
   * A named collection of templates supporting `define`, `template`, and
   * `block` layouts and partials.
   *
   * @since 0.0.0-alpha.0
   */
  export interface TemplateSet {
    /**
     * Validate and register a named template in the set, replacing any existing
     * entry with the same name. Returns `this` for chaining.
     *
     * @param name Unique template name.
     * @param source Go `text/template` source text.
     * @returns `this` for chaining.
     * @throws {Error} with `code` `"INVALID_NAME"`, `"INVALID_DATA"`,
     * `"LIMIT_EXCEEDED"`, or `"PARSE"`.
     * @since 0.0.0-alpha.0
     */
    add(name: string, source: string): TemplateSet;

    /**
     * Render the registered template `name` against optional JSON object `data`.
     *
     * @param name Registered template name to execute.
     * @param data Optional JSON object context (defaults to `{}`).
     * @returns Rendered template output string.
     * @throws {Error} with `code` `"TEMPLATE_NOT_FOUND"`, `"INVALID_NAME"`,
     * `"INVALID_DATA"`, `"LIMIT_EXCEEDED"`, `"PARSE"`, or `"RENDER"`.
     * @since 0.0.0-alpha.0
     */
    render(name: string, data?: unknown): string;

    /**
     * Return the registered template names in sorted order.
     *
     * @returns Lexicographically sorted list of registered template names.
     * @since 0.0.0-alpha.0
     */
    names(): string[];
  }

  /**
   * Compile and render a Go `text/template` once against optional JSON object
   * `data`.
   *
   * @param source Go `text/template` source text.
   * @param data Optional JSON object context (defaults to `{}`).
   * @param options Optional Sprig, HTML-escaping, and resource limit options.
   * @returns Rendered template output string.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: TemplateErrorCode`.
   * @example
   * ```ts
   * import { render } from "vael:template";
   *
   * console.log(render("Items: {{ len .items }}", { items: [1, 2, 3] }));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function render(
    source: string,
    data?: unknown,
    options?: TemplateOptions,
  ): string;

  /**
   * Compile a Go `text/template` once for repeated rendering.
   *
   * @param source Go `text/template` source text.
   * @param options Optional Sprig, HTML-escaping, and resource limit options.
   * @returns Pre-compiled `Template` handle.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: TemplateErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function compile(
    source: string,
    options?: TemplateOptions,
  ): Template;

  /**
   * Create an empty `TemplateSet` for composing layouts and partials.
   *
   * @param options Optional Sprig, HTML-escaping, and resource limit options.
   * @returns Empty `TemplateSet` instance.
   * @throws {Error} with `code` `"INVALID_DATA"` on invalid options.
   * @since 0.0.0-alpha.0
   */
  export function createTemplateSet(options?: TemplateOptions): TemplateSet;

  /**
   * Escape HTML special characters (`&`, `<`, `>`, `"`, `'`) in `input`.
   *
   * @param input Plain text string to HTML-escape.
   * @returns HTML-escaped string.
   * @throws {Error} with `code` `"INVALID_DATA"` if `input` is not a string.
   * @example
   * ```ts
   * import { escapeHtml } from "vael:template";
   *
   * console.log(escapeHtml("<script>")); // "&lt;script&gt;"
   * ```
   * @since 0.0.0-alpha.0
   */
  export function escapeHtml(input: string): string;

  /**
   * Default export mirroring the named `vael:template` exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** @since 0.0.0-alpha.0 */
    readonly render: typeof render;
    /** @since 0.0.0-alpha.0 */
    readonly compile: typeof compile;
    /** @since 0.0.0-alpha.0 */
    readonly createTemplateSet: typeof createTemplateSet;
    /** @since 0.0.0-alpha.0 */
    readonly escapeHtml: typeof escapeHtml;
  };

  /**
   * Default export mirroring `render`, `compile`, `createTemplateSet`, and `escapeHtml`.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}
