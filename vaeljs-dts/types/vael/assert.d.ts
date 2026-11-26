/**
 * Structural equality, identity, error, and rejection assertions.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Exports a callable default `assert` function with assertion methods attached.
 *
 * @example
 * ```ts
 * import assert from "vael:assert";
 *
 * assert.strictEquals(1 + 1, 2);
 * assert.equals({ a: [1, 2] }, { a: [1, 2] });
 * assert.throws(() => {
 *   throw new RangeError("bad index");
 * }, RangeError);
 * ```
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
     * @param message Optional failure message.
     * @param options Optional operator, actual, and expected metadata.
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
     * @param value Value tested for truthiness.
     * @param message Optional failure message.
     * @throws {TypeError} when `message` is not a string or arity is wrong.
     * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `value` is falsy.
     * @since 0.0.0-alpha.0
     */
    (value: unknown, message?: string): asserts value;

    /** `AssertionError` constructor reference. @since 0.0.0-alpha.0 */
    readonly AssertionError: typeof AssertionError;
    /** Alias for `assert`. @since 0.0.0-alpha.0 */
    readonly ok: VaelAssert;
    /** Structural equality assertion helper. @since 0.0.0-alpha.0 */
    readonly equals: typeof equals;
    /** Structural inequality assertion helper. @since 0.0.0-alpha.0 */
    readonly notEquals: typeof notEquals;
    /** `Object.is` identity equality assertion helper. @since 0.0.0-alpha.0 */
    readonly strictEquals: typeof strictEquals;
    /** `Object.is` identity inequality assertion helper. @since 0.0.0-alpha.0 */
    readonly notStrictEquals: typeof notStrictEquals;
    /** Unconditional failure helper. @since 0.0.0-alpha.0 */
    readonly fail: typeof fail;
    /** Synchronous throw assertion helper. @since 0.0.0-alpha.0 */
    readonly throws: typeof throws;
    /** Synchronous non-throw assertion helper. @since 0.0.0-alpha.0 */
    readonly doesNotThrow: typeof doesNotThrow;
    /** Asynchronous rejection assertion helper. @since 0.0.0-alpha.0 */
    readonly rejects: typeof rejects;
    /** Asynchronous fulfillment assertion helper. @since 0.0.0-alpha.0 */
    readonly doesNotReject: typeof doesNotReject;
  }

  /**
   * Assert that `value` is truthy.
   *
   * @param value Value tested for truthiness.
   * @param message Optional failure message.
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `value` is falsy.
   * @example
   * ```ts
   * import { assert } from "vael:assert";
   *
   * assert(2 > 1, "math should hold");
   * ```
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
   * @param actual Observed value.
   * @param expected Expected value.
   * @param message Optional failure message.
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when values differ.
   * @example
   * ```ts
   * import { equals } from "vael:assert";
   *
   * equals(new Set([1, 2]), new Set([2, 1]));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function equals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert that `actual` and `expected` are not structurally equal.
   *
   * @param actual Observed value.
   * @param expected Value that `actual` must not structurally equal.
   * @param message Optional failure message.
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when values are structurally equal.
   * @since 0.0.0-alpha.0
   */
  export function notEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert identity equality under `Object.is(actual, expected)`.
   *
   * @param actual Observed value.
   * @param expected Expected value.
   * @param message Optional failure message.
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `Object.is(actual, expected)` is false.
   * @example
   * ```ts
   * import { strictEquals } from "vael:assert";
   *
   * strictEquals(Number.NaN, Number.NaN);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function strictEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert that `actual` and `expected` differ under `Object.is(actual, expected)`.
   *
   * @param actual Observed value.
   * @param expected Value that `actual` must not match under `Object.is`.
   * @param message Optional failure message.
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `Object.is(actual, expected)` is true.
   * @since 0.0.0-alpha.0
   */
  export function notStrictEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Unconditionally throw an `AssertionError` (`code = "VAEL_ASSERT"`).
   *
   * @param message Optional failure message.
   * @returns Never returns normally.
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
   * @param callback Synchronous function expected to throw.
   * @param expected Constructor function or class the thrown value must match via `instanceof`.
   * @param message Optional failure message.
   * @returns The thrown error instance.
   * @throws {TypeError} on invalid arguments or Promise-returning callback.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` does not throw or mismatches.
   * @example
   * ```ts
   * import { throws } from "vael:assert";
   *
   * const err = throws(() => JSON.parse("{"), SyntaxError);
   * console.log(err.name);
   * ```
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
   * @param callback Synchronous function expected to throw.
   * @param expected Optional `RegExp` matched against the thrown value or error message.
   * @param message Optional failure message.
   * @returns The thrown value.
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
   * @param callback Synchronous function expected not to throw.
   * @param message Optional failure message.
   * @returns The value returned by `callback`.
   * @throws {TypeError} on invalid arguments or Promise-returning callback.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` throws.
   * @since 0.0.0-alpha.0
   */
  export function doesNotThrow<T>(callback: () => T, message?: string): T;

  /**
   * Assert that `callback` rejects (or throws synchronously) with an instance of
   * `expected`, resolving with the rejection reason.
   *
   * @param callback Function returning a Promise/thenable expected to reject.
   * @param expected Constructor function or class the rejection reason must match via `instanceof`.
   * @param message Optional failure message.
   * @returns Promise resolving to the rejection reason.
   * @throws {TypeError} when arguments are invalid.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` fulfills, returns non-thenable, or mismatches.
   * @example
   * ```ts
   * import { rejects } from "vael:assert";
   *
   * await rejects(async () => {
   *   throw new TypeError("bad input");
   * }, TypeError);
   * ```
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
   * @param callback Function returning a Promise/thenable expected to reject.
   * @param expected Optional `RegExp` matched against the rejection reason.
   * @param message Optional failure message.
   * @returns Promise resolving to the rejection reason.
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
   * @param callback Function returning a Promise/thenable expected to fulfill.
   * @param message Optional failure message.
   * @returns Promise resolving to the fulfillment value of `callback`.
   * @throws {TypeError} when arguments are invalid.
   * @throws {AssertionError} (`code = "VAEL_ASSERT"`) when `callback` throws, rejects, or returns non-thenable.
   * @since 0.0.0-alpha.0
   */
  export function doesNotReject<T>(callback: () => PromiseLike<T>, message?: string): Promise<T>;
}
