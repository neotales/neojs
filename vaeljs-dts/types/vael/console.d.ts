/**
 * Standard console output methods mirroring `globalThis.console`.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:console" {
  /**
   * Runtime console object installed on `globalThis.console` (`vaeljs` and `vaeljs-lite`).
   *
   * @since 0.0.0-alpha.0
   */
  export interface VaelConsole {
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
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  export function log(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  export function debug(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stdout, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  export function info(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stderr, followed by a newline.
   *
   * @since 0.0.0-alpha.0
   */
  export function warn(...args: unknown[]): void;
  /**
   * Write formatted arguments separated by spaces to stderr, followed by a newline.
   *
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
