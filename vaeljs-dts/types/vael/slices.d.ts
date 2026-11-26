/**
 * Mutable and read-only live views over arrays, typed arrays, and Unicode code-point slices.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Provides zero-copy `[start, end)` views over `Array` and typed arrays
 * (`Slice`, `ReadOnlySlice`) as well as Unicode code-point views over
 * `Uint32Array` (`CharSlice`, `ReadOnlyCharSlice`).
 *
 * @example
 * ```ts
 * import { CharSlice, slice } from "vael:slices";
 *
 * const nums = [10, 20, 30, 40];
 * const middle = slice(nums, 1, 3);
 * middle.set(0, 99);
 * console.log(nums); // [10, 99, 30, 40]
 *
 * const chars = CharSlice.fromString("hello");
 * chars.slice(0, 1).toUpper();
 * console.log(chars.toString()); // "Hello"
 * ```
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
     * @param index Zero-based view element index in `0..length`.
     * @returns Element at `index`.
     * @throws {TypeError} when `index` is not a number.
     * @throws {RangeError} when `index` is not an integer in `0..length`.
     * @since 0.0.0-alpha.0
     */
    at(index: number): SliceElement<T>;

    /**
     * Write `value` through to the underlying target at view index `index`.
     *
     * @param index Zero-based view element index in `0..length`.
     * @param value New element value to write to the underlying target.
     * @returns `this` for chaining.
     * @throws {TypeError} when `index` is not a number or target rejects write.
     * @throws {RangeError} when `index` is not an integer in `0..length`.
     * @since 0.0.0-alpha.0
     */
    set(index: number, value: SliceElement<T>): this;

    /**
     * Return a nested mutable view over `[start, end)` relative to this view.
     *
     * @param start Inclusive start index relative to this view (default `0`).
     * @param end Exclusive end index relative to this view (default `this.length`).
     * @returns Nested mutable `Slice<T>`.
     * @throws {TypeError} when a bound is not a number.
     * @throws {RangeError} when bounds are invalid or out of range.
     * @since 0.0.0-alpha.0
     */
    slice(start?: number, end?: number): Slice<T>;

    /**
     * Copy the current view range into a new `Array` or typed array of the same kind.
     *
     * @returns Fresh snapshot copy of this slice's elements.
     * @throws {RangeError} when a typed-array target was detached or shrunk below the fixed range.
     * @since 0.0.0-alpha.0
     */
    copy(): SliceCopy<T>;

    /**
     * Return the first view index strictly equal (`===`) to `value`, or `-1`.
     *
     * @param value Element value to search for.
     * @returns First matching index in `0..length`, or `-1`.
     * @since 0.0.0-alpha.0
     */
    indexOf(value: SliceElement<T>): number;

    /**
     * Return `true` if any view element matches `value` under SameValueZero (`NaN` matches).
     *
     * @param value Element value to search for.
     * @returns `true` if present in this view.
     * @since 0.0.0-alpha.0
     */
    includes(value: SliceElement<T>): boolean;

    /**
     * Iterate current elements in view order.
     *
     * @returns Iterator over elements in view order.
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
     * @param index Zero-based view element index in `0..length`.
     * @returns Element at `index`.
     * @throws {TypeError} when `index` is not a number.
     * @throws {RangeError} when `index` is out of range.
     * @since 0.0.0-alpha.0
     */
    at(index: number): SliceElement<T>;

    /**
     * Return a nested read-only view over `[start, end)` relative to this view.
     *
     * @param start Inclusive start index relative to this view (default `0`).
     * @param end Exclusive end index relative to this view (default `this.length`).
     * @returns Nested `ReadOnlySlice<T>`.
     * @throws {TypeError} when a bound is not a number.
     * @throws {RangeError} when bounds are invalid or out of range.
     * @since 0.0.0-alpha.0
     */
    slice(start?: number, end?: number): ReadOnlySlice<T>;

    /**
     * Copy the current view range into a new `Array` or typed array of the same kind.
     *
     * @returns Fresh snapshot copy of this slice's elements.
     * @throws {RangeError} when a typed-array target was detached or shrunk below the fixed range.
     * @since 0.0.0-alpha.0
     */
    copy(): SliceCopy<T>;

    /**
     * Return the first view index strictly equal (`===`) to `value`, or `-1`.
     *
     * @param value Element value to search for.
     * @returns First matching index in `0..length`, or `-1`.
     * @since 0.0.0-alpha.0
     */
    indexOf(value: SliceElement<T>): number;

    /**
     * Return `true` if any view element matches `value` under SameValueZero (`NaN` matches).
     *
     * @param value Element value to search for.
     * @returns `true` if present in this view.
     * @since 0.0.0-alpha.0
     */
    includes(value: SliceElement<T>): boolean;

    /**
     * Iterate current elements in view order.
     *
     * @returns Iterator over elements in view order.
     * @since 0.0.0-alpha.0
     */
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
     * @throws {TypeError} when `buffer` is not a genuine `Uint32Array` or a bound is not a number.
     * @throws {RangeError} when bounds are invalid or out of range.
     * @since 0.0.0-alpha.0
     */
    constructor(buffer: Uint32Array, start?: number, end?: number);

    /**
     * Allocate a `Uint32Array` of code points from `text` (combining surrogate
     * pairs) and return a `CharSlice` over it.
     *
     * @param text Source JavaScript string.
     * @returns Mutable `CharSlice` backed by a newly allocated `Uint32Array`.
     * @throws {TypeError} when `text` is not a string.
     * @example
     * ```ts
     * import { CharSlice } from "vael:slices";
     *
     * const cs = CharSlice.fromString("abc");
     * console.log(cs.toUpper().toString()); // "ABC"
     * ```
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
     * @param index Zero-based code-point index in `0..length`.
     * @returns Numeric Unicode code point at `index`.
     * @throws {TypeError} when `index` is not a number.
     * @throws {RangeError} when `index` is out of range.
     * @since 0.0.0-alpha.0
     */
    at(index: number): number;

    /**
     * Write Unicode code point `value` (`0..=0x10FFFF`) at view index `index`.
     *
     * @param index Zero-based code-point index in `0..length`.
     * @param value Unicode code point integer in `0..=0x10FFFF`.
     * @returns `this` for chaining.
     * @throws {TypeError} when `index` or `value` is not a number.
     * @throws {RangeError} when `index` or `value` is out of range.
     * @since 0.0.0-alpha.0
     */
    set(index: number, value: number): this;

    /**
     * Return a nested mutable `CharSlice` over `[start, end)` relative to this view.
     *
     * @param start Inclusive start index relative to this view (default `0`).
     * @param end Exclusive end index relative to this view (default `this.length`).
     * @returns Nested mutable `CharSlice`.
     * @throws {TypeError} when a bound is not a number.
     * @throws {RangeError} when bounds are invalid or out of range.
     * @since 0.0.0-alpha.0
     */
    slice(start?: number, end?: number): CharSlice;

    /**
     * Map every Unicode scalar value in place using `vael:chars` `toUpper`.
     * Expanding mappings and surrogates are left unchanged.
     *
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    toUpper(): this;

    /**
     * Map every Unicode scalar value in place using `vael:chars` `toLower`.
     * Expanding mappings and surrogates are left unchanged.
     *
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    toLower(): this;

    /**
     * Build a new JavaScript string from the view's code points.
     *
     * @returns Decoded string representing the slice's code points.
     * @since 0.0.0-alpha.0
     */
    toString(): string;

    /**
     * Copy the view's code points into a new `Uint32Array`.
     *
     * @returns New `Uint32Array` snapshot of code points.
     * @since 0.0.0-alpha.0
     */
    toArray(): Uint32Array;

    /**
     * Iterate code points in view order.
     *
     * @returns Iterator over numeric code points.
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
    /**
     * Construct a `ReadOnlyCharSlice` over `buffer[start..end)`.
     *
     * @param buffer Genuine `Uint32Array` holding Unicode code points.
     * @param start Inclusive start code-point index (default `0`).
     * @param end Exclusive end code-point index (default `buffer.length`).
     * @throws {TypeError} when `buffer` is not a genuine `Uint32Array` or a bound is not a number.
     * @throws {RangeError} when bounds are invalid or out of range.
     * @since 0.0.0-alpha.0
     */
    constructor(buffer: Uint32Array, start?: number, end?: number);

    /**
     * Allocate a `Uint32Array` of code points from `text` and return a `ReadOnlyCharSlice` over it.
     *
     * @param text Source JavaScript string.
     * @returns Read-only `ReadOnlyCharSlice` backed by a newly allocated `Uint32Array`.
     * @throws {TypeError} when `text` is not a string.
     * @since 0.0.0-alpha.0
     */
    static fromString(text: string): ReadOnlyCharSlice;

    /** Fixed number of code points in the view. @since 0.0.0-alpha.0 */
    readonly length: number;

    /** Whether `length === 0`. @since 0.0.0-alpha.0 */
    readonly isEmpty: boolean;

    /**
     * Read the code point at view index `index`.
     *
     * @param index Zero-based code-point index in `0..length`.
     * @returns Numeric Unicode code point at `index`.
     * @throws {TypeError} when `index` is not a number.
     * @throws {RangeError} when `index` is out of range.
     * @since 0.0.0-alpha.0
     */
    at(index: number): number;

    /**
     * Return a nested read-only `CharSlice` over `[start, end)` relative to this view.
     *
     * @param start Inclusive start index relative to this view (default `0`).
     * @param end Exclusive end index relative to this view (default `this.length`).
     * @returns Nested `ReadOnlyCharSlice`.
     * @throws {TypeError} when a bound is not a number.
     * @throws {RangeError} when bounds are invalid or out of range.
     * @since 0.0.0-alpha.0
     */
    slice(start?: number, end?: number): ReadOnlyCharSlice;

    /**
     * Build a new JavaScript string from the view's code points.
     *
     * @returns Decoded string representing the slice's code points.
     * @since 0.0.0-alpha.0
     */
    toString(): string;

    /**
     * Copy the view's code points into a new `Uint32Array`.
     *
     * @returns New `Uint32Array` snapshot of code points.
     * @since 0.0.0-alpha.0
     */
    toArray(): Uint32Array;

    /**
     * Iterate code points in view order.
     *
     * @returns Iterator over numeric code points.
     * @since 0.0.0-alpha.0
     */
    [Symbol.iterator](): IterableIterator<number>;
  }

  /**
   * Create a mutable live `Slice` over `target[start..end)`.
   *
   * @param target Live `Array` or genuine typed array.
   * @param start Inclusive start index (default `0`).
   * @param end Exclusive end index (default `target.length`).
   * @returns Mutable `Slice<T>` view.
   * @throws {TypeError} when `target` is invalid or a bound is not a number.
   * @throws {RangeError} when bounds are invalid or out of range.
   * @example
   * ```ts
   * import { slice } from "vael:slices";
   *
   * const buf = new Uint8Array([1, 2, 3, 4]);
   * const head = slice(buf, 0, 2);
   * head.set(1, 20);
   * console.log(buf[1]); // 20
   * ```
   * @since 0.0.0-alpha.0
   */
  export function slice<T extends SliceTarget>(target: T, start?: number, end?: number): Slice<T>;

  /**
   * Create a read-only live `ReadOnlySlice` over `target[start..end)`.
   *
   * @param target Live `Array` or genuine typed array.
   * @param start Inclusive start index (default `0`).
   * @param end Exclusive end index (default `target.length`).
   * @returns Read-only `ReadOnlySlice<T>` view.
   * @throws {TypeError} when `target` is invalid or a bound is not a number.
   * @throws {RangeError} when bounds are invalid or out of range.
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
   * @param buffer Genuine `Uint32Array` of Unicode code points.
   * @param start Inclusive start code-point index (default `0`).
   * @param end Exclusive end code-point index (default `buffer.length`).
   * @returns Mutable `CharSlice` view.
   * @throws {TypeError} when `buffer` is invalid or a bound is not a number.
   * @throws {RangeError} when bounds are invalid or out of range.
   * @since 0.0.0-alpha.0
   */
  export function charSlice(buffer: Uint32Array, start?: number, end?: number): CharSlice;

  /**
   * Create a read-only `ReadOnlyCharSlice` over `buffer[start..end)`.
   *
   * @param buffer Genuine `Uint32Array` of Unicode code points.
   * @param start Inclusive start code-point index (default `0`).
   * @param end Exclusive end code-point index (default `buffer.length`).
   * @returns Read-only `ReadOnlyCharSlice` view.
   * @throws {TypeError} when `buffer` is invalid or a bound is not a number.
   * @throws {RangeError} when bounds are invalid or out of range.
   * @since 0.0.0-alpha.0
   */
  export function readonlyCharSlice(
    buffer: Uint32Array,
    start?: number,
    end?: number,
  ): ReadOnlyCharSlice;
}
