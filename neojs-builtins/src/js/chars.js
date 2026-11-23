// The engine-neutral implementation of the `neo:chars` built-in module.
//
// This file is an ECMAScript expression, not a module. Evaluating it returns the
// frozen `chars` module object. The V8 realm shim and the QuickJS realm shim
// both splice this same expression in, so the two engines share one
// implementation and one set of captured intrinsics instead of maintaining
// independent copies.
//
// The expression captures the intrinsics it uses at evaluation time. Replacing
// `Number.isInteger`, `String.fromCodePoint`, `String.prototype.codePointAt`,
// `RegExp.prototype.exec`, `String.prototype.toLowerCase`,
// `String.prototype.toUpperCase`, `Object.freeze`, or the error constructors
// after the shim is installed does not change these functions. This is not
// isolation from code that replaced them before the shim ran, and it is not a
// sandbox.
//
// Character classification uses ECMAScript Unicode property escapes with the
// `u` flag. Both V8 and QuickJS implement them, but the Unicode version follows
// the engine build. A code point added in a newer Unicode release can therefore
// be classified differently by the two engines. The shared contract tests use
// long-assigned code points so the engines agree on the contract they enforce.
(() => {
    "use strict";

    const MAX_CODE_POINT = 0x10ffff;
    const SURROGATE_MIN = 0xd800;
    const SURROGATE_MAX = 0xdfff;
    const ASCII_MAX = 0x7f;
    const LATIN1_MAX = 0xff;

    const isInteger = Number.isInteger;
    const fromCodePoint = String.fromCodePoint;
    const codePointAt = Function.prototype.call.bind(String.prototype.codePointAt);
    // Capture `exec`, not `test`. `RegExp.prototype.test` looks up `this.exec`
    // dynamically, so replacing `RegExp.prototype.exec` would change what the
    // predicates answer or make them throw. The captured intrinsic does not
    // consult that mutable property.
    const regexpExec = Function.prototype.call.bind(RegExp.prototype.exec);
    const lowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
    const upperCase = Function.prototype.call.bind(String.prototype.toUpperCase);
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const freeze = Object.freeze;

    // A Unicode scalar value: an integer code point that is not a surrogate.
    // Surrogates cannot be encoded by `String.fromCodePoint`, and treating
    // them as characters would make the predicates disagree with strings.
    function isScalar(value) {
        return (
            typeof value === "number" &&
            isInteger(value) &&
            value >= 0 &&
            value <= MAX_CODE_POINT &&
            (value < SURROGATE_MIN || value > SURROGATE_MAX)
        );
    }

    const LETTER = /\p{L}/u;
    const DECIMAL_DIGIT = /\p{Nd}/u;
    const UPPERCASE_LETTER = /\p{Lu}/u;
    const LOWERCASE_LETTER = /\p{Ll}/u;
    const CONTROL = /\p{Cc}/u;
    const PUNCTUATION = /\p{P}/u;
    const SYMBOL = /\p{S}/u;
    // Go's unicode.IsSpace: ASCII \t through \r and space, U+0085, and the
    // Unicode separator categories. U+FEFF (BOM) and U+200B (zero width space)
    // are deliberately not whitespace here even though JavaScript's `\s`
    // includes the former.
    const WHITESPACE = /[\t\n\v\f\r \u0085\p{Zs}\p{Zl}\p{Zp}]/u;

    function matches(pattern, value) {
        return isScalar(value) && regexpExec(pattern, fromCodePoint(value)) !== null;
    }

    function requireScalar(value, name) {
        if (typeof value !== "number") {
            throw new makeTypeError(`${name} must be a number`);
        }
        if (!isScalar(value)) {
            throw new makeRangeError(`${name} must be a Unicode scalar value`);
        }
        return value;
    }

    // Apply a full case mapping to one code point and return a number only when
    // the result is still one code point. Expanding mappings (`ß` to `SS`,
    // `İ` to `i` plus a combining dot) are left unchanged because the API
    // returns a code point, not a string.
    function mapCase(value, map) {
        const codePoint = requireScalar(value, "value");
        const mapped = map(fromCodePoint(codePoint));
        const first = codePointAt(mapped, 0);
        const width = first > 0xffff ? 2 : 1;
        return mapped.length === width ? first : codePoint;
    }

    /** True for a Unicode scalar value. */
    function isChar(value) {
        return isScalar(value);
    }

    /** True for an ASCII scalar value, 0 through 0x7F. */
    function isAscii(value) {
        return isScalar(value) && value <= ASCII_MAX;
    }

    /** True for a Latin-1 scalar value, 0 through 0xFF. */
    function isLatin1(value) {
        return isScalar(value) && value <= LATIN1_MAX;
    }

    /** True for a C0 or C1 control character (Unicode category Cc). */
    function isControl(value) {
        return matches(CONTROL, value);
    }

    /** True for a decimal digit (Unicode category Nd), not other numerals. */
    function isDigit(value) {
        return matches(DECIMAL_DIGIT, value);
    }

    /** True for a letter (Unicode category L). */
    function isLetter(value) {
        return matches(LETTER, value);
    }

    /** True for a letter or a decimal digit. */
    function isLetterOrDigit(value) {
        return matches(LETTER, value) || matches(DECIMAL_DIGIT, value);
    }

    /** True for a lowercase letter (Unicode category Ll). */
    function isLower(value) {
        return matches(LOWERCASE_LETTER, value);
    }

    /** True for an uppercase letter (Unicode category Lu). */
    function isUpper(value) {
        return matches(UPPERCASE_LETTER, value);
    }

    /** True for punctuation (Unicode category P). */
    function isPunc(value) {
        return matches(PUNCTUATION, value);
    }

    /**
     * True for whitespace: ASCII tab, line feed, vertical tab, form feed,
     * carriage return, and space, plus U+0085 and categories Zs, Zl, and Zp.
     */
    function isSpace(value) {
        return matches(WHITESPACE, value);
    }

    /** True for a symbol (Unicode category S), including emoji. */
    function isSymbol(value) {
        return matches(SYMBOL, value);
    }

    /**
     * Return the lowercase code point for a scalar value, or the value itself
     * when the full mapping expands to more than one code point.
     *
     * @throws {TypeError} when the value is not a number.
     * @throws {RangeError} when the value is not a Unicode scalar value.
     */
    function toLower(value) {
        return mapCase(value, lowerCase);
    }

    /**
     * Return the uppercase code point for a scalar value, or the value itself
     * when the full mapping expands to more than one code point.
     *
     * @throws {TypeError} when the value is not a number.
     * @throws {RangeError} when the value is not a Unicode scalar value.
     */
    function toUpper(value) {
        return mapCase(value, upperCase);
    }

    return freeze({
        isChar,
        isAscii,
        isLatin1,
        isControl,
        isDigit,
        isLetter,
        isLetterOrDigit,
        isLower,
        isUpper,
        isPunc,
        isSpace,
        isSymbol,
        toLower,
        toUpper,
    });
})()
