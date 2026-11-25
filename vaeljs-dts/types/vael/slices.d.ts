/**
 * Mutable and read-only live views over arrays, typed arrays, and Unicode code-point slices.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:slices" {
  /**
   * Genuine typed array types accepted by `Slice` and `ReadOnlySlice` (excludes
   * `DataView` and proxies of typed arrays).
   *
   * @since 0.0.0-alpha.0
   */
  export type TypedArray =
    | Int8Array
    | Uint8Array
    | Uint8ClampedArray
    | Int16Array
    | Uint16Array
    | Int32Array
    | Uint32Array
    | Float32Array
    | Float64Array
    | BigInt64Array
    | BigUint64Array;

  /**
   * Valid live target for `Slice` and `ReadOnlySlice`.
   *
   * @since 0.0.0-alpha.0
   */
  export type SliceTarget<E = unknown> = E[] | TypedArray;

  /**
   * Element type yielded by reading or iterating `Slice<T>` / `ReadOnlySlice<T>`.
   *
   * @since 0.0.0-alpha.0
   */
  export type SliceElement<T extends SliceTarget> =
    T extends readonly (infer E)[]
      ? E
      : T extends BigInt64Array | BigUint64Array
        ? bigint
        : number;

  /**
   * Snapshot type returned by `copy()` (a new plain array for array targets, or
   * a new intrinsic typed array of the same type).
   *
   * @since 0.0.0-alpha.0
   */
  export type SliceCopy<T extends SliceTarget> = T extends readonly (infer E)[] ? E[] : T;

  /**
   * Mutable live view over a fixed `[start, end)` range of an `Array` or genuine
   * typed array.
   *
   * Construction, `slice`, `at`, `set`, `indexOf`, `includes`, and iteration do
   * not copy; `copy()` is the explicit snapshot operation.
   *
   * @since 0.0.0-alpha.0
   */
  export class Slice<T extends SliceTarget = SliceTarget> {
    /**
     * Construct a mutable live view over `target[start..end)`.
     *
     * @param target Live `Array` or genuine typed array.
     * @param start Inclusive start index (integer `>= 0`, default `0`).
     * @param end Exclusive end index (integer `<= target.length`, default `target.length`).
     * @throws {TypeError} when `target` is invalid or a bound is not a number.
     * @throws {RangeError} when bounds are non-integer, negative, reversed, or out of range.
     * @since 0.0.0-alpha.0
     */
    constructor(target: T, start?: number, end?: number);

    /** Fixed number of elements in the view. @since 0.0.0-alpha.0 */
    readonly length: number;

    /** Whether `length === 0`. @since 0.0.0-alpha.0 */
    readonly isEmpty: boolean;

    /**
     * Read the element at view index `index`.
     *
     * @throws {TypeError} when `index` is not a number.
     * @throws {RangeError} when `index` is not an integer in `0..length`.
     * @since 0.0.0-alpha.0
     */
    at(index: number): SliceElement<T>;

    /**
     * Write `value` through to the underlying target at view index `index`.
     *
     * @returns `this` for chaining.
     * @throws {TypeError} when `index` is not a number or target rejects write.
     * @throws {RangeError} when `index` is not an integer in `0..length`.
     * @since 0.0.0-alpha.0
     */
    set(index: number, value: SliceElement<T>): this;

    /**
     * Return a nested mutable view over `[start, end)` relative to this view.
     *
     * @since 0.0.0-alpha.0
     */
    slice(start?: number, end?: number): Slice<T>;

    /**
     * Copy the current view range into a new `Array` or typed array of the same kind.
     *
     * @throws {RangeError} when a typed-array target was detached or shrunk below the fixed range.
     * @since 0.0.0-alpha.0
     */
    copy(): SliceCopy<T>;

    /**
     * Return the first view index strictly equal (`===`) to `value`, or `-1`.
     *
     * @since 0.0.0-alpha.0
     */
    indexOf(value: SliceElement<T>): number;

    /**
     * Return `true` if any view element matches `value` under SameValueZero (`NaN` matches).
     *
     * @since 0.0.0-alpha.0
     */
    includes(value: SliceElement<T>): boolean;

    /**
     * Iterate current elements in view order.
     *
     * @since 0.0.0-alpha.0
     */
    [Symbol.iterator](): IterableIterator<SliceElement<T>>;
  }

  /**
   * Read-only live view over a fixed `[start, end)` range of an `Array` or
   * genuine typed array.
   *
   * Reads observe writes made through the target or another view.
   *
   * @since 0.0.0-alpha.0
   */
  export class ReadOnlySlice<T extends SliceTarget = SliceTarget> {
    /**
     * Construct a read-only live view over `target[start..end)`.
     *
     * @since 0.0.0-alpha.0
     */
    constructor(target: T, start?: number, end?: number);
    /** Fixed number of elements in the view. @since 0.0.0-alpha.0 */
    readonly length: number;
    /** Whether `length === 0`. @since 0.0.0-alpha.0 */
    readonly isEmpty: boolean;
    /** Read the element at view index `index`. @since 0.0.0-alpha.0 */
    at(index: number): SliceElement<T>;
    /** Return a nested read-only view over `[start, end)` relative to this view. @since 0.0.0-alpha.0 */
    slice(start?: number, end?: number): ReadOnlySlice<T>;
    /** Copy the current view range into a new `Array` or typed array of the same kind. @since 0.0.0-alpha.0 */
    copy(): SliceCopy<T>;
    /** Return the first view index strictly equal (`===`) to `value`, or `-1`. @since 0.0.0-alpha.0 */
    indexOf(value: SliceElement<T>): number;
    /** Return `true` if any view element matches `value` under SameValueZero (`NaN` matches). @since 0.0.0-alpha.0 */
    includes(value: SliceElement<T>): boolean;
    /** Iterate current elements in view order. @since 0.0.0-alpha.0 */
    [Symbol.iterator](): IterableIterator<SliceElement<T>>;
  }

  /**
   * Mutable live view over Unicode code points in a caller-owned `Uint32Array`.
   *
   * @since 0.0.0-alpha.0
   */
  export class CharSlice {
    /**
     * Construct a `CharSlice` over `buffer[start..end)`.
     *
     * @param buffer Genuine `Uint32Array` holding Unicode code points.
     * @param start Inclusive start code-point index (default `0`).
     * @param end Exclusive end code-point index (default `buffer.length`).
     * @since 0.0.0-alpha.0
     */
    constructor(buffer: Uint32Array, start?: number, end?: number);

    /**
     * Allocate a `Uint32Array` of code points from `text` (combining surrogate
     * pairs) and return a `CharSlice` over it.
     *
     * @since 0.0.0-alpha.0
     */
    static fromString(text: string): CharSlice;

    /** Fixed number of code points in the view. @since 0.0.0-alpha.0 */
    readonly length: number;

    /** Whether `length === 0`. @since 0.0.0-alpha.0 */
    readonly isEmpty: boolean;

    /**
     * Read the code point at view index `index`.
     *
     * @since 0.0.0-alpha.0
     */
    at(index: number): number;

    /**
     * Write Unicode code point `value` (`0..=0x10FFFF`) at view index `index`.
     *
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    set(index: number, value: number): this;

    /**
     * Return a nested mutable `CharSlice` over `[start, end)` relative to this view.
     *
     * @since 0.0.0-alpha.0
     */
    slice(start?: number, end?: number): CharSlice;

    /**
     * Map every Unicode scalar value in place using `vael:chars` `toUpper`.
     * Expanding mappings and surrogates are left unchanged.
     *
     * @since 0.0.0-alpha.0
     */
    toUpper(): this;

    /**
     * Map every Unicode scalar value in place using `vael:chars` `toLower`.
     * Expanding mappings and surrogates are left unchanged.
     *
     * @since 0.0.0-alpha.0
     */
    toLower(): this;

    /**
     * Build a new JavaScript string from the view's code points.
     *
     * @since 0.0.0-alpha.0
     */
    toString(): string;

    /**
     * Copy the view's code points into a new `Uint32Array`.
     *
     * @since 0.0.0-alpha.0
     */
    toArray(): Uint32Array;

    /**
     * Iterate code points in view order.
     *
     * @since 0.0.0-alpha.0
     */
    [Symbol.iterator](): IterableIterator<number>;
  }

  /**
   * Read-only live view over Unicode code points in a caller-owned `Uint32Array`.
   *
   * @since 0.0.0-alpha.0
   */
  export class ReadOnlyCharSlice {
    /** Construct a `ReadOnlyCharSlice` over `buffer[start..end)`. @since 0.0.0-alpha.0 */
    constructor(buffer: Uint32Array, start?: number, end?: number);
    /** Allocate a `Uint32Array` of code points from `text` and return a `ReadOnlyCharSlice` over it. @since 0.0.0-alpha.0 */
    static fromString(text: string): ReadOnlyCharSlice;
    /** Fixed number of code points in the view. @since 0.0.0-alpha.0 */
    readonly length: number;
    /** Whether `length === 0`. @since 0.0.0-alpha.0 */
    readonly isEmpty: boolean;
    /** Read the code point at view index `index`. @since 0.0.0-alpha.0 */
    at(index: number): number;
    /** Return a nested read-only `CharSlice` over `[start, end)` relative to this view. @since 0.0.0-alpha.0 */
    slice(start?: number, end?: number): ReadOnlyCharSlice;
    /** Build a new JavaScript string from the view's code points. @since 0.0.0-alpha.0 */
    toString(): string;
    /** Copy the view's code points into a new `Uint32Array`. @since 0.0.0-alpha.0 */
    toArray(): Uint32Array;
    /** Iterate code points in view order. @since 0.0.0-alpha.0 */
    [Symbol.iterator](): IterableIterator<number>;
  }

  /**
   * Create a mutable live `Slice` over `target[start..end)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function slice<T extends SliceTarget>(target: T, start?: number, end?: number): Slice<T>;

  /**
   * Create a read-only live `ReadOnlySlice` over `target[start..end)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function readonlySlice<T extends SliceTarget>(
    target: T,
    start?: number,
    end?: number,
  ): ReadOnlySlice<T>;

  /**
   * Create a mutable `CharSlice` over `buffer[start..end)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function charSlice(buffer: Uint32Array, start?: number, end?: number): CharSlice;

  /**
   * Create a read-only `ReadOnlyCharSlice` over `buffer[start..end)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function readonlyCharSlice(
    buffer: Uint32Array,
    start?: number,
    end?: number,
  ): ReadOnlyCharSlice;
}
