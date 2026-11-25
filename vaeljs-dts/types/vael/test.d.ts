/**
 * Sequential registration-based test runner and reporting API.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:test" {
  /**
   * Maximum number of tests that can be registered in one runtime (`10_000`).
   *
   * @since 0.0.0-alpha.0
   */
  export const MAX_TESTS: 10000;

  /**
   * Error codes thrown by `vael:test` operations on `Error.code`.
   *
   * @since 0.0.0-alpha.0
   */
  export type TestErrorCode =
    | "VAEL_TEST_FAILURES"
    | "VAEL_TEST_DURING_RUN"
    | "VAEL_TEST_ALREADY_RUNNING"
    | "VAEL_TEST_LIMIT"
    | "VAEL_TEST_UNSUPPORTED"
    | "VAEL_TEST_NO_TIMER"
    | "VAEL_TEST_PENDING"
    | "VAEL_TEST_CYCLE"
    | "VAEL_TEST_CHAIN"
    | "VAEL_TEST_OBSERVE";

  /**
   * Options accepted by `test(name, fn, options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface TestOptions {
    /**
     * Per-test timeout in milliseconds (`1..=2_147_483_647`, default `30_000`
     * in `vaeljs` V8).
     *
     * Not supported in `vaeljs-lite` (throws `VAEL_TEST_NO_TIMER` because QuickJS
     * has no host timers).
     *
     * @since 0.0.0-alpha.0
     */
    timeout?: number;
  }

  /**
   * Options accepted by `run(options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface RunOptions {
    /**
     * Throw `TestFailureError` (`code = "VAEL_TEST_FAILURES"`) if the cumulative
     * report contains any failure, pending test, unhandled rejection, or
     * unsettled event-loop work (default: `true`).
     *
     * @since 0.0.0-alpha.0
     */
    throwOnFailure?: boolean;
    /**
     * Run a bounded event-loop settle drain after tests finish (default: `false`).
     * Intended for the runtime's final pass after entry module evaluation.
     *
     * @since 0.0.0-alpha.0
     */
    settle?: boolean;
  }

  /**
   * Cumulative summary returned by `run(options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface TestReport {
    /** @since 0.0.0-alpha.0 */
    total: number;
    /** @since 0.0.0-alpha.0 */
    passed: number;
    /** @since 0.0.0-alpha.0 */
    failed: number;
    /** @since 0.0.0-alpha.0 */
    pending: number;
    /** @since 0.0.0-alpha.0 */
    unhandled: number;
    /** @since 0.0.0-alpha.0 */
    unsettled: number;
    /** @since 0.0.0-alpha.0 */
    failures: string[];
  }

  /**
   * Error thrown by `run()` when `throwOnFailure` is `true` and any test
   * failed, was pending, or left unhandled/unsettled work.
   *
   * @since 0.0.0-alpha.0
   */
  export class TestFailureError extends Error {
    /** Error name (`"TestFailureError"`). @since 0.0.0-alpha.0 */
    override name: "TestFailureError";
    /** Machine-readable error code (`"VAEL_TEST_FAILURES"`). @since 0.0.0-alpha.0 */
    code: "VAEL_TEST_FAILURES";
    /** Formatted failure messages. @since 0.0.0-alpha.0 */
    failures: string[];
    /** Construct a `TestFailureError`. @since 0.0.0-alpha.0 */
    constructor(message: string, failures: string[]);
  }

  /**
   * Register a pending test case (fails the run because no callback was supplied).
   *
   * @param name Non-blank test name.
   * @throws {TypeError} on invalid `name`.
   * @throws {Error} with `code` `"VAEL_TEST_DURING_RUN"` or `"VAEL_TEST_LIMIT"`.
   * @since 0.0.0-alpha.0
   */
  export function test(name: string): void;
  /**
   * Register a test case to execute on the next `run()`.
   *
   * Generator functions are rejected with `VAEL_TEST_UNSUPPORTED`.
   *
   * @param name Non-blank test name.
   * @param fn Synchronous or async test body.
   * @param options Optional registration options (`{ timeout }`).
   * @throws {TypeError} on invalid arguments or unknown options.
   * @throws {Error} with `code` `"VAEL_TEST_DURING_RUN"`, `"VAEL_TEST_LIMIT"`,
   * `"VAEL_TEST_UNSUPPORTED"`, or `"VAEL_TEST_NO_TIMER"`.
   * @since 0.0.0-alpha.0
   */
  export function test(name: string, fn: () => unknown, options?: TestOptions): void;

  /**
   * Execute queued tests in registration order and return the cumulative
   * `TestReport`.
   *
   * @throws {TypeError} on invalid options.
   * @throws {Error} with `code = "VAEL_TEST_ALREADY_RUNNING"` if re-entered.
   * @throws {TestFailureError} (`code = "VAEL_TEST_FAILURES"`) when `throwOnFailure`
   * is `true` and any failure, pending test, unhandled rejection, or unsettled
   * work was recorded.
   * @since 0.0.0-alpha.0
   */
  export function run(options?: RunOptions): Promise<TestReport>;

  /**
   * Default export: the `test` registration function.
   *
   * @since 0.0.0-alpha.0
   */
  export default test;
}
