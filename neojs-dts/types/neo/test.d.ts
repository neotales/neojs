declare module "neo:test" {
  /** Maximum number of tests that can be registered in one runtime (`10_000`). */
  export const MAX_TESTS: 10000;

  /**
   * Error codes thrown by `neo:test` operations on `Error.code`.
   */
  export type TestErrorCode =
    | "NEO_TEST_FAILURES"
    | "NEO_TEST_DURING_RUN"
    | "NEO_TEST_ALREADY_RUNNING"
    | "NEO_TEST_LIMIT"
    | "NEO_TEST_UNSUPPORTED"
    | "NEO_TEST_NO_TIMER"
    | "NEO_TEST_PENDING"
    | "NEO_TEST_CYCLE"
    | "NEO_TEST_CHAIN"
    | "NEO_TEST_OBSERVE";

  /**
   * Options accepted by `test(name, fn, options)`.
   */
  export interface TestOptions {
    /**
     * Per-test timeout in milliseconds (`1..=2_147_483_647`, default `30_000`
     * in `neojs` V8).
     *
     * Not supported in `neojs-lite` (throws `NEO_TEST_NO_TIMER` because QuickJS
     * has no host timers).
     */
    timeout?: number;
  }

  /**
   * Options accepted by `run(options)`.
   */
  export interface RunOptions {
    /**
     * Throw `TestFailureError` (`code = "NEO_TEST_FAILURES"`) if the cumulative
     * report contains any failure, pending test, unhandled rejection, or
     * unsettled event-loop work (default: `true`).
     */
    throwOnFailure?: boolean;
    /**
     * Run a bounded event-loop settle drain after tests finish (default: `false`).
     * Intended for the runtime's final pass after entry module evaluation.
     */
    settle?: boolean;
  }

  /**
   * Cumulative summary returned by `run(options)`.
   */
  export interface TestReport {
    total: number;
    passed: number;
    failed: number;
    pending: number;
    unhandled: number;
    unsettled: number;
    failures: string[];
  }

  /**
   * Error thrown by `run()` when `throwOnFailure` is `true` and any test
   * failed, was pending, or left unhandled/unsettled work.
   */
  export class TestFailureError extends Error {
    override name: "TestFailureError";
    code: "NEO_TEST_FAILURES";
    failures: string[];
    constructor(message: string, failures: string[]);
  }

  /**
   * Register a pending test case (fails the run because no callback was supplied).
   *
   * @param name Non-blank test name.
   * @throws {TypeError} on invalid `name`.
   * @throws {Error} with `code` `"NEO_TEST_DURING_RUN"` or `"NEO_TEST_LIMIT"`.
   */
  export function test(name: string): void;
  /**
   * Register a test case to execute on the next `run()`.
   *
   * Generator functions are rejected with `NEO_TEST_UNSUPPORTED`.
   *
   * @param name Non-blank test name.
   * @param fn Synchronous or async test body.
   * @param options Optional registration options (`{ timeout }`).
   * @throws {TypeError} on invalid arguments or unknown options.
   * @throws {Error} with `code` `"NEO_TEST_DURING_RUN"`, `"NEO_TEST_LIMIT"`,
   * `"NEO_TEST_UNSUPPORTED"`, or `"NEO_TEST_NO_TIMER"`.
   */
  export function test(name: string, fn: () => unknown, options?: TestOptions): void;

  /**
   * Execute queued tests in registration order and return the cumulative
   * `TestReport`.
   *
   * @throws {TypeError} on invalid options.
   * @throws {Error} with `code = "NEO_TEST_ALREADY_RUNNING"` if re-entered.
   * @throws {TestFailureError} (`code = "NEO_TEST_FAILURES"`) when `throwOnFailure`
   * is `true` and any failure, pending test, unhandled rejection, or unsettled
   * work was recorded.
   */
  export function run(options?: RunOptions): Promise<TestReport>;

  /** Default export: the `test` registration function. */
  export default test;
}
