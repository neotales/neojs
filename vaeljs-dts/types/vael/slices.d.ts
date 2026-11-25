declare module "vael:slices" {
  /**
   * Genuine typed array types accepted by `Slice` and `ReadOnlySlice` (excludes
   * `DataView` and proxies of typed arrays).
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
   */
  export type SliceTarget<E = unknown> = E[] | TypedArray;

  /**
   * Element type yielded by reading or iterating `Slice<T>` / `ReadOnlySlice<T>`.
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
   */
  export type SliceCopy<T extends SliceTarget> = T extends readonly (infer E)[] ? E[] : T;

  /**
   * Mutable live view over a fixed `[start, end)` range of an `Array` or genuine
   * typed array.
   *
   * Construction, `slice`, `at`, `set`, `indexOf`, `includes`, and iteration do
   * not copy; `copy()` is the explicit snapshot operation.
   */
  export class Slice<T extends SliceTarget = SliceTarget> {
    /**
     * @param target Live `Array` or genuine typed array.
     * @param start Inclusive start index (integer `>= 0`, default `0`).
     * @param end Exclusive end index (integer `<= target.length`, default `target.length`).
     * @throws {TypeError} when `target` is invalid or a bound is not a number.
     * @throws {RangeError} when bounds are non-integer, negative, reversed, or out of range.
     */
    constructor(target: T, start?: number, end?: number);

    /** Fixed number of elements in the view. */
    readonly length: number;

    /** Whether `length === 0`. */
    readonly isEmpty: boolean;

    /**
     * Read the element at view index `index`.
     *
     * @throws {TypeError} when `index` is not a number.
     * @throws {RangeError} when `index` is not an integer in `0..length`.
     */
    at(index: number): SliceElement<T>;

    /**
     * Write `value` through to the underlying target at view index `index`.
     *
     * @returns `this` for chaining.
     * @throws {TypeError} when `index` is not a number or target rejects write.
     * @throws {RangeError} when `index` is not an integer in `0..length`.
     */
    set(index: number, value: SliceElement<T>): this;

    /**
     * Return a nested mutable view over `[start, end)` relative to this view.
     */
    slice(start?: number, end?: number): Slice<T>;

    /**
     * Copy the current view range into a new `Array` or typed array of the same kind.
     *
     * @throws {RangeError} when a typed-array target was detached or shrunk below the fixed range.
     */
    copy(): SliceCopy<T>;

    /** Return the first view index strictly equal (`===`) to `value`, or `-1`. */
    indexOf(value: SliceElement<T>): number;

    /** Return `true` if any view element matches `value` under SameValueZero (`NaN` matches). */
    includes(value: SliceElement<T>): boolean;

    /** Iterate current elements in view order. */
    [Symbol.iterator](): IterableIterator<SliceElement<T>>;
  }

  /**
   * Read-only live view over a fixed `[start, end)` range of an `Array` or
   * genuine typed array.
   *
   * Reads observe writes made through the target or another view.
   */
  export class ReadOnlySlice<T extends SliceTarget = SliceTarget> {
    constructor(target: T, start?: number, end?: number);
    readonly length: number;
    readonly isEmpty: boolean;
    at(index: number): SliceElement<T>;
    slice(start?: number, end?: number): ReadOnlySlice<T>;
    copy(): SliceCopy<T>;
    indexOf(value: SliceElement<T>): number;
    includes(value: SliceElement<T>): boolean;
    [Symbol.iterator](): IterableIterator<SliceElement<T>>;
  }

  /**
   * Mutable live view over Unicode code points in a caller-owned `Uint32Array`.
   */
  export class CharSlice {
    /**
     * @param buffer Genuine `Uint32Array` holding Unicode code points.
     * @param start Inclusive start code-point index (default `0`).
     * @param end Exclusive end code-point index (default `buffer.length`).
     */
    constructor(buffer: Uint32Array, start?: number, end?: number);

    /**
     * Allocate a `Uint32Array` of code points from `text` (combining surrogate
     * pairs) and return a `CharSlice` over it.
     */
    static fromString(text: string): CharSlice;

    /** Fixed number of code points in the view. */
    readonly length: number;

    /** Whether `length === 0`. */
    readonly isEmpty: boolean;

    /** Read the code point at view index `index`. */
    at(index: number): number;

    /**
     * Write Unicode code point `value` (`0..=0x10FFFF`) at view index `index`.
     *
     * @returns `this` for chaining.
     */
    set(index: number, value: number): this;

    /** Return a nested mutable `CharSlice` over `[start, end)` relative to this view. */
    slice(start?: number, end?: number): CharSlice;

    /**
     * Map every Unicode scalar value in place using `vael:chars` `toUpper`.
     * Expanding mappings and surrogates are left unchanged.
     */
    toUpper(): this;

    /**
     * Map every Unicode scalar value in place using `vael:chars` `toLower`.
     * Expanding mappings and surrogates are left unchanged.
     */
    toLower(): this;

    /** Build a new JavaScript string from the view's code points. */
    toString(): string;

    /** Copy the view's code points into a new `Uint32Array`. */
    toArray(): Uint32Array;

    /** Iterate code points in view order. */
    [Symbol.iterator](): IterableIterator<number>;
  }

  /**
   * Read-only live view over Unicode code points in a caller-owned `Uint32Array`.
   */
  export class ReadOnlyCharSlice {
    constructor(buffer: Uint32Array, start?: number, end?: number);
    static fromString(text: string): ReadOnlyCharSlice;
    readonly length: number;
    readonly isEmpty: boolean;
    at(index: number): number;
    slice(start?: number, end?: number): ReadOnlyCharSlice;
    toString(): string;
    toArray(): Uint32Array;
    [Symbol.iterator](): IterableIterator<number>;
  }

  /** Create a mutable live `Slice` over `target[start..end)`. */
  export function slice<T extends SliceTarget>(target: T, start?: number, end?: number): Slice<T>;

  /** Create a read-only live `ReadOnlySlice` over `target[start..end)`. */
  export function readonlySlice<T extends SliceTarget>(
    target: T,
    start?: number,
    end?: number,
  ): ReadOnlySlice<T>;

  /** Create a mutable `CharSlice` over `buffer[start..end)`. */
  export function charSlice(buffer: Uint32Array, start?: number, end?: number): CharSlice;

  /** Create a read-only `ReadOnlyCharSlice` over `buffer[start..end)`. */
  export function readonlyCharSlice(
    buffer: Uint32Array,
    start?: number,
    end?: number,
  ): ReadOnlyCharSlice;
}
