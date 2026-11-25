// Global runtime declarations for the `neojs` (V8) profile.
// Declares only `Neo` and `console`; browser and legacy host globals are absent.

/**
 * Standard console output methods installed on `globalThis.console` in the
 * `neojs` (V8) runtime.
 *
 * `log`, `debug`, and `info` write to stdout; `warn` and `error` write to
 * stderr. Strings print unchanged, typed arrays print their type and up to
 * ten elements, and objects are formatted as indented JSON when serializable.
 */
interface NeoConsole {
  /** Write formatted arguments separated by spaces to stdout, followed by a newline. */
  log(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stdout, followed by a newline. */
  debug(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stdout, followed by a newline. */
  info(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stderr, followed by a newline. */
  warn(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stderr, followed by a newline. */
  error(...args: unknown[]): void;
}

/**
 * Mapping from canonical `neo:*` built-in module specifiers available in the
 * `neojs` (V8) profile to the object returned by `Neo.getBuiltInModule`.
 */
interface NeoBuiltInModuleMap {
  "neo:age": typeof import("neo:age");
  "neo:assert": typeof import("neo:assert")["default"];
  "neo:chars": typeof import("neo:chars");
  "neo:console": typeof import("neo:console")["default"];
  "neo:cron": typeof import("neo:cron");
  "neo:crypto": typeof import("neo:crypto");
  "neo:db": typeof import("neo:db");
  "neo:fs": typeof import("neo:fs");
  "neo:http": typeof import("neo:http");
  "neo:os": typeof import("neo:os");
  "neo:path": typeof import("neo:path");
  "neo:slices": typeof import("neo:slices");
  "neo:ssh": typeof import("neo:ssh");
  "neo:stdio": typeof import("neo:stdio");
  "neo:strings": typeof import("neo:strings");
  "neo:template": Omit<typeof import("neo:template"), "default">;
  "neo:test": Omit<typeof import("neo:test"), "default">;
  "neo:uuid": typeof import("neo:uuid");
  "neo:uuid/v4": typeof import("neo:uuid/v4");
  "neo:uuid/v7": typeof import("neo:uuid/v7");
}

/**
 * Frozen introspection global installed on `globalThis.Neo` in the `neojs` (V8)
 * runtime.
 */
interface NeoGlobal {
  /** Active runtime engine identifier (`"neo"` for V8). */
  readonly engine: "neo";

  /**
   * Return the cached built-in module object for a canonical `neo:*` specifier,
   * or `undefined` when the specifier is not registered.
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
 * Global console output object installed by `neojs` (V8).
 */
declare const console: NeoConsole;

/**
 * Frozen NeoJS runtime introspection object (`engine: "neo"`).
 */
declare const Neo: NeoGlobal;
