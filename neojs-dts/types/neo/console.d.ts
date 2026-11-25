declare module "neo:console" {
  /**
   * Runtime console object installed on `globalThis.console` (`neojs` and `neojs-lite`).
   */
  export interface NeoConsole {
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

  /** Write formatted arguments separated by spaces to stdout, followed by a newline. */
  export function log(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stdout, followed by a newline. */
  export function debug(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stdout, followed by a newline. */
  export function info(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stderr, followed by a newline. */
  export function warn(...args: unknown[]): void;
  /** Write formatted arguments separated by spaces to stderr, followed by a newline. */
  export function error(...args: unknown[]): void;

  const consoleObject: NeoConsole;
  /** Default export: the live `console` object installed by the runtime. */
  export default consoleObject;
}
