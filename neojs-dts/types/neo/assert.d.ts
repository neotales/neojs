declare module "neo:assert" {
  /**
   * Options accepted by `new AssertionError(message, options)`.
   */
  export interface AssertionErrorOptions {
    /** Assertion operator name (defaults to `"assert"`). */
    operator?: string;
    /** Observed value, if applicable. */
    actual?: unknown;
    /** Expected value or matcher, if applicable. */
    expected?: unknown;
  }

  /**
   * Error thrown when a `neo:assert` assertion fails.
   *
   * Carries `code = "NEO_ASSERT"`, `name = "AssertionError"`, and `operator`.
   */
  export class AssertionError extends Error {
    override name: "AssertionError";
    code: "NEO_ASSERT";
    operator: string;
    actual?: unknown;
    expected?: unknown;
    constructor(message?: string, options?: AssertionErrorOptions);
  }

  /**
   * Callable `assert` object exposing all assertion helpers as properties.
   */
  export interface NeoAssert {
    /**
     * Assert that `value` is truthy.
     *
     * @throws {TypeError} when `message` is not a string or arity is wrong.
     * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `value` is falsy.
     */
    (value: unknown, message?: string): asserts value;
    readonly AssertionError: typeof AssertionError;
    readonly ok: NeoAssert;
    readonly equals: typeof equals;
    readonly notEquals: typeof notEquals;
    readonly strictEquals: typeof strictEquals;
    readonly notStrictEquals: typeof notStrictEquals;
    readonly fail: typeof fail;
    readonly throws: typeof throws;
    readonly doesNotThrow: typeof doesNotThrow;
    readonly rejects: typeof rejects;
    readonly doesNotReject: typeof doesNotReject;
  }

  /**
   * Assert that `value` is truthy.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `value` is falsy.
   */
  export const assert: NeoAssert;

  /** Default export: the callable `assert` function with all assertion properties. */
  export default assert;

  /** Alias for `assert` (identical function object). */
  export const ok: NeoAssert;

  /**
   * Assert strict structural equality between `actual` and `expected`.
   *
   * Prototypes must match; `NaN` equals `NaN`; `-0` differs from `0`; `Map` and
   * `Set` ignore iteration order; cycles are handled.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when values differ.
   */
  export function equals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert that `actual` and `expected` are not structurally equal.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when values are structurally equal.
   */
  export function notEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert identity equality under `Object.is(actual, expected)`.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `Object.is(actual, expected)` is false.
   */
  export function strictEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Assert that `actual` and `expected` differ under `Object.is(actual, expected)`.
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `Object.is(actual, expected)` is true.
   */
  export function notStrictEquals(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Unconditionally throw an `AssertionError` (`code = "NEO_ASSERT"`).
   *
   * @throws {TypeError} when `message` is not a string or arity is wrong.
   * @throws {AssertionError} always.
   */
  export function fail(message?: string): never;

  /**
   * Assert that synchronous `callback` throws an instance of `expected`,
   * returning the thrown value.
   *
   * Returning a native `Promise` throws `TypeError` (use `rejects` instead).
   *
   * @throws {TypeError} on invalid arguments or Promise-returning callback.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `callback` does not throw or mismatches.
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
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `callback` does not throw or mismatches.
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
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `callback` throws.
   */
  export function doesNotThrow<T>(callback: () => T, message?: string): T;

  /**
   * Assert that `callback` rejects (or throws synchronously) with an instance of
   * `expected`, resolving with the rejection reason.
   *
   * @throws {TypeError} when arguments are invalid.
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `callback` fulfills, returns non-thenable, or mismatches.
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
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `callback` fulfills, returns non-thenable, or mismatches.
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
   * @throws {AssertionError} (`code = "NEO_ASSERT"`) when `callback` throws, rejects, or returns non-thenable.
   */
  export function doesNotReject<T>(callback: () => PromiseLike<T>, message?: string): Promise<T>;
}
