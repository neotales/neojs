/**
 * Unicode 16.0 case-folding string comparisons and code-point trimming.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Implements locale-independent Go `strings.EqualFold` semantics over UTF-16
 * strings and code-point-based trimming.
 *
 * @example
 * ```ts
 * import { equalFold, indexOfFold, trim } from "vael:strings";
 *
 * console.log(equalFold("GoLang", "golang")); // true
 * console.log(indexOfFold("Hello World", "WORLD")); // 6
 * console.log(trim("///path/to/file///", "/")); // "path/to/file"
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:strings" {
  /**
   * Compare `text` and `other` under locale-independent Unicode 16.0 simple
   * case folding (Go `strings.EqualFold` semantics over UTF-16 code units).
   *
   * Lone surrogates fold to themselves and never match half of a surrogate pair.
   *
   * @param text First string to compare.
   * @param other Second string to compare.
   * @returns `true` if `text` and `other` are equal under Unicode simple case folding.
   * @throws {TypeError} when `text` or `other` is not a string.
   * @example
   * ```ts
   * import { equalFold } from "vael:strings";
   *
   * console.log(equalFold("Kelvin", "Kelvin")); // true
   * ```
   * @since 0.0.0-alpha.0
   */
  export function equalFold(text: string, other: string): boolean;

  /**
   * Return `true` if `text` starts with `prefix` under Unicode 16.0 simple case
   * folding.
   *
   * @param text Input string.
   * @param prefix Prefix string to match.
   * @returns `true` if `text` starts with `prefix` under case folding.
   * @throws {TypeError} when `text` or `prefix` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function startsWithFold(text: string, prefix: string): boolean;

  /**
   * Return `true` if `text` ends with `suffix` under Unicode 16.0 simple case
   * folding.
   *
   * @param text Input string.
   * @param suffix Suffix string to match.
   * @returns `true` if `text` ends with `suffix` under case folding.
   * @throws {TypeError} when `text` or `suffix` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function endsWithFold(text: string, suffix: string): boolean;

  /**
   * Return the UTF-16 code-unit index of the first occurrence of `search` in
   * `text` at or after `fromIndex` under Unicode 16.0 simple case folding, or
   * `-1` if not found.
   *
   * @param text Input string to search.
   * @param search Substring to locate.
   * @param fromIndex Optional UTF-16 code-unit offset (clamped at `0` and `text.length`; `Infinity` / `-Infinity` clamp, `NaN` and fractions throw `RangeError`).
   * @returns UTF-16 code-unit index of the first match, or `-1`.
   * @throws {TypeError} when `text` or `search` is not a string, or `fromIndex` is defined and not a number.
   * @throws {RangeError} when `fromIndex` is `NaN` or a non-integer finite number.
   * @example
   * ```ts
   * import { indexOfFold } from "vael:strings";
   *
   * console.log(indexOfFold("alpha BETA beta", "beta", 7)); // 11
   * ```
   * @since 0.0.0-alpha.0
   */
  export function indexOfFold(text: string, search: string, fromIndex?: number): number;

  /**
   * Return the UTF-16 code-unit index of the last occurrence of `search` in
   * `text` starting at or before `fromIndex` under Unicode 16.0 simple case
   * folding, or `-1` if not found.
   *
   * @param text Input string to search.
   * @param search Substring to locate.
   * @param fromIndex Optional UTF-16 code-unit offset (defaults to `text.length`).
   * @returns UTF-16 code-unit index of the last match, or `-1`.
   * @throws {TypeError} when `text` or `search` is not a string, or `fromIndex` is defined and not a number.
   * @throws {RangeError} when `fromIndex` is `NaN` or a non-integer finite number.
   * @since 0.0.0-alpha.0
   */
  export function lastIndexOfFold(text: string, search: string, fromIndex?: number): number;

  /**
   * Remove leading and trailing code points contained in `chars` (or standard
   * ECMAScript `String.prototype.trim` whitespace when `chars` is omitted).
   *
   * Preserves lone surrogates in the retained interior.
   *
   * @param text Input string to trim.
   * @param chars Optional cutset string of code points to strip from both ends.
   * @returns Trimmed string.
   * @throws {TypeError} when `text` is not a string, or `chars` is defined and not a string.
   * @example
   * ```ts
   * import { trim } from "vael:strings";
   *
   * console.log(trim("--hello--", "-")); // "hello"
   * ```
   * @since 0.0.0-alpha.0
   */
  export function trim(text: string, chars?: string): string;

  /**
   * Remove leading code points contained in `chars` (or ECMAScript whitespace
   * when `chars` is omitted).
   *
   * @param text Input string to trim.
   * @param chars Optional cutset string of code points to strip from the start.
   * @returns String with matching leading code points removed.
   * @throws {TypeError} when `text` is not a string, or `chars` is defined and not a string.
   * @since 0.0.0-alpha.0
   */
  export function trimStart(text: string, chars?: string): string;

  /**
   * Remove trailing code points contained in `chars` (or ECMAScript whitespace
   * when `chars` is omitted).
   *
   * @param text Input string to trim.
   * @param chars Optional cutset string of code points to strip from the end.
   * @returns String with matching trailing code points removed.
   * @throws {TypeError} when `text` is not a string, or `chars` is defined and not a string.
   * @since 0.0.0-alpha.0
   */
  export function trimEnd(text: string, chars?: string): string;
}
