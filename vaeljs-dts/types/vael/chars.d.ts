/**
 * Unicode scalar character classification and single-code-point case conversion predicates.
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
   * @since 0.0.0-alpha.0
   */
  export function isChar(value: unknown): boolean;

  /**
   * Return `true` if `value` is an ASCII scalar value (`0..=0x7F`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isAscii(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Latin-1 scalar value (`0..=0xFF`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isLatin1(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode control character (category `Cc`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isControl(value: unknown): boolean;

  /**
   * Return `true` if `value` is a decimal digit (Unicode category `Nd`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isDigit(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode letter (category `L`: `Lu | Ll | Lt | Lm | Lo`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isLetter(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode letter (`L`) or decimal digit (`Nd`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isLetterOrDigit(value: unknown): boolean;

  /**
   * Return `true` if `value` is a lowercase letter (Unicode category `Ll`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isLower(value: unknown): boolean;

  /**
   * Return `true` if `value` is an uppercase letter (Unicode category `Lu`). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isUpper(value: unknown): boolean;

  /**
   * Return `true` if `value` is Unicode punctuation (category `P`). Total predicate.
   *
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
   * @since 0.0.0-alpha.0
   */
  export function isSpace(value: unknown): boolean;

  /**
   * Return `true` if `value` is a Unicode symbol (category `S`, including emoji). Total predicate.
   *
   * @since 0.0.0-alpha.0
   */
  export function isSymbol(value: unknown): boolean;

  /**
   * Map a Unicode scalar value to its single-code-point lowercase equivalent.
   *
   * Multi-code-point expansions leave the scalar unchanged.
   *
   * @param value Unicode scalar value (`0..=0x10FFFF`, excluding surrogates).
   * @throws {TypeError} when `value` is not a number.
   * @throws {RangeError} when `value` is not a Unicode scalar value.
   * @since 0.0.0-alpha.0
   */
  export function toLower(value: number): number;

  /**
   * Map a Unicode scalar value to its single-code-point uppercase equivalent.
   *
   * Multi-code-point expansions such as `U+00DF` (`ß`) leave the scalar unchanged.
   *
   * @param value Unicode scalar value (`0..=0x10FFFF`, excluding surrogates).
   * @throws {TypeError} when `value` is not a number.
   * @throws {RangeError} when `value` is not a Unicode scalar value.
   * @since 0.0.0-alpha.0
   */
  export function toUpper(value: number): number;
}
