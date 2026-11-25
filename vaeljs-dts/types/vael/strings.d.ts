declare module "vael:strings" {
  /**
   * Compare `text` and `other` under locale-independent Unicode 16.0 simple
   * case folding (Go `strings.EqualFold` semantics over UTF-16 code units).
   *
   * Lone surrogates fold to themselves and never match half of a surrogate pair.
   *
   * @throws {TypeError} when `text` or `other` is not a string.
   */
  export function equalFold(text: string, other: string): boolean;

  /**
   * Return `true` if `text` starts with `prefix` under Unicode 16.0 simple case
   * folding.
   *
   * @throws {TypeError} when `text` or `prefix` is not a string.
   */
  export function startsWithFold(text: string, prefix: string): boolean;

  /**
   * Return `true` if `text` ends with `suffix` under Unicode 16.0 simple case
   * folding.
   *
   * @throws {TypeError} when `text` or `suffix` is not a string.
   */
  export function endsWithFold(text: string, suffix: string): boolean;

  /**
   * Return the UTF-16 code-unit index of the first occurrence of `search` in
   * `text` at or after `fromIndex` under Unicode 16.0 simple case folding, or
   * `-1` if not found.
   *
   * @param fromIndex Optional UTF-16 code-unit offset (clamped at `0` and `text.length`; `Infinity` / `-Infinity` clamp, `NaN` and fractions throw `RangeError`).
   * @throws {TypeError} when `text` or `search` is not a string, or `fromIndex` is defined and not a number.
   * @throws {RangeError} when `fromIndex` is `NaN` or a non-integer finite number.
   */
  export function indexOfFold(text: string, search: string, fromIndex?: number): number;

  /**
   * Return the UTF-16 code-unit index of the last occurrence of `search` in
   * `text` starting at or before `fromIndex` under Unicode 16.0 simple case
   * folding, or `-1` if not found.
   *
   * @param fromIndex Optional UTF-16 code-unit offset (defaults to `text.length`).
   * @throws {TypeError} when `text` or `search` is not a string, or `fromIndex` is defined and not a number.
   * @throws {RangeError} when `fromIndex` is `NaN` or a non-integer finite number.
   */
  export function lastIndexOfFold(text: string, search: string, fromIndex?: number): number;

  /**
   * Remove leading and trailing code points contained in `chars` (or standard
   * ECMAScript `String.prototype.trim` whitespace when `chars` is omitted).
   *
   * Preserves lone surrogates in the retained interior.
   *
   * @throws {TypeError} when `text` is not a string, or `chars` is defined and not a string.
   */
  export function trim(text: string, chars?: string): string;

  /**
   * Remove leading code points contained in `chars` (or ECMAScript whitespace
   * when `chars` is omitted).
   *
   * @throws {TypeError} when `text` is not a string, or `chars` is defined and not a string.
   */
  export function trimStart(text: string, chars?: string): string;

  /**
   * Remove trailing code points contained in `chars` (or ECMAScript whitespace
   * when `chars` is omitted).
   *
   * @throws {TypeError} when `text` is not a string, or `chars` is defined and not a string.
   */
  export function trimEnd(text: string, chars?: string): string;
}
