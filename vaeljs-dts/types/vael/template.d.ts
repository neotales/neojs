/**
 * Go `text/template` engine with Sprig helpers, template sets, and HTML escaping.
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
     * @throws {Error} with `code: TemplateErrorCode`.
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
     * @throws {Error} with `code` `"INVALID_NAME"`, `"INVALID_DATA"`,
     * `"LIMIT_EXCEEDED"`, or `"PARSE"`.
     * @since 0.0.0-alpha.0
     */
    add(name: string, source: string): TemplateSet;

    /**
     * Render the registered template `name` against optional JSON object `data`.
     *
     * @throws {Error} with `code` `"TEMPLATE_NOT_FOUND"`, `"INVALID_NAME"`,
     * `"INVALID_DATA"`, `"LIMIT_EXCEEDED"`, `"PARSE"`, or `"RENDER"`.
     * @since 0.0.0-alpha.0
     */
    render(name: string, data?: unknown): string;

    /**
     * Return the registered template names in sorted order.
     *
     * @since 0.0.0-alpha.0
     */
    names(): string[];
  }

  /**
   * Compile and render a Go `text/template` once against optional JSON object
   * `data`.
   *
   * @throws {Error} with `code: TemplateErrorCode`.
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
   * @throws {Error} with `code` `"INVALID_DATA"` on invalid options.
   * @since 0.0.0-alpha.0
   */
  export function createTemplateSet(options?: TemplateOptions): TemplateSet;

  /**
   * Escape HTML special characters (`&`, `<`, `>`, `"`, `'`) in `input`.
   *
   * @throws {Error} with `code` `"INVALID_DATA"` if `input` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function escapeHtml(input: string): string;

  /**
   * Default export mirroring the named `vael:template` exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    readonly render: typeof render;
    readonly compile: typeof compile;
    readonly createTemplateSet: typeof createTemplateSet;
    readonly escapeHtml: typeof escapeHtml;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}
