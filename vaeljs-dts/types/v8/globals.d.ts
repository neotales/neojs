/**
 * Global runtime declarations for the `vjs` (`vaeljs`, V8) profile.
 *
 * Available in the `vjs` runtime. Declares only `Vael` and `console`; browser
 * and legacy host globals are absent.
 *
 * @example
 * ```ts
 * console.log("Engine:", Vael.engine);
 * console.log("Modules:", Vael.getBuiltInModulesList());
 * const os = Vael.getBuiltInModule("vael:os");
 * console.log(os.platform, os.arch);
 * ```
 *
 * @since 0.0.0-alpha.0
 */

/**
 * Standard console output methods installed on `globalThis.console` in the
 * `vjs` (`vaeljs`, V8) runtime.
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
   * @param args Values to format and write to stdout.
   * @example
   * ```ts
   * console.log("hello", { ok: true });
   * ```
   * @since 0.0.0-alpha.0
   */
  log(...args: unknown[]): void;

  /**
   * Write formatted debug arguments separated by spaces to stdout, followed by a newline.
   *
   * @param args Values to format and write to stdout.
   * @since 0.0.0-alpha.0
   */
  debug(...args: unknown[]): void;

  /**
   * Write formatted informational arguments separated by spaces to stdout, followed by a newline.
   *
   * @param args Values to format and write to stdout.
   * @since 0.0.0-alpha.0
   */
  info(...args: unknown[]): void;

  /**
   * Write formatted warning arguments separated by spaces to stderr, followed by a newline.
   *
   * @param args Values to format and write to stderr.
   * @since 0.0.0-alpha.0
   */
  warn(...args: unknown[]): void;

  /**
   * Write formatted error arguments separated by spaces to stderr, followed by a newline.
   *
   * @param args Values to format and write to stderr.
   * @example
   * ```ts
   * console.error("operation failed:", new Error("timeout"));
   * ```
   * @since 0.0.0-alpha.0
   */
  error(...args: unknown[]): void;
}

/**
 * Mapping from canonical `vael:*` built-in module specifiers available in the
 * `vjs` (`vaeljs`, V8) profile to the object returned by `Vael.getBuiltInModule`.
 *
 * @since 0.0.0-alpha.0
 */
interface VaelBuiltInModuleMap {
  /** Built-in `vael:age` module object. @since 0.0.0-alpha.0 */
  "vael:age": typeof import("vael:age");
  /** Built-in `vael:assert` callable assertion object. @since 0.0.0-alpha.0 */
  "vael:assert": typeof import("vael:assert")["default"];
  /** Built-in `vael:chars` module object. @since 0.0.0-alpha.0 */
  "vael:chars": typeof import("vael:chars");
  /** Built-in `vael:console` live console object. @since 0.0.0-alpha.0 */
  "vael:console": typeof import("vael:console")["default"];
  /** Built-in `vael:cron` module object. @since 0.0.0-alpha.0 */
  "vael:cron": typeof import("vael:cron");
  /** Built-in `vael:crypto` module object. @since 0.0.0-alpha.0 */
  "vael:crypto": typeof import("vael:crypto");
  /** Built-in `vael:db` module object. @since 0.0.0-alpha.0 */
  "vael:db": typeof import("vael:db");
  /** Built-in `vael:fs` module object. @since 0.0.0-alpha.0 */
  "vael:fs": typeof import("vael:fs");
  /** Built-in `vael:http` module object. @since 0.0.0-alpha.0 */
  "vael:http": typeof import("vael:http");
  /** Built-in `vael:os` module object. @since 0.0.0-alpha.0 */
  "vael:os": typeof import("vael:os");
  /** Built-in `vael:path` module object. @since 0.0.0-alpha.0 */
  "vael:path": typeof import("vael:path");
  /** Built-in `vael:slices` module object. @since 0.0.0-alpha.0 */
  "vael:slices": typeof import("vael:slices");
  /** Built-in `vael:ssh` module object. @since 0.0.0-alpha.0 */
  "vael:ssh": typeof import("vael:ssh");
  /** Built-in `vael:stdio` module object. @since 0.0.0-alpha.0 */
  "vael:stdio": typeof import("vael:stdio");
  /** Built-in `vael:strings` module object. @since 0.0.0-alpha.0 */
  "vael:strings": typeof import("vael:strings");
  /** Built-in `vael:template` module object. @since 0.0.0-alpha.0 */
  "vael:template": Omit<typeof import("vael:template"), "default">;
  /** Built-in `vael:test` module object. @since 0.0.0-alpha.0 */
  "vael:test": Omit<typeof import("vael:test"), "default">;
  /** Built-in `vael:uuid` module object. @since 0.0.0-alpha.0 */
  "vael:uuid": typeof import("vael:uuid");
  /** Built-in `vael:uuid/v4` module object. @since 0.0.0-alpha.0 */
  "vael:uuid/v4": typeof import("vael:uuid/v4");
  /** Built-in `vael:uuid/v7` module object. @since 0.0.0-alpha.0 */
  "vael:uuid/v7": typeof import("vael:uuid/v7");
}

/**
 * Frozen introspection global installed on `globalThis.Vael` in the `vjs`
 * (`vaeljs`, V8) runtime.
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
   * @param specifier Canonical `vael:*` built-in module specifier.
   * @returns The cached built-in module object for `specifier`.
   * @throws {TypeError} when `specifier` is not a string.
   * @example
   * ```ts
   * const os = Vael.getBuiltInModule("vael:os");
   * console.log(os.platform);
   * ```
   * @since 0.0.0-alpha.0
   */
  getBuiltInModule<K extends keyof VaelBuiltInModuleMap>(specifier: K): VaelBuiltInModuleMap[K];

  /**
   * Return the cached built-in module object for a canonical `vael:*` specifier,
   * or `undefined` when the specifier is not registered.
   *
   * @param specifier Module specifier string to look up.
   * @returns The cached built-in module object, or `undefined` if unknown.
   * @throws {TypeError} when `specifier` is not a string.
   * @since 0.0.0-alpha.0
   */
  getBuiltInModule(specifier: string): unknown;

  /**
   * Return the frozen array of canonical `vael:*` specifiers enabled in this
   * runtime, in registration order.
   *
   * @returns Frozen array of registered `vael:*` module specifiers.
   * @example
   * ```ts
   * const modules = Vael.getBuiltInModulesList();
   * console.log(modules.includes("vael:fs"));
   * ```
   * @since 0.0.0-alpha.0
   */
  getBuiltInModulesList(): readonly string[];
}

/**
 * Global console output object installed by `vjs` (`vaeljs`, V8).
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
