// Global runtime declarations for the `neojs-lite` (QuickJS) profile.
// Declares only `Neo` and `console` (`log` and `error`); V8-only modules and
// browser/legacy host globals are absent.

/**
 * Minimal console output methods installed on `globalThis.console` in the
 * `neojs-lite` (QuickJS) runtime.
 *
 * Only `log` (stdout) and `error` (stderr) exist; `debug`, `info`, and `warn`
 * are absent. Arguments are stringified with JavaScript `String(value)` and
 * joined with single spaces.
 */
interface NeoConsole {
  /** Write arguments converted via `String(value)` separated by spaces to stdout, followed by a newline. */
  log(...args: unknown[]): void;
  /** Write arguments converted via `String(value)` separated by spaces to stderr, followed by a newline. */
  error(...args: unknown[]): void;
}

/**
 * Mapping from canonical `neo:*` built-in module specifiers available in the
 * `neojs-lite` (QuickJS) profile to the object returned by
 * `Neo.getBuiltInModule`.
 */
interface NeoBuiltInModuleMap {
  "neo:assert": typeof import("neo:assert")["default"];
  "neo:chars": typeof import("neo:chars");
  "neo:fs": typeof import("neo:fs");
  "neo:os": typeof import("neo:os");
  "neo:strings": typeof import("neo:strings");
  "neo:test": Omit<typeof import("neo:test"), "default">;
  "neo:uuid": typeof import("neo:uuid");
  "neo:uuid/v4": typeof import("neo:uuid/v4");
  "neo:uuid/v7": typeof import("neo:uuid/v7");
}

/**
 * Frozen introspection global installed on `globalThis.Neo` in the `neojs-lite`
 * (QuickJS) runtime.
 */
interface NeoGlobal {
  /** Active runtime engine identifier (`"neo-lite"` for QuickJS). */
  readonly engine: "neo-lite";

  /**
   * Return the cached built-in module object for a canonical `neo:*` specifier
   * enabled in `neojs-lite`, or `undefined` when the specifier is not
   * implemented.
   *
   * @throws {TypeError} when `specifier` is not a string.
   */
  getBuiltInModule<K extends keyof NeoBuiltInModuleMap>(specifier: K): NeoBuiltInModuleMap[K];
  getBuiltInModule(specifier: string): unknown;

  /**
   * Return the frozen array of canonical `neo:*` specifiers enabled in this
   * runtime, in registration order.
   */
  getBuiltInModulesList(): readonly string[];
}

/**
 * Global minimal console output object installed by `neojs-lite` (QuickJS).
 */
declare const console: NeoConsole;

/**
 * Frozen NeoJS Lite runtime introspection object (`engine: "neo-lite"`).
 */
declare const Neo: NeoGlobal;
