/**
 * Standard console output methods mirroring `globalThis.console`.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Exports the same live `log`, `debug`, `info`, `warn`, and `error` function
 * objects installed on `globalThis.console`.
 *
 * @example
 * ```ts
 * import { error, info, log } from "vael:console";
 *
 * log("server starting");
 * info("listening on port", 8080);
 * error("unexpected state");
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:console" {
  /**
   * Runtime console object installed on `globalThis.console` (`vjs` and `vjs-lite`).
   *
   * @since 0.0.0-alpha.0
   */
  export interface VaelConsole {
    /**
     * Write formatted arguments separated by spaces to stdout, followed by a newline.
     *
     * @param args Values to format and write to stdout.
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
     * @since 0.0.0-alpha.0
     */
    error(...args: unknown[]): void;
  }

  /**
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @param args Values to format and write to stdout.
   * @example
   * ```ts
   * import { log } from "vael:console";
   *
   * log("status:", "ready");
   * ```
   * @since 0.0.0-alpha.0
   */
  export function log(...args: unknown[]): void;

  /**
   * Write formatted debug arguments separated by spaces to stdout, followed by a newline.
   *
   * @param args Values to format and write to stdout.
   * @since 0.0.0-alpha.0
   */
  export function debug(...args: unknown[]): void;

  /**
   * Write formatted informational arguments separated by spaces to stdout, followed by a newline.
   *
   * @param args Values to format and write to stdout.
   * @since 0.0.0-alpha.0
   */
  export function info(...args: unknown[]): void;

  /**
   * Write formatted warning arguments separated by spaces to stderr, followed by a newline.
   *
   * @param args Values to format and write to stderr.
   * @since 0.0.0-alpha.0
   */
  export function warn(...args: unknown[]): void;

  /**
   * Write formatted error arguments separated by spaces to stderr, followed by a newline.
   *
   * @param args Values to format and write to stderr.
   * @example
   * ```ts
   * import { error } from "vael:console";
   *
   * error("failed to bind port");
   * ```
   * @since 0.0.0-alpha.0
   */
  export function error(...args: unknown[]): void;

  const consoleObject: VaelConsole;

  /**
   * Default export: the live `console` object installed by the runtime.
   *
   * @since 0.0.0-alpha.0
   */
  export default consoleObject;
}
