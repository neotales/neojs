/**
 * Structural equality, identity, error, and rejection assertions.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:assert" {
  /**
   * Options accepted by `new AssertionError(message, options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface AssertionErrorOptions {
    /**
     * Assertion operator name (defaults to `"assert"`).
     *
     * @since 0.0.0-alpha.0
     */
    operator?: string;
    /**
     * Observed value, if applicable.
     *
     * @since 0.0.0-alpha.0
     */
    actual?: unknown;
    /**
     * Expected value or matcher, if applicable.
     *
     * @since 0.0.0-alpha.0
     */
    expected?: unknown;
  }

  /**
   * Error thrown when a `vael:assert` assertion fails.
   *
   * Carries `code = "VAEL_ASSERT"`, `name = "AssertionError"`, and `operator`.
   *
   * @since 0.0.0-alpha.0
   */
  export class AssertionError extends Error {
    /**
     * Error name (`"AssertionError"`).
     *
     * @since 0.0.0-alpha.0
     */
    override name: "AssertionError";
    /**
     * Machine-readable error code (`"VAEL_ASSERT"`).
     *
     * @since 0.0.0-alpha.0
     */
    code: "VAEL_ASSERT";
    /**
     * Assertion operator name.
     *
     * @since 0.0.0-alpha.0
     */
    operator: string;
    /**
     * Observed value, if applicable.
     *
     * @since 0.0.0-alpha.0
     */
    actual?: unknown;
    /**
     * Expected value or matcher, if applicable.
     *
     * @since 0.0.0-alpha.0
     */
    expected?: unknown;
    /**
     * Construct an `AssertionError`.
     *
     * @since 0.0.0-alpha.0
     */
    constructor(message?: string, options?: AssertionErrorOptions);
  }

  /**
   * Callable `assert` object exposing all assertion helpers as properties.
   *
   * @since 0.0.0-alpha.0
   */
  export interface VaelAssert {
    /**
     * Assert that `value` is truthy.
     *
     * @throws {TypeError} when `message` is not a string or arity is wrong.
     * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `value` is falsy.
     * @since 0.0.0-alpha.0
     */
    (value: unknown, message?: string): asserts value;
    /** @since 0.0.0-alpha.0 */
    readonly AssertionError: typeof AssertionError;
    /** @since 0.0.0-alpha.0 */
    readonly ok: VaelAssert;
    /** @since 0.0.0-alpha.0 */
    readonly equals: typeof equals;
    /** @since 0.0.0-alpha.0 */
    readonly notEquals: typeof notEquals;
    /** @since 0.0.0-alpha.0 */
    readonly strictEquals: typeof strictEquals;
    /** @since 0.0.0-alpha.0 */
    readonly notStrictEquals: typeof notStrictEquals;
    /** @since 0.0.0-alpha.0 */
    readonly fail: typeof fail;
    /** @since 0.0.0-alpha.0 */
    readonly throws: typeof throws;
    /** @since 0.0.0-alpha.0 */
    readonly doesNotThrow: typeof doesNotThrow;
    /** @since 0.0.0-alpha.0 */
    readonly rejects: typeof rejects;
    /** @since 0.0.0-alpha.0 */
    readonly doesNotReject: typeof doesNotReject;
  }

  /**
   * Assert that `value` is truthy.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `value` is falsy.
   * @since 0.0.0-alpha.0
   */
  export const assert: VaelAssert;

  /**
   * Default export: the callable `assert` function with all assertion properties.
   *
   * @since 0.0.0-alpha.0
   */
  export default assert;

  /**
   * Alias for `assert` (identical function object).
   *
   * @since 0.0.0-alpha.0
   */
  export const ok: VaelAssert;

  /**
   * Assert strict structural equality between `actual` and `expected`.
   *
   * Prototypes must match; `NaN` equals `NaN`; `-0` differs from `0`; `Map` and
   * `Set` ignore iteration order; cycles are handled.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when values differ.
   * @since 0.0.0-alpha.0
   */
  export function equals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert that `actual` and `expected` are not structurally equal.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when values are structurally equal.
   * @since 0.0.0-alpha.0
   */
  export function notEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert identity equality under `Object.is(actual, expected)`.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `Object.is(actual, expected)` is false.
   * @since 0.0.0-alpha.0
   */
  export function strictEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert that `actual` and `expected` differ under `Object.is(actual, expected)`.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `Object.is(actual, expected)` is true.
   * @since 0.0.0-alpha.0
   */
  export function notStrictEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Unconditionally throw an `AssertionError` (`code = "VAEL_ASSERT"`).
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} always.
   * @since 0.0.0-alpha.0
   */
  export function fail(message?: string): never;

  /**
   * Assert that synchronous `callback` throws an instance of `expected`,
   * returning the thrown value.
   *
   * Returning a native `Promise` throws `TypeError` (use `rejects` instead).
   *
   * @throws {TypeError} on invalid arguments or Promise-returning callback.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` does not throw or mismatches.
   * @since 0.0.0-alpha.0
   */
  export function throws<E>(
    callback: () => unknown,
    expected: abstract new (...args: any[]) => E,
    message?: string,
  ): E;
  /**
   * Assert that synchronous `callback` throws (optionally matching `expected`
   * `RegExp`), returning the thrown value.
   *
   * @throws {TypeError} on invalid arguments or Promise-returning callback.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` does not throw or mismatches.
   * @since 0.0.0-alpha.0
   */
  export function throws(
    callback: () => unknown,
    expected?: RegExp | null,
    message?: string,
  ): unknown;

  /**
   * Assert that synchronous `callback` returns without throwing, returning its
   * result.
   *
   * Returning a native `Promise` throws `TypeError` (use `doesNotReject` instead).
   *
   * @throws {TypeError} on invalid arguments or Promise-returning callback.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` throws.
   * @since 0.0.0-alpha.0
   */
  export function doesNotThrow<T>(callback: () => T, message?: string): T;

  /**
   * Assert that `callback` rejects (or throws synchronously) with an instance of
   * `expected`, resolving with the rejection reason.
   *
   * @throws {TypeError} when arguments are invalid.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` fulfills, returns non-thenable, or mismatches.
   * @since 0.0.0-alpha.0
   */
  export function rejects<E>(
    callback: () => unknown,
    expected: abstract new (...args: any[]) => E,
    message?: string,
  ): Promise<E>;
  /**
   * Assert that `callback` rejects (or throws synchronously), optionally
   * matching `expected` `RegExp`, resolving with the rejection reason.
   *
   * @throws {TypeError} when arguments are invalid.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` fulfills, returns non-thenable, or mismatches.
   * @since 0.0.0-alpha.0
   */
  export function rejects(
    callback: () => unknown,
    expected?: RegExp | null,
    message?: string,
  ): Promise<unknown>;

  /**
   * Assert that `callback` returns a Promise or thenable that fulfills without
   * throwing or rejecting, resolving with its fulfillment value.
   *
   * @throws {TypeError} when arguments are invalid.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` throws, rejects, or returns non-thenable.
   * @since 0.0.0-alpha.0
   */
  export function doesNotReject<T>(callback: () => PromiseLike<T>, message?: string): Promise<T>;
}
