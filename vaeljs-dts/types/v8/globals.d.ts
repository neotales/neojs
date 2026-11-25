/**
 * Global runtime declarations for the `vaeljs` (V8) profile.
 *
 * Declares only `Vael` and `console`; browser and legacy host globals are absent.
 *
 * @since 0.0.0-alpha.0
 */

/**
 * Standard console output methods installed on `globalThis.console` in the
 * `vaeljs` (V8) runtime.
 *
 * `log`, `debug`, and `info` write to stdout; `warn` and `error` write to
 * stderr. Strings print unchanged, typed arrays print their type and up to
 * ten elements, and objects are formatted as indented JSON when serializable.
 *
 * @since 0.0.0-alpha.0
 */
interface VaelConsole {
  /**
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  log(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  debug(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  info(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stderr, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  warn(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stderr, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  error(...args: unknown[]): void;
}

/**
 * Mapping from canonical `vael:*` built-in module specifiers available in the
 * `vaeljs` (V8) profile to the object returned by `Vael.getBuiltInModule`.
 *
 * @since 0.0.0-alpha.0
 */
interface VaelBuiltInModuleMap {
  "vael:age": typeof import("vael:age");
  "vael:assert": typeof import("vael:assert")["default"];
  "vael:chars": typeof import("vael:chars");
  "vael:console": typeof import("vael:console")["default"];
  "vael:cron": typeof import("vael:cron");
  "vael:crypto": typeof import("vael:crypto");
  "vael:db": typeof import("vael:db");
  "vael:fs": typeof import("vael:fs");
  "vael:http": typeof import("vael:http");
  "vael:os": typeof import("vael:os");
  "vael:path": typeof import("vael:path");
  "vael:slices": typeof import("vael:slices");
  "vael:ssh": typeof import("vael:ssh");
  "vael:stdio": typeof import("vael:stdio");
  "vael:strings": typeof import("vael:strings");
  "vael:template": Omit<typeof import("vael:template"), "default">;
  "vael:test": Omit<typeof import("vael:test"), "default">;
  "vael:uuid": typeof import("vael:uuid");
  "vael:uuid/v4": typeof import("vael:uuid/v4");
  "vael:uuid/v7": typeof import("vael:uuid/v7");
}

/**
 * Frozen introspection global installed on `globalThis.Vael` in the `vaeljs` (V8)
 * runtime.
 *
 * @since 0.0.0-alpha.0
 */
interface VaelGlobal {
  /**
   * Active runtime engine identifier (`"vael"` for V8).
   *
   * @since 0.0.0-alpha.0
   */
  readonly engine: "vael";

  /**
   * Return the cached built-in module object for a canonical `vael:*` specifier,
   * or `undefined` when the specifier is not registered.
   *
   * @throws {TypeError} when `specifier` is not a string.
   * @since 0.0.0-alpha.0
   */
  getBuiltInModule<K extends keyof VaelBuiltInModuleMap>(specifier: K): VaelBuiltInModuleMap[K];
  /**
   * Return the cached built-in module object for a canonical `vael:*` specifier,
   * or `undefined` when the specifier is not registered.
   *
   * @throws {TypeError} when `specifier` is not a string.
   * @since 0.0.0-alpha.0
   */
  getBuiltInModule(specifier: string): unknown;

  /**
   * Return the frozen array of canonical `vael:*` specifiers enabled in this
   * runtime, in registration order.
   *
   * @since 0.0.0-alpha.0
   */
  getBuiltInModulesList(): readonly string[];
}

/**
 * Global console output object installed by `vaeljs` (V8).
 *
 * @since 0.0.0-alpha.0
 */
declare const console: VaelConsole;

/**
 * Frozen VaelJS runtime introspection object (`engine: "vael"`).
 *
 * @since 0.0.0-alpha.0
 */
declare const Vael: VaelGlobal;
