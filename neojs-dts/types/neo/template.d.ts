declare module "neo:template" {
  /**
   * Error codes reported on `Error.code` by `neo:template` operations.
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
   */
  export interface TemplateOptions {
    /** Enable safe deterministic Sprig template helpers (default: `true`). */
    sprig?: boolean;
    /**
     * Restrict Sprig helpers to hermetic functions, omitting clock- and
     * environment-reading helpers such as `now` and `ago` (default: `false`).
     */
    hermetic?: boolean;
    /**
     * Automatically HTML-escape interpolated values unless marked with `safeHtml`
     * (default: `false`).
     */
    htmlEscape?: boolean;
    /** Maximum template source length in UTF-8 bytes (default: `1_048_576`). */
    maxSourceLength?: number;
    /** Maximum control-structure nesting depth (default: `100`). */
    maxNestingDepth?: number;
    /** Maximum rendered output length in UTF-8 bytes (default: `4_194_304`). */
    maxOutputLength?: number;
    /** Maximum `range` loop iterations per render (default: `100_000`). */
    maxIterations?: number;
    /** Maximum helper function calls per render (default: `100_000`). */
    maxFunctionCalls?: number;
  }

  /**
   * A pre-compiled Go `text/template` ready for repeated rendering.
   */
  export interface Template {
    /**
     * Render the compiled template against an optional JSON data object.
     *
     * @param data Optional JSON object context (defaults to `{}`).
     * @throws {Error} with `code: TemplateErrorCode`.
     */
    render(data?: unknown): string;
  }

  /**
   * A named collection of templates supporting `define`, `template`, and
   * `block` layouts and partials.
   */
  export interface TemplateSet {
    /**
     * Validate and register a named template in the set, replacing any existing
     * entry with the same name. Returns `this` for chaining.
     *
     * @throws {Error} with `code` `"INVALID_NAME"`, `"INVALID_DATA"`,
     * `"LIMIT_EXCEEDED"`, or `"PARSE"`.
     */
    add(name: string, source: string): TemplateSet;

    /**
     * Render the registered template `name` against optional JSON object `data`.
     *
     * @throws {Error} with `code` `"TEMPLATE_NOT_FOUND"`, `"INVALID_NAME"`,
     * `"INVALID_DATA"`, `"LIMIT_EXCEEDED"`, `"PARSE"`, or `"RENDER"`.
     */
    render(name: string, data?: unknown): string;

    /**
     * Return the registered template names in sorted order.
     */
    names(): string[];
  }

  /**
   * Compile and render a Go `text/template` once against optional JSON object
   * `data`.
   *
   * @throws {Error} with `code: TemplateErrorCode`.
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
   */
  export function compile(
    source: string,
    options?: TemplateOptions,
  ): Template;

  /**
   * Create an empty `TemplateSet` for composing layouts and partials.
   *
   * @throws {Error} with `code` `"INVALID_DATA"` on invalid options.
   */
  export function createTemplateSet(options?: TemplateOptions): TemplateSet;

  /**
   * Escape HTML special characters (`&`, `<`, `>`, `"`, `'`) in `input`.
   *
   * @throws {Error} with `code` `"INVALID_DATA"` if `input` is not a string.
   */
  export function escapeHtml(input: string): string;

  /** Default export mirroring the named `neo:template` exports. */
  const _default: {
    readonly render: typeof render;
    readonly compile: typeof compile;
    readonly createTemplateSet: typeof createTemplateSet;
    readonly escapeHtml: typeof escapeHtml;
  };
  export default _default;
}
