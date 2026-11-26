/**
 * Unicode scalar character classification and single-code-point case conversion predicates.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * All classification predicates are total and never throw.
 *
 * @example
 * ```ts
 * import { isDigit, isLetter, isSpace, toLower, toUpper } from "vael:chars";
 *
 * console.log(isLetter("A".codePointAt(0))); // true
 * console.log(isDigit("7".codePointAt(0)));  // true
 * console.log(isSpace(" ".codePointAt(0)));  // true
 * console.log(String.fromCodePoint(toUpper(0x61), toLower(0x5a))); // "Az"
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:chars" {
  /**
   * Return `true` if `value` is a Unicode scalar value (integer in `0..=0x10FFFF`
   * excluding UTF-16 surrogates `0xD800..=0xDFFF`).
   *
   * Total predicate: non-numbers, `NaN`, infinities, fractions, and out-of-range
   * values return `false`.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a valid Unicode scalar code point.
   * @example
   * ```ts
   * import { isChar } from "vael:chars";
   *
   * console.log(isChar(0x1f680)); // true
   * console.log(isChar(0xd800));  // false (surrogate)
   * ```
   * @since 0.0.0-alpha.0
   */
  export function isChar(value: unknown): boolean;

  /**
   * Return `true` if `value` is an ASCII scalar value (`0..=0x7F`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is an integer in `0..=0x7F`.
   * @since 0.0.0-alpha.0
   */
  export function isAscii(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Latin-1 scalar value (`0..=0xFF`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is an integer in `0..=0xFF`.
   * @since 0.0.0-alpha.0
   */
  export function isLatin1(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode control character (category `Cc`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode control scalar.
   * @since 0.0.0-alpha.0
   */
  export function isControl(value: unknown): boolean;

  /**
   * Return `true` if `value` is a decimal digit (Unicode category `Nd`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode decimal digit scalar.
   * @since 0.0.0-alpha.0
   */
  export function isDigit(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode letter (category `L`: `Lu | Ll | Lt | Lm | Lo`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode letter scalar.
   * @since 0.0.0-alpha.0
   */
  export function isLetter(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode letter (`L`) or decimal digit (`Nd`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode letter or decimal digit scalar.
   * @since 0.0.0-alpha.0
   */
  export function isLetterOrDigit(value: unknown): boolean;

  /**
   * Return `true` if `value` is a lowercase letter (Unicode category `Ll`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode lowercase letter scalar.
   * @since 0.0.0-alpha.0
   */
  export function isLower(value: unknown): boolean;

  /**
   * Return `true` if `value` is an uppercase letter (Unicode category `Lu`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode uppercase letter scalar.
   * @since 0.0.0-alpha.0
   */
  export function isUpper(value: unknown): boolean;

  /**
   * Return `true` if `value` is Unicode punctuation (category `P`). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode punctuation scalar.
   * @since 0.0.0-alpha.0
   */
  export function isPunc(value: unknown): boolean;

  /**
   * Return `true` if `value` is whitespace (`\t`, `\n`, `\v`, `\f`, `\r`, space,
   * `U+0085`, or Unicode categories `Zs`, `Zl`, `Zp`).
   *
   * Follows Go `unicode.IsSpace` semantics: `U+FEFF` and `U+200B` return `false`.
   * Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode whitespace scalar.
   * @since 0.0.0-alpha.0
   */
  export function isSpace(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode symbol (category `S`, including emoji). Total predicate.
   *
   * @param value Candidate value to test.
   * @returns `true` when `value` is a Unicode symbol scalar.
   * @since 0.0.0-alpha.0
   */
  export function isSymbol(value: unknown): boolean;

  /**
   * Map a Unicode scalar value to its single-code-point lowercase equivalent.
   *
   * Multi-code-point expansions leave the scalar unchanged.
   *
   * @param value Unicode scalar value (`0..=0x10FFFF`, excluding surrogates).
   * @returns Lowercase Unicode scalar code point.
   * @throws {TypeError} when `value` is not a number.
   * @throws {RangeError} when `value` is not a Unicode scalar value.
   * @example
   * ```ts
   * import { toLower } from "vael:chars";
   *
   * console.log(String.fromCodePoint(toLower(0x41))); // "a"
   * ```
   * @since 0.0.0-alpha.0
   */
  export function toLower(value: number): number;

  /**
   * Map a Unicode scalar value to its single-code-point uppercase equivalent.
   *
   * Multi-code-point expansions such as `U+00DF` (`ß`) leave the scalar unchanged.
   *
   * @param value Unicode scalar value (`0..=0x10FFFF`, excluding surrogates).
   * @returns Uppercase Unicode scalar code point.
   * @throws {TypeError} when `value` is not a number.
   * @throws {RangeError} when `value` is not a Unicode scalar value.
   * @example
   * ```ts
   * import { toUpper } from "vael:chars";
   *
   * console.log(String.fromCodePoint(toUpper(0x61))); // "A"
   * ```
   * @since 0.0.0-alpha.0
   */
  export function toUpper(value: number): number;
}
