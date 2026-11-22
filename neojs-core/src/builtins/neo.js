// neojs built-in modules.
//
// This script is evaluated once per realm before the entry module. It installs
// the global `Neo` object and the module objects that object caches. `chars` and
// `console` are plain ECMAScript; `crypto` calls twenty-six private host ops:
// four use the operating system CSPRNG and AES-256-GCM in
// `neojs-core/src/crypto.rs`; eight implement bounded AES-256-CBC/AES-256-GCM
// ciphers in `neojs-core/src/crypto_cipher.rs`; seven implement SHA-2 hashing and HMAC in
// `neojs-core/src/crypto_hash.rs`; and seven implement PBKDF2 and bcrypt in
// `neojs-core/src/crypto_hash/password.rs`. The hash-family ops include
// bounded asynchronous calls on the runtime's Tokio blocking pool and explicit
// incremental-object disposal; `age` calls four private host ops that use the
// `neotales-age` crate
// for binary age v1 X25519 encryption; `stdio` calls four
// private host ops that copy or encode bytes, copy a byte range out of a private
// snapshot, and write to the current process's standard output and standard
// error. `ssh` calls three private host ops that dial one host, verify an
// explicit host-key pin or known-hosts document, authenticate, and run one
// command at a time. None of them is a permission or sandbox
// and none is gated on a secure context or TLS. `stdio` adds no filesystem,
// process, environment, or stdin capability. `ssh` adds outbound TCP and SSH
// command execution; an explicit Unix agent opt-in reads SSH_AUTH_SOCK after
// host verification, but other modes read no environment and no agent. It adds
// no filesystem, stdin, or permission capability and never forwards an agent.
// The `console` module is the realm's installed console
// object, not a separate console or an isolation boundary.
//
// The `chars` module is an independent implementation for neojs. It is
// informed by `@neotales/chars` and the QuickJS `neo:chars` module, but it does
// not port their lookup tables and does not claim compatibility with them.
// Character classification uses the Unicode property tables compiled into V8,
// so the exact Unicode version follows the V8 build.
(() => {
    "use strict";

    const MAX_CODE_POINT = 0x10ffff;
    const SURROGATE_MIN = 0xd800;
    const SURROGATE_MAX = 0xdfff;
    const ASCII_MAX = 0x7f;
    const LATIN1_MAX = 0xff;

    // Capture the built-ins used below so later replacements in the same realm
    // do not change classification. This is not isolation from code that
    // replaced them before this script ran, and it is not a sandbox.
    const isInteger = Number.isInteger;
    const fromCodePoint = String.fromCodePoint;
    const fromCharCode = String.fromCharCode;
    const codePointAt = Function.prototype.call.bind(String.prototype.codePointAt);
    // Capture `exec`, not `test`. `RegExp.prototype.test` looks up `this.exec`
    // dynamically, so replacing `RegExp.prototype.exec` would change what the
    // predicates answer or make them throw. The captured intrinsic does not
    // consult that mutable property.
    const regexpExec = Function.prototype.call.bind(RegExp.prototype.exec);
    const lowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
    const upperCase = Function.prototype.call.bind(String.prototype.toUpperCase);
    // Look up modules through a bound intrinsic, not `modules.get`. A dynamic
    // property lookup would consult the mutable `Map.prototype.get`, letting
    // later code replace lookups or receive the private registry map itself.
    const mapGet = Function.prototype.call.bind(Map.prototype.get);
    const makeError = Error;
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const defineProperty = Object.defineProperty;
    const objectKeys = Object.keys;
    const arrayIsArray = Array.isArray;
    const arrayIncludes = Function.prototype.call.bind(Array.prototype.includes);
    // The stdio module builds and reads typed arrays without consulting mutable
    // globals or prototype methods. These captures are not isolation from code
    // that replaced them before this script ran.
    const makeUint8Array = Uint8Array;
    // `set` copies without a species lookup. `subarray` is deliberately not
    // captured or used: it calls `Symbol.species`, which could pass the private
    // snapshot buffer to user JavaScript and re-enter the module. Range copies
    // go through the native slice op instead.
    const typedArraySet = Function.prototype.call.bind(Uint8Array.prototype.set);

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

    const chars = Object.freeze({
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

    // `strings` is plain ECMAScript: folded matching and set-based trimming.
    // It calls no host op and adds no capability beyond the strings it is
    // given. Folding is ECMAScript's simple case folding, decided per code
    // point by the engine's own RegExp `/iu` canonicalization, so no Unicode
    // table is shipped and the Unicode version follows the V8 build. This is
    // informed by `@neotales/strings` and `@neotales/slices` but is an
    // independent implementation; it does not port their tables and does not
    // claim byte-for-byte parity with them.
    //
    // Indices are UTF-16 code unit offsets, like `String.prototype.indexOf`,
    // not Unicode code point offsets. A lone surrogate is compared as itself
    // and never matches half of a surrogate pair. JS RegExp `$` without `m`
    // matches only at the end of the input, so `endsWithFold("a\n", "a")` is
    // false; unlike Python's `re`, there is no match before a final newline.
    //
    // Capture the intrinsics used below at shim evaluation. Replacing them
    // afterwards does not change this module. This is not isolation from code
    // that replaced them before this script ran, and it is not a sandbox.
    const makeRegExp = RegExp;
    const makeSet = Set;
    const makeUint32Array = Uint32Array;
    const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
    const stringSlice = Function.prototype.call.bind(String.prototype.slice);
    const stringTrim = Function.prototype.call.bind(String.prototype.trim);
    const stringTrimStart = Function.prototype.call.bind(String.prototype.trimStart);
    const stringTrimEnd = Function.prototype.call.bind(String.prototype.trimEnd);
    const setAdd = Function.prototype.call.bind(Set.prototype.add);
    const setHas = Function.prototype.call.bind(Set.prototype.has);
    const mapSet = Function.prototype.call.bind(Map.prototype.set);

    // Code units that are special outside a character class in a RegExp
    // source: \ ^ $ . * + ? ( ) [ ] { } |. Everything else in a literal is
    // escaped as itself, so a caller's text is always a literal pattern.
    const REGEXP_SPECIAL = new makeUint8Array(128);
    for (const special of [
        0x5c, 0x5e, 0x24, 0x2e, 0x2a, 0x2b, 0x3f, 0x28, 0x29, 0x5b, 0x5d, 0x7b,
        0x7d, 0x7c,
    ]) {
        REGEXP_SPECIAL[special] = 1;
    }

    // One single-code-point pattern per code point is enough to decide fold
    // equality, and the cache keeps a repeated non-ASCII comparison cheap. It
    // is bounded so a string of many distinct code points cannot retain an
    // unbounded number of RegExp objects; once full, comparisons fall back to
    // building a pattern they do not keep.
    const FOLD_CACHE_LIMIT = 4096;
    const foldCache = new Map();
    let foldCacheSize = 0;

    function escapeRegExp(text) {
        let escaped = "";
        for (let index = 0; index < text.length; index += 1) {
            const code = charCodeAt(text, index);
            if (code < 128 && REGEXP_SPECIAL[code] === 1) escaped += "\\";
            escaped += fromCharCode(code);
        }
        return escaped;
    }

    function requireString(value, name) {
        if (typeof value !== "string") {
            throw new makeTypeError(`${name} must be a string`);
        }
    }

    /**
     * Clamp an optional index argument to `0..length`.
     *
     * `undefined` uses `fallback`; `Infinity` and `-Infinity` clamp like the
     * corresponding `String.prototype` arguments. Any other non-integer is a
     * `RangeError`, and a non-number is a `TypeError`: this module does not
     * coerce its arguments.
     */
    function toIndex(value, length, fallback) {
        if (value === undefined) return fallback;
        if (typeof value !== "number") {
            throw new makeTypeError("fromIndex must be a number");
        }
        if (value === Infinity) return length;
        if (value === -Infinity) return 0;
        if (!isInteger(value)) {
            throw new makeRangeError("fromIndex must be an integer");
        }
        if (value <= 0) return 0;
        if (value >= length) return length;
        return value;
    }

    function codepointWidth(point) {
        return point > 0xffff ? 2 : 1;
    }

    /**
     * Move `index` forward off the trailing half of a surrogate pair.
     *
     * No well-formed match can start in the middle of a pair, so this changes
     * no result; it only keeps the scan at code point boundaries.
     */
    function codepointStart(text, index) {
        if (index <= 0 || index >= text.length) return index;
        const unit = charCodeAt(text, index);
        if (unit >= 0xdc00 && unit <= 0xdfff) {
            const previous = charCodeAt(text, index - 1);
            if (previous >= 0xd800 && previous <= 0xdbff) return index + 1;
        }
        return index;
    }

    /** The start offset of the code point that ends just before `end`. */
    function previousCodepointStart(text, end) {
        const unit = charCodeAt(text, end - 1);
        if (unit >= 0xdc00 && unit <= 0xdfff && end >= 2) {
            const high = charCodeAt(text, end - 2);
            if (high >= 0xd800 && high <= 0xdbff) return end - 2;
        }
        return end - 1;
    }

    function asciiFoldEqual(left, right) {
        if (left === right) return true;
        if (left >= 0x41 && left <= 0x5a) left += 0x20;
        if (right >= 0x41 && right <= 0x5a) right += 0x20;
        return left === right;
    }

    /**
     * True when two code points are equivalent under simple case folding.
     *
     * ASCII is decided directly. For anything else the engine's `/iu` matcher
     * decides, which is the ECMAScript definition of canonicalization and the
     * same relation `String.prototype` regular expressions use.
     */
    function foldEqual(left, right) {
        if (left === right) return true;
        if (left < 0x80 && right < 0x80) return asciiFoldEqual(left, right);
        let expression = mapGet(foldCache, left);
        if (expression === undefined) {
            expression = makeRegExp(
                `^(?:${escapeRegExp(fromCodePoint(left))})$`,
                "iu",
            );
            if (foldCacheSize < FOLD_CACHE_LIMIT) {
                mapSet(foldCache, left, expression);
                foldCacheSize += 1;
            }
        }
        return regexpExec(expression, fromCodePoint(right)) !== null;
    }

    /** True when `pattern` matches `text` starting at `start`. */
    function foldMatchAt(text, start, pattern) {
        let textIndex = start;
        let patternIndex = 0;
        while (patternIndex < pattern.length) {
            if (textIndex >= text.length) return false;
            const textPoint = codePointAt(text, textIndex);
            const patternPoint = codePointAt(pattern, patternIndex);
            if (!foldEqual(textPoint, patternPoint)) return false;
            textIndex += codepointWidth(textPoint);
            patternIndex += codepointWidth(patternPoint);
        }
        return true;
    }

    /**
     * Determine whether two strings are equal under simple case folding.
     *
     * Locale-independent: this is not `toLowerCase()` comparison and does not
     * apply Turkish or other locale rules. `"K"` equals U+212A KELVIN SIGN and
     * `"s"` equals U+017F LATIN SMALL LETTER LONG S, while `"ss"` does not
     * equal `"ß"` (that is full folding) and `"I"` does not equal `"ı"`.
     *
     * @param {string} text
     * @param {string} other
     * @returns {boolean}
     * @throws {TypeError} when an argument is not a string.
     */
    function equalFold(text, other) {
        requireString(text, "text");
        requireString(other, "other");
        if (text === other) return true;
        let textIndex = 0;
        let otherIndex = 0;
        while (textIndex < text.length && otherIndex < other.length) {
            const textPoint = codePointAt(text, textIndex);
            const otherPoint = codePointAt(other, otherIndex);
            if (!foldEqual(textPoint, otherPoint)) return false;
            textIndex += codepointWidth(textPoint);
            otherIndex += codepointWidth(otherPoint);
        }
        return textIndex === text.length && otherIndex === other.length;
    }

    /**
     * Determine whether `text` begins with `prefix`, ignoring case.
     *
     * @param {string} text
     * @param {string} prefix
     * @returns {boolean}
     * @throws {TypeError} when an argument is not a string.
     */
    function startsWithFold(text, prefix) {
        requireString(text, "text");
        requireString(prefix, "prefix");
        return foldMatchAt(text, 0, prefix);
    }

    /**
     * Determine whether `text` ends with `suffix`, ignoring case.
     *
     * `$` in a JavaScript RegExp without the `m` flag matches only at the end
     * of the input, so a final newline is part of the text: `"a\n"` does not
     * end with `"a"`.
     *
     * @param {string} text
     * @param {string} suffix
     * @returns {boolean}
     * @throws {TypeError} when an argument is not a string.
     */
    function endsWithFold(text, suffix) {
        requireString(text, "text");
        requireString(suffix, "suffix");
        let textIndex = text.length;
        let suffixIndex = suffix.length;
        while (suffixIndex > 0) {
            if (textIndex <= 0) return false;
            const textStart = previousCodepointStart(text, textIndex);
            const suffixStart = previousCodepointStart(suffix, suffixIndex);
            const textPoint = codePointAt(text, textStart);
            const suffixPoint = codePointAt(suffix, suffixStart);
            if (!foldEqual(textPoint, suffixPoint)) return false;
            textIndex = textStart;
            suffixIndex = suffixStart;
        }
        return true;
    }

    /** Count the code points in `text`. */
    function codePointCount(text) {
        let count = 0;
        for (let index = 0; index < text.length; ) {
            index += codepointWidth(codePointAt(text, index));
            count += 1;
        }
        return count;
    }

    /**
     * Copy the code points of `text` into a `Uint32Array`.
     *
     * KMP needs random access to the pattern, so the search string is
     * materialized once and the text is streamed. Extra memory is O(search).
     */
    function codePointsOf(text) {
        const count = codePointCount(text);
        const points = new makeUint32Array(count);
        let index = 0;
        for (let position = 0; position < count; position += 1) {
            const point = codePointAt(text, index);
            points[position] = point;
            index += codepointWidth(point);
        }
        return points;
    }

    /**
     * Compute the KMP prefix function under simple-fold equivalence.
     *
     * `foldEqual` is an equivalence relation: ECMAScript canonicalization is
     * applied to both code points, so equality of canonical forms is
     * reflexive, symmetric, and transitive. The usual prefix-function
     * recurrence is therefore valid even when the pattern contains different
     * code points from one fold class, such as `"K"` and U+212A.
     */
    function foldPrefix(pattern) {
        const length = pattern.length;
        const prefix = new makeUint32Array(length);
        let matched = 0;
        for (let index = 1; index < length; index += 1) {
            const point = pattern[index];
            while (matched > 0 && !foldEqual(point, pattern[matched])) {
                matched = prefix[matched - 1];
            }
            if (foldEqual(point, pattern[matched])) matched += 1;
            prefix[index] = matched;
        }
        return prefix;
    }

    /**
     * Find the first occurrence of `search` in `text`, ignoring case.
     *
     * Linear in the text and the search string. The scan is KMP over code
     * points with `foldEqual` as the equivalence, so a long repeated prefix
     * cannot make the search retry the whole needle at every position. The
     * text is streamed; only the needle, its prefix function, and a ring of
     * UTF-16 start offsets are materialized, so extra memory is O(search).
     * Matches start and end on code point boundaries, and indices are UTF-16
     * code unit offsets.
     *
     * @param {string} text
     * @param {string} search
     * @param {number} [fromIndex=0] an integer, or `Infinity`/`-Infinity`
     * @returns {number} a UTF-16 code unit index, or `-1`
     * @throws {TypeError} when `text` or `search` is not a string, or
     * `fromIndex` is not a number.
     * @throws {RangeError} when `fromIndex` is a non-integer number.
     */
    function indexOfFold(text, search, fromIndex) {
        requireString(text, "text");
        requireString(search, "search");
        const from = toIndex(fromIndex, text.length, 0);
        if (search.length === 0) return from;
        let offset = codepointStart(text, from);
        if (offset >= text.length) return -1;
        const pattern = codePointsOf(search);
        const length = pattern.length;
        const prefix = foldPrefix(pattern);
        const starts = new makeUint32Array(length);
        let matched = 0;
        let slot = 0;
        while (offset < text.length) {
            const point = codePointAt(text, offset);
            starts[slot] = offset;
            slot += 1;
            if (slot === length) slot = 0;
            while (matched > 0 && !foldEqual(pattern[matched], point)) {
                matched = prefix[matched - 1];
            }
            if (foldEqual(pattern[matched], point)) matched += 1;
            // The ring holds the last `length` start offsets. After the
            // increment above, `starts[slot]` is the oldest of them, which is
            // the start of the match that has just completed.
            if (matched === length) return starts[slot];
            offset += codepointWidth(point);
        }
        return -1;
    }

    /**
     * Find the last occurrence of `search` in `text`, ignoring case.
     *
     * Overlapping occurrences are found, like `String.prototype.lastIndexOf`:
     * `lastIndexOfFold("aaa", "aa")` is `1`. A match must start at or before
     * `fromIndex`; the default is the end of the string.
     *
     * Linear in the text and the search string, with the same KMP scan as
     * `indexOfFold`, so a repeated prefix cannot make it quadratic. It scans
     * from the start and stops as soon as no pending or future match can begin
     * at or before `fromIndex`, so a small limit does not read the rest of a
     * long text; extra memory is O(search).
     *
     * @param {string} text
     * @param {string} search
     * @param {number} [fromIndex=text.length] an integer, or
     * `Infinity`/`-Infinity`
     * @returns {number} a UTF-16 code unit index, or `-1`
     * @throws {TypeError} when `text` or `search` is not a string, or
     * `fromIndex` is not a number.
     * @throws {RangeError} when `fromIndex` is a non-integer number.
     */
    function lastIndexOfFold(text, search, fromIndex) {
        requireString(text, "text");
        requireString(search, "search");
        const limit = toIndex(fromIndex, text.length, text.length);
        if (search.length === 0) return limit;
        const pattern = codePointsOf(search);
        const length = pattern.length;
        const prefix = foldPrefix(pattern);
        const starts = new makeUint32Array(length);
        let last = -1;
        let matched = 0;
        let offset = 0;
        let slot = 0;
        while (offset < text.length) {
            const point = codePointAt(text, offset);
            const width = codepointWidth(point);
            starts[slot] = offset;
            slot += 1;
            if (slot === length) slot = 0;
            while (matched > 0 && !foldEqual(pattern[matched], point)) {
                matched = prefix[matched - 1];
            }
            if (foldEqual(pattern[matched], point)) matched += 1;
            if (matched === length) {
                const start = starts[slot];
                if (start > limit) break;
                last = start;
                matched = prefix[matched - 1];
            }
            // The earliest match that can still complete begins at the start
            // of the surviving partial match, or at the next code point when
            // no partial match survives. Start offsets only increase with the
            // scan, so once that candidate is past the limit no match can be
            // recorded and the rest of the text need not be read.
            if (matched > 0) {
                const candidate = (slot + length - matched) % length;
                if (starts[candidate] > limit) break;
            } else if (offset + width > limit) {
                break;
            }
            offset += width;
        }
        return last;
    }

    function codePointSet(chars) {
        const set = new makeSet();
        for (let index = 0; index < chars.length; ) {
            const point = codePointAt(chars, index);
            setAdd(set, point);
            index += codepointWidth(point);
        }
        return set;
    }

    function trimStartOffset(text, set) {
        let start = 0;
        while (start < text.length) {
            const point = codePointAt(text, start);
            if (!setHas(set, point)) break;
            start += codepointWidth(point);
        }
        return start;
    }

    function trimEndOffset(text, set, floor) {
        let end = text.length;
        while (end > floor) {
            const start = previousCodepointStart(text, end);
            const point = codePointAt(text, start);
            if (!setHas(set, point)) break;
            end = start;
        }
        return end;
    }

    /**
     * Remove leading and trailing code points that occur in `chars`.
     *
     * With no `chars`, this is exactly `String.prototype.trim()`, including
     * JavaScript's whitespace set (which contains U+FEFF and excludes U+0085).
     * This is deliberately not `neo:chars`' `isSpace`, which follows Go's
     * `unicode.IsSpace`.
     *
     * @param {string} text
     * @param {string} [chars] a set of Unicode code points, not a prefix
     * @returns {string}
     * @throws {TypeError} when `text` or `chars` is not a string.
     */
    function trim(text, chars) {
        requireString(text, "text");
        if (chars === undefined) return stringTrim(text);
        requireString(chars, "chars");
        if (chars.length === 0) return text;
        const set = codePointSet(chars);
        const start = trimStartOffset(text, set);
        if (start === text.length) return "";
        const end = trimEndOffset(text, set, start);
        return stringSlice(text, start, end);
    }

    /**
     * Remove leading code points that occur in `chars`.
     *
     * @param {string} text
     * @param {string} [chars] a set of Unicode code points, not a prefix
     * @returns {string}
     * @throws {TypeError} when `text` or `chars` is not a string.
     */
    function trimStart(text, chars) {
        requireString(text, "text");
        if (chars === undefined) return stringTrimStart(text);
        requireString(chars, "chars");
        if (chars.length === 0) return text;
        const set = codePointSet(chars);
        const start = trimStartOffset(text, set);
        return start === 0 ? text : stringSlice(text, start);
    }

    /**
     * Remove trailing code points that occur in `chars`.
     *
     * @param {string} text
     * @param {string} [chars] a set of Unicode code points, not a suffix
     * @returns {string}
     * @throws {TypeError} when `text` or `chars` is not a string.
     */
    function trimEnd(text, chars) {
        requireString(text, "text");
        if (chars === undefined) return stringTrimEnd(text);
        requireString(chars, "chars");
        if (chars.length === 0) return text;
        const set = codePointSet(chars);
        const end = trimEndOffset(text, set, 0);
        return end === text.length ? text : stringSlice(text, 0, end);
    }

    // A frozen module object, like `chars`: the properties cannot be replaced,
    // but the function objects themselves are shared with the imports.
    const strings = Object.freeze({
        equalFold,
        startsWithFold,
        endsWithFold,
        indexOfFold,
        lastIndexOfFold,
        trim,
        trimStart,
        trimEnd,
    });

    // The host ops for `crypto` are captured now, before user code runs. Later
    // replacement of `Deno.core.ops` therefore does not change what the module
    // calls. This is not isolation from code that replaced them before this
    // script ran, and it is not a sandbox.
    const cryptoOps = Deno.core.ops;
    const opGenerateEnvelopeKey = cryptoOps.op_neo_crypto_generate_envelope_key;
    const opRandomBytes = cryptoOps.op_neo_crypto_random_bytes;
    const opEncrypt = cryptoOps.op_neo_crypto_encrypt;
    const opDecrypt = cryptoOps.op_neo_crypto_decrypt;
    const opCipherCreate = cryptoOps.op_neo_crypto_cipher_create;
    const opCipherUpdate = cryptoOps.op_neo_crypto_cipher_update;
    const opCipherFinal = cryptoOps.op_neo_crypto_cipher_final;
    const opCipherSetAad = cryptoOps.op_neo_crypto_cipher_set_aad;
    const opCipherSetTag = cryptoOps.op_neo_crypto_cipher_set_tag;
    const opCipherGetTag = cryptoOps.op_neo_crypto_cipher_get_tag;
    const opCipherSetPadding = cryptoOps.op_neo_crypto_cipher_set_padding;
    const opCipherRelease = cryptoOps.op_neo_crypto_cipher_release;
    const opHashCreate = cryptoOps.op_neo_crypto_hash_create;
    const opHmacCreate = cryptoOps.op_neo_crypto_hmac_create;
    const opHashRelease = cryptoOps.op_neo_crypto_hash_release;
    const opHashUpdate = cryptoOps.op_neo_crypto_hash_update;
    const opHashDigest = cryptoOps.op_neo_crypto_hash_digest;
    const opAsyncSubmit = cryptoOps.op_neo_crypto_async_submit;
    const opAsyncJoin = cryptoOps.op_neo_crypto_async_join;
    const opPbkdf2 = cryptoOps.op_neo_crypto_pbkdf2;
    const opPbkdf2Async = cryptoOps.op_neo_crypto_pbkdf2_async_submit;
    const opPasswordJoin = cryptoOps.op_neo_crypto_password_async_join;
    const opBcryptHash = cryptoOps.op_neo_crypto_bcrypt_hash;
    const opBcryptHashAsync = cryptoOps.op_neo_crypto_bcrypt_hash_async_submit;
    const opBcryptVerify = cryptoOps.op_neo_crypto_bcrypt_verify;
    const opBcryptVerifyAsync = cryptoOps.op_neo_crypto_bcrypt_verify_async_submit;

    // The age host ops are captured here for the same reason: replacing
    // `Deno.core.ops` later does not change what the age module calls. The ops
    // are synchronous and return a result object; the wrapper below throws.
    const ageOps = Deno.core.ops;
    const opAgeGenerateIdentity = ageOps.op_neo_age_generate_identity;
    const opAgeIdentityToRecipient = ageOps.op_neo_age_identity_to_recipient;
    const opAgeEncrypt = ageOps.op_neo_age_encrypt;
    const opAgeDecrypt = ageOps.op_neo_age_decrypt;

    // The stdio host ops are captured for the same reason: replacing
    // `Deno.core.ops` later does not change what the classes call. They
    // validate views, copy byte ranges natively, and write to the current
    // process's standard streams; they do not grant filesystem, process,
    // environment, or stdin access.
    const stdioOps = Deno.core.ops;
    const opStdioCopy = stdioOps.op_neo_stdio_copy;
    const opStdioViewLength = stdioOps.op_neo_stdio_view_length;
    const opStdioSlice = stdioOps.op_neo_stdio_slice;
    const opStdioWrite = stdioOps.op_neo_stdio_write;

    // The ssh host ops are captured here for the same reason: replacing
    // `Deno.core.ops` later does not change what the module calls. They dial
    // one host, verify an explicit host-key pin or known-hosts document,
    // authenticate, and run one command at a time. They add no filesystem,
    // environment, agent, or stdin access.
    const sshOps = Deno.core.ops;
    const opSshConnect = sshOps.op_neo_ssh_connect;
    const opSshExec = sshOps.op_neo_ssh_exec;
    const opSshClose = sshOps.op_neo_ssh_close;

    /**
     * Throw the exception described by a failed host result.
     *
     * Argument errors are `TypeError` or `RangeError` and carry no code. Other
     * failures are `Error`; `code` is defined as an own property instead of
     * assigned, so an inherited `Object.prototype.code` accessor cannot swallow
     * it. The host result object has a null prototype, so its properties cannot
     * be intercepted either.
     *
     * @param {{ok: boolean, name?: string, code?: string, message?: string}} response
     * @returns {never}
     */
    function throwCrypto(response) {
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        if (response.name === "RangeError") {
            throw new makeRangeError(response.message);
        }
        const error = new makeError(response.message);
        defineProperty(error, "code", {
            __proto__: null,
            value: response.code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        throw error;
    }

    /**
     * Turn the host result object into a value or the documented exception.
     *
     * @param {{ok: boolean, value?: Uint8Array, name?: string, code?: string, message?: string}} response
     * @returns {Uint8Array}
     */
    function unwrapCrypto(response) {
        if (response.ok) return response.value;
        throwCrypto(response);
    }

    /**
     * Turn a host result object that carries a resource id into the id.
     *
     * @param {{ok: boolean, id?: number, name?: string, code?: string, message?: string}} response
     * @returns {number}
     */
    function unwrapId(response) {
        if (response.ok) return response.id;
        throwCrypto(response);
    }

    /** Throw unless a host result object reports success. */
    function unwrapUnit(response) {
        if (!response.ok) throwCrypto(response);
    }

    /**
     * The ordinary `Error` this module reports for hash lifecycle and worker
     * failures, with a stable own `code`.
     */
    function hashError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /**
     * Generate a 32-byte AES-256 key from the operating system CSPRNG.
     *
     * This is the envelope key generator. `generateKey` is not exported in this
     * slice: that name is reserved for a future Node-compatible surface.
     *
     * Keep the key secret, use a distinct key per purpose, and retire it well
     * before about 2^32 messages under it (the AES-GCM nonce-collision risk).
     * The module stores no keys and does not promise zeroization.
     *
     * @returns {Uint8Array}
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `NEO_CRYPTO_RANDOM` when the CSPRNG fails.
     */
    function generateEnvelopeKey() {
        if (arguments.length !== 0) {
            throw new makeTypeError("generateEnvelopeKey takes no arguments");
        }
        return unwrapCrypto(opGenerateEnvelopeKey());
    }

    /**
     * Return `length` bytes from the operating system CSPRNG.
     *
     * For salts, IVs, or nonces used by another construction. This is the same
     * CSPRNG source as `generateEnvelopeKey`; there is no weaker fallback, and
     * a failure throws rather than returning predictable bytes. The call is
     * synchronous. It is not a statistical test of the output, and it does not
     * prove cryptographic security.
     *
     * There is no callback form in this slice and none is reserved:
     * asynchronous APIs will be `Promise`-based. Passing a second argument
     * throws instead of silently ignoring it.
     *
     * @param {number} length a non-negative integer, at most 8 MiB
     * @returns {Uint8Array}
     * @throws {TypeError} when `length` is not a number or the argument count
     * is not exactly one.
     * @throws {RangeError} when `length` is not an integer or is outside the
     * documented range.
     * @throws {Error} with `code` `NEO_CRYPTO_RANDOM` when the OS CSPRNG fails.
     */
    function randomBytes(length) {
        if (arguments.length !== 1) {
            throw new makeTypeError("randomBytes takes exactly one argument");
        }
        return unwrapCrypto(opRandomBytes(length));
    }

    /**
     * Encrypt `plaintext` into a fresh versioned envelope.
     *
     * The nonce is drawn fresh for every call and the header, nonce, and
     * `associatedData` are authenticated. Synchronous; every argument and the
     * result are `Uint8Array`s.
     *
     * Limits: `plaintext` is limited to 8 MiB, `associatedData` to 8 MiB, and
     * the returned envelope to 8 MiB + 34 bytes. These are neojs limits, not
     * intrinsic AES-GCM limits.
     *
     * Security: a fresh random 96-bit nonce is used per call. Under one key the
     * nonce-collision probability reaches about 2^-32 after roughly 2^32
     * encryptions, so retire the key well before that. Never reuse a key with a
     * repeated nonce. The module does not track key use or promise zeroization.
     *
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} plaintext
     * @param {Uint8Array} [associatedData] authenticated but not encrypted
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument is not a `Uint8Array` or the
     * argument count is not two or three.
     * @throws {Error} with `code` `NEO_CRYPTO_KEY`, `NEO_CRYPTO_RANDOM`,
     * `NEO_CRYPTO_LIMIT`, or `NEO_CRYPTO_DETACHED`.
     */
    function encrypt(key, plaintext, associatedData) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("encrypt takes two or three arguments");
        }
        return unwrapCrypto(opEncrypt(key, plaintext, associatedData));
    }

    /**
     * Verify and decrypt one envelope.
     *
     * The whole header and nonce, the associated data, and the ciphertext are
     * authenticated. No plaintext is returned when the tag does not verify.
     *
     * Limits: the envelope is limited to 8 MiB + 34 bytes and `associatedData`
     * to 8 MiB. These are neojs limits, not intrinsic AES-GCM limits.
     *
     * Security: a failed tag check returns no plaintext. AES-GCM needs unique
     * nonces under one key, so retire a key well before about 2^32 messages
     * under it. This module does not track or enforce key use, and it does not
     * promise zeroization.
     *
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} envelope
     * @param {Uint8Array} [associatedData] must match the value used to encrypt
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument is not a `Uint8Array` or the
     * argument count is not two or three.
     * @throws {Error} with `code` `NEO_CRYPTO_AUTH`, `NEO_CRYPTO_FORMAT`,
     * `NEO_CRYPTO_KEY`, `NEO_CRYPTO_LIMIT`, or `NEO_CRYPTO_DETACHED`.
     */
    function decrypt(key, envelope, associatedData) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("decrypt takes two or three arguments");
        }
        return unwrapCrypto(opDecrypt(key, envelope, associatedData));
    }

    // Stateful AES-256-CBC and AES-256-GCM. See
    // `neojs-core/src/crypto_cipher.rs`. `update` buffers a bounded amount of
    // input and returns an empty Uint8Array; `final` performs the operation and
    // returns the output. GCM never releases plaintext before its tag check.

    const cipherToken = Symbol("neojs.crypto.Cipher");

    /**
     * Normalize an algorithm name without exposing an argument-coercion hook.
     *
     * @param {string} algorithm
     * @returns {string} the lowercased name, which the host op still validates
     */
    function cipherAlgorithmName(algorithm) {
        if (typeof algorithm !== "string") {
            throw new makeTypeError("algorithm must be a string");
        }
        return lowerCase(algorithm);
    }

    /**
     * A bounded stateful cipher created by `createEncryptor` or
     * `createDecryptor`.
     *
     * `update(data[, encoding])` copies bytes or a UTF-8 string into the
     * object's bounded buffer and returns an empty `Uint8Array`. `final()`
     * performs the operation over the whole buffered message and marks the
     * object finalized. For GCM encryption, `getAuthTag()` is available only
     * after `final()`. For GCM decryption, `setAuthTag(tag)` is required before
     * `final()`, and `final()` returns no plaintext when the tag does not
     * verify. A closed object's methods throw `NEO_CRYPTO_CLOSED`; an operation
     * after `final()` throws `NEO_CRYPTO_STATE`.
     *
     * Always call `close()`/`dispose()` in a `finally` block, even after a
     * successful `final()` or a thrown `final()`. Finalization releases the
     * buffered message, but the object keeps its native slot until it is closed
     * or the runtime is dropped. The per-runtime open-object and aggregate
     * buffer caps make explicit disposal mandatory for long-lived code.
     *
     * This is not Node's `Cipher` class and does not promise Node's chunk
     * timing, output encodings, or callback form.
     */
    class Cipher {
        #id;
        #encrypt;
        #gcm;
        #closed = false;
        #finalized = false;

        constructor(token, id, encrypt, gcm) {
            if (token !== cipherToken) {
                throw new makeTypeError(
                    "Cipher objects cannot be constructed directly; use createEncryptor() or createDecryptor()",
                );
            }
            this.#id = id;
            this.#encrypt = encrypt;
            this.#gcm = gcm;
        }

        #assertOpen() {
            if (this.#closed) {
                throw hashError("NEO_CRYPTO_CLOSED", "the cipher object is closed");
            }
        }

        #assertBuffering() {
            this.#assertOpen();
            if (this.#finalized) {
                throw hashError("NEO_CRYPTO_STATE", "the cipher has already been finalized");
            }
        }

        /**
         * Buffer bytes or a UTF-8 string for the cipher operation.
         *
         * @param {Uint8Array | string} data
         * @param {string} [encoding] only `utf8`/`utf-8`, and only for a string
         * @returns {Uint8Array} always empty in this buffered slice
         * @throws {TypeError} for a wrong argument count or type, an encoding
         * supplied with binary input, or an unsupported input encoding.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`, `NEO_CRYPTO_STATE`,
         * `NEO_CRYPTO_LIMIT`, or `NEO_CRYPTO_DETACHED`.
         */
        update(data, encoding) {
            if (arguments.length < 1 || arguments.length > 2) {
                throw new makeTypeError("update takes one or two arguments");
            }
            this.#assertBuffering();
            let code;
            if (typeof data === "string") {
                code = inputEncodingCode(encoding);
            } else {
                if (encoding !== undefined) {
                    throw new makeTypeError("encoding must not be supplied for binary input");
                }
                code = undefined;
            }
            unwrapUnit(opCipherUpdate(this.#id, data, code));
            return new makeUint8Array(0);
        }

        /**
         * Finish the operation and return its output.
         *
         * For CBC this verifies and removes PKCS#7 padding, or checks the
         * no-padding block length. For GCM decryption this checks the
         * authentication tag before returning any plaintext.
         *
         * @returns {Uint8Array} ciphertext or plaintext
         * @throws {TypeError} when any argument is supplied.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`, `NEO_CRYPTO_STATE`,
         * `NEO_CRYPTO_AUTH`, `NEO_CRYPTO_PADDING`, `NEO_CRYPTO_TAG`, or
         * `NEO_CRYPTO_LIMIT`.
         */
        final() {
            if (arguments.length !== 0) {
                throw new makeTypeError("final takes no arguments");
            }
            this.#assertBuffering();
            this.#finalized = true;
            return unwrapCrypto(opCipherFinal(this.#id));
        }

        /**
         * Set GCM associated data, at most once and before `update`/`final`.
         *
         * @param {Uint8Array} aad authenticated but not encrypted
         * @returns {Cipher} this object, for chaining
         * @throws {TypeError} for a wrong argument count, a non-`Uint8Array`,
         * or a detached view.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`, `NEO_CRYPTO_STATE`,
         * `NEO_CRYPTO_UNSUPPORTED`, or `NEO_CRYPTO_LIMIT`.
         */
        setAAD(aad) {
            if (arguments.length !== 1) {
                throw new makeTypeError("setAAD takes exactly one argument");
            }
            this.#assertBuffering();
            if (!this.#gcm) {
                throw hashError(
                    "NEO_CRYPTO_UNSUPPORTED",
                    "associated data is only supported for aes-256-gcm",
                );
            }
            unwrapUnit(opCipherSetAad(this.#id, aad));
            return this;
        }

        /**
         * Set the expected 16-byte GCM authentication tag before `final`.
         *
         * @param {Uint8Array} tag exactly 16 bytes
         * @returns {Cipher} this object, for chaining
         * @throws {TypeError} for a wrong argument count or a non-`Uint8Array`.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`, `NEO_CRYPTO_STATE`,
         * `NEO_CRYPTO_UNSUPPORTED`, or `NEO_CRYPTO_TAG`.
         */
        setAuthTag(tag) {
            if (arguments.length !== 1) {
                throw new makeTypeError("setAuthTag takes exactly one argument");
            }
            this.#assertBuffering();
            if (!this.#gcm || this.#encrypt) {
                throw hashError(
                    "NEO_CRYPTO_UNSUPPORTED",
                    "setAuthTag is only supported on aes-256-gcm decryptors",
                );
            }
            unwrapUnit(opCipherSetTag(this.#id, tag));
            return this;
        }

        /**
         * Return the 16-byte GCM tag produced by `final`.
         *
         * @returns {Uint8Array}
         * @throws {TypeError} when any argument is supplied.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`, `NEO_CRYPTO_STATE`,
         * or `NEO_CRYPTO_UNSUPPORTED`.
         */
        getAuthTag() {
            if (arguments.length !== 0) {
                throw new makeTypeError("getAuthTag takes no arguments");
            }
            this.#assertOpen();
            if (!this.#gcm || !this.#encrypt) {
                throw hashError(
                    "NEO_CRYPTO_UNSUPPORTED",
                    "getAuthTag is only supported on aes-256-gcm encryptors",
                );
            }
            if (!this.#finalized) {
                throw hashError("NEO_CRYPTO_STATE", "final must be called before getAuthTag");
            }
            return unwrapCrypto(opCipherGetTag(this.#id));
        }

        /**
         * Enable or disable CBC PKCS#7 padding before `final`.
         *
         * With padding disabled, the plaintext or ciphertext must already be a
         * multiple of 16 bytes.
         *
         * @param {boolean} enabled
         * @returns {Cipher} this object, for chaining
         * @throws {TypeError} for a wrong argument count or a non-boolean.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`, `NEO_CRYPTO_STATE`,
         * or `NEO_CRYPTO_UNSUPPORTED`.
         */
        setAutoPadding(enabled) {
            if (arguments.length !== 1) {
                throw new makeTypeError("setAutoPadding takes exactly one argument");
            }
            this.#assertBuffering();
            if (this.#gcm) {
                throw hashError(
                    "NEO_CRYPTO_UNSUPPORTED",
                    "padding is not supported for aes-256-gcm",
                );
            }
            if (typeof enabled !== "boolean") {
                throw new makeTypeError("setAutoPadding expects a boolean");
            }
            unwrapUnit(opCipherSetPadding(this.#id, enabled));
            return this;
        }

        /**
         * Release this object's native state.
         *
         * Idempotent. A later method throws `NEO_CRYPTO_CLOSED`. Closing is the
         * deterministic way to stay below the per-runtime open-object and
         * buffer caps.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            opCipherRelease(this.#id);
        }

        /**
         * Alias for `close()`, for explicit-disposal call sites.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        dispose() {
            if (arguments.length !== 0) {
                throw new makeTypeError("dispose takes no arguments");
            }
            this.close();
        }
    }

    /**
     * Create a bounded stateful encryptor for AES-256-CBC or AES-256-GCM.
     *
     * The caller supplies the IV/nonce and must keep it unique: never reuse a
     * GCM nonce under one key, and use an unpredictable CBC IV per key. This
     * module does not generate, count, or track IVs.
     *
     * Always call `close()`/`dispose()` in a `finally` block, even after
     * `final()` or a thrown `final()`. Finalization frees the message buffer
     * but the object keeps its native slot until it is closed or the runtime is
     * dropped, so explicit disposal is required to stay below the per-runtime
     * open-object cap and to release the object deterministically.
     *
     * `options` must be `undefined` or a plain or null-prototype non-Proxy
     * record whose own properties are enumerable data properties with string
     * keys. Validation is own-only: own symbols, accessors, and non-enumerable
     * properties are rejected, and a custom prototype is rejected. Inherited
     * properties on the intrinsic `Object.prototype` or a null prototype are
     * ignored, including a polluted `Object.prototype.padding`. Arrays, class
     * instances, functions, and Proxies throw `TypeError` before a resource is
     * created. CBC accepts only `padding`; GCM accepts none.
     *
     * @param {"aes-256-cbc" | "aes-256-gcm"} algorithm case-insensitive
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} iv 16 bytes for CBC, 12 bytes for GCM
     * @param {{padding?: boolean}} [options] CBC only; no other key is accepted
     * @returns {Cipher}
     * @throws {TypeError} for a wrong argument count or an unsupported option.
     * @throws {Error} with `code` `NEO_CRYPTO_ALGORITHM`, `NEO_CRYPTO_KEY`,
     * `NEO_CRYPTO_IV`, `NEO_CRYPTO_DETACHED`, or
     * `NEO_CRYPTO_TOO_MANY_OBJECTS`.
     */
    function createEncryptor(algorithm, key, iv, options) {
        if (arguments.length < 3 || arguments.length > 4) {
            throw new makeTypeError("createEncryptor takes three or four arguments");
        }
        const name = cipherAlgorithmName(algorithm);
        const id = unwrapId(opCipherCreate(0, name, key, iv, options));
        return new Cipher(cipherToken, id, true, name === "aes-256-gcm");
    }

    /**
     * Create a bounded stateful decryptor for AES-256-CBC or AES-256-GCM.
     *
     * GCM decryptors require `setAuthTag(tag)` before `final()`. No plaintext
     * is returned from `update`, so an unauthenticated message cannot leak
     * plaintext before the tag check. CBC decryption is unauthenticated; if it
     * is needed for interoperability, verify an independent encrypt-then-MAC
     * tag over the IV and ciphertext before calling this object, and do not
     * expose padding validity as an oracle.
     *
     * Always call `close()`/`dispose()` in a `finally` block, even after
     * `final()` or a thrown `final()`. Finalization frees the message buffer
     * but the object keeps its native slot until it is closed or the runtime is
     * dropped, so explicit disposal is required to stay below the per-runtime
     * open-object cap and to release the object deterministically.
     *
     * `options` must be `undefined` or a plain or null-prototype non-Proxy
     * record whose own properties are enumerable data properties with string
     * keys. Validation is own-only: own symbols, accessors, and non-enumerable
     * properties are rejected, and a custom prototype is rejected. Inherited
     * properties on the intrinsic `Object.prototype` or a null prototype are
     * ignored, including a polluted `Object.prototype.padding`. Arrays, class
     * instances, functions, and Proxies throw `TypeError` before a resource is
     * created. CBC accepts only `padding`; GCM accepts none.
     *
     * @param {"aes-256-cbc" | "aes-256-gcm"} algorithm case-insensitive
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} iv 16 bytes for CBC, 12 bytes for GCM
     * @param {{padding?: boolean}} [options] CBC only; no other key is accepted
     * @returns {Cipher}
     * @throws {TypeError} for a wrong argument count or an unsupported option.
     * @throws {Error} with `code` `NEO_CRYPTO_ALGORITHM`, `NEO_CRYPTO_KEY`,
     * `NEO_CRYPTO_IV`, `NEO_CRYPTO_DETACHED`, or
     * `NEO_CRYPTO_TOO_MANY_OBJECTS`.
     */
    function createDecryptor(algorithm, key, iv, options) {
        if (arguments.length < 3 || arguments.length > 4) {
            throw new makeTypeError("createDecryptor takes three or four arguments");
        }
        const name = cipherAlgorithmName(algorithm);
        const id = unwrapId(opCipherCreate(1, name, key, iv, options));
        return new Cipher(cipherToken, id, false, name === "aes-256-gcm");
    }

    // Hash and HMAC. The synchronous factories return an incremental object
    // with `update` and `digest`. The asynchronous helpers are one-shot and
    // return a Promise resolved by a bounded worker pool; they are not
    // Promise-wrapped blocking calls. See `neojs-core/src/crypto_hash.rs`.

    const hashToken = Symbol("neojs.crypto.Hash");

    /**
     * Return the numeric code for a digest output encoding.
     *
     * The accepted names are `hex`, `base64`, `base64url`, `latin1`, and the
     * Node alias `binary`. `undefined` means raw bytes. Any other value throws
     * `TypeError`, and the hash state is not consumed.
     *
     * @param {string | undefined} encoding
     * @returns {number} 0 raw, 1 hex, 2 base64, 3 base64url, 4 latin1
     */
    function outputEncodingCode(encoding) {
        if (encoding === undefined) return 0;
        if (typeof encoding !== "string") {
            throw new makeTypeError("encoding must be a string");
        }
        const name = lowerCase(encoding);
        if (name === "hex") return 1;
        if (name === "base64") return 2;
        if (name === "base64url") return 3;
        if (name === "latin1" || name === "binary") return 4;
        throw new makeTypeError("unsupported digest encoding");
    }

    /**
     * Return the numeric code for a string input encoding.
     *
     * Only UTF-8 is accepted, as `utf8` or `utf-8`, case-insensitively, and as
     * the default when the argument is `undefined`. Binary input may not name
     * an encoding at all.
     *
     * @param {string | undefined} encoding
     * @returns {number} always 0
     */
    function inputEncodingCode(encoding) {
        if (encoding === undefined) return 0;
        if (typeof encoding !== "string") {
            throw new makeTypeError("encoding must be a string");
        }
        const name = lowerCase(encoding);
        if (name === "utf8" || name === "utf-8") return 0;
        throw new makeTypeError("unsupported input encoding");
    }

    /** Map byte values 0-255 to one JavaScript code unit each. */
    function latin1String(bytes) {
        let text = "";
        for (let index = 0; index < bytes.length; index += 1) {
            text += fromCharCode(bytes[index]);
        }
        return text;
    }

    /**
     * A synchronous incremental SHA-2 hash or HMAC.
     *
     * Created by `createHash` or `createHmac`, not by user code. `update`
     * copies the current view's bytes or encodes the string as UTF-8, so later
     * mutation of the input does not change the result. `digest` consumes the
     * state; a second `digest` or a later `update` throws
     * `NEO_CRYPTO_DIGESTED`. `close()`/`dispose()` release the state without a
     * digest; later calls throw `NEO_CRYPTO_CLOSED`. Open objects are capped per
     * runtime, so close objects you do not finish.
     */
    class Hash {
        #id;
        #digested = false;
        #closed = false;

        constructor(token, id) {
            if (token !== hashToken) {
                throw new makeTypeError(
                    "Hash objects cannot be constructed directly; use createHash() or createHmac()",
                );
            }
            this.#id = id;
        }

        /**
         * Add bytes or a UTF-8 string to the hash.
         *
         * @param {Uint8Array | string} data
         * @param {string} [encoding] only `utf8`/`utf-8`, and only for a string
         * @returns {Hash} this object, for chaining
         * @throws {TypeError} for a wrong argument count or type, an encoding
         * supplied with binary input, or an unsupported input encoding.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`,
         * `NEO_CRYPTO_DIGESTED`, `NEO_CRYPTO_LIMIT`, or
         * `NEO_CRYPTO_DETACHED`.
         */
        update(data, encoding) {
            if (arguments.length < 1 || arguments.length > 2) {
                throw new makeTypeError("update takes one or two arguments");
            }
            if (this.#closed) {
                throw hashError("NEO_CRYPTO_CLOSED", "the hash object is closed");
            }
            if (this.#digested) {
                throw hashError("NEO_CRYPTO_DIGESTED", "digest has already been called");
            }
            const isText = typeof data === "string";
            if (!isText && encoding !== undefined) {
                throw new makeTypeError("encoding must not be supplied for binary input");
            }
            const code = isText ? inputEncodingCode(encoding) : undefined;
            unwrapUnit(opHashUpdate(this.#id, data, code));
            return this;
        }

        /**
         * Finish the hash and return the digest.
         *
         * `digest()` returns a `Uint8Array`. A string encoding returns a
         * string: lowercase hex, padded standard Base64, unpadded URL-safe
         * Base64, or one code unit per byte for `latin1`/`binary`. An
         * unsupported encoding is rejected before the state is consumed.
         *
         * @param {"hex" | "base64" | "base64url" | "latin1" | "binary"} [encoding]
         * @returns {Uint8Array | string}
         * @throws {TypeError} for a second argument when the count is over one
         * or an unsupported encoding.
         * @throws {Error} with `code` `NEO_CRYPTO_CLOSED`,
         * `NEO_CRYPTO_DIGESTED`, or `NEO_CRYPTO_LIMIT`.
         */
        digest(encoding) {
            if (arguments.length > 1) {
                throw new makeTypeError("digest takes at most one argument");
            }
            if (this.#closed) {
                throw hashError("NEO_CRYPTO_CLOSED", "the hash object is closed");
            }
            if (this.#digested) {
                throw hashError("NEO_CRYPTO_DIGESTED", "digest has already been called");
            }
            const code = outputEncodingCode(encoding);
            this.#digested = true;
            const response = opHashDigest(this.#id, code);
            if (!response.ok) throwCrypto(response);
            return response.value;
        }

        /**
         * Release this object's native state without producing a digest.
         *
         * Idempotent. A later `update` or `digest` throws `NEO_CRYPTO_CLOSED`.
         * Closing is the deterministic way to stay below the per-runtime open
         * object limit; an object that is never closed or digested keeps its
         * slot until the runtime is dropped.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            opHashRelease(this.#id);
        }

        /**
         * Alias for `close()`, for explicit-disposal call sites.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        dispose() {
            if (arguments.length !== 0) {
                throw new makeTypeError("dispose takes no arguments");
            }
            this.close();
        }
    }

    /**
     * Create an incremental SHA-2 hash.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @returns {Hash}
     * @throws {TypeError} when the argument count is not one or the value is
     * not a string.
     * @throws {Error} with `code` `NEO_CRYPTO_ALGORITHM` for another name, or
     * `NEO_CRYPTO_TOO_MANY_OBJECTS` when the per-runtime open-object cap is
     * reached. Close objects you will not digest with `close()`/`dispose()`;
     * an abandoned object counts against the cap until the runtime is dropped.
     */
    function createHash(algorithm) {
        if (arguments.length !== 1) {
            throw new makeTypeError("createHash takes exactly one argument");
        }
        return new Hash(hashToken, unwrapId(opHashCreate(algorithm)));
    }

    /**
     * Create an incremental HMAC.
     *
     * The key is a `Uint8Array` of exact bytes or a UTF-8 string. HMAC accepts
     * a key of any length, including zero. The key is copied now, so mutating
     * the caller's view afterwards does not change the MAC.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} key
     * @returns {Hash}
     * @throws {TypeError} when the argument count is not two or a value has
     * the wrong type.
     * @throws {Error} with `code` `NEO_CRYPTO_ALGORITHM`,
     * `NEO_CRYPTO_TOO_MANY_OBJECTS` when the per-runtime open-object cap is
     * reached, `NEO_CRYPTO_LIMIT`, or `NEO_CRYPTO_DETACHED`. Close objects you
     * will not digest with `close()`/`dispose()`; an abandoned object counts
     * against the cap until the runtime is dropped.
     */
    function createHmac(algorithm, key) {
        if (arguments.length !== 2) {
            throw new makeTypeError("createHmac takes exactly two arguments");
        }
        return new Hash(hashToken, unwrapId(opHmacCreate(algorithm, key)));
    }

    /**
     * Await one asynchronous digest job and decode its bytes.
     *
     * Calling the join op removes the job from the resource table immediately.
     * An unawaited result is retained by this promise, not by the table. The
     * join op resolves to an empty `Uint8Array` only when the worker failed; a
     * real digest is never empty, so that sentinel becomes
     * `NEO_CRYPTO_INTERNAL`.
     */
    async function joinDigest(id, code) {
        const bytes = await opAsyncJoin(id);
        if (bytes.length === 0) {
            throw hashError("NEO_CRYPTO_INTERNAL", "the hash worker failed");
        }
        return code === 0 ? bytes : latin1String(bytes);
    }

    /**
     * Hash one input on the runtime's bounded worker pool.
     *
     * Genuinely asynchronous: validation, budget admission, and a single bounded
     * copy of `data` into a module-owned buffer run before the returned Promise
     * exists, then all hashing runs on a blocking worker and the Promise
     * resolves on the isolate thread.
     * It does not wrap a blocking call in `Promise.resolve`. The input is copied
     * before the promise is returned, so mutating it afterwards does not change
     * the digest. The join starts immediately: ignoring the returned Promise
     * does not cancel the work, and dropping only the `run_file_async` entry
     * future does not either. Only runtime teardown aborts a queued job; a
     * running job cannot be stopped. At most 64 jobs, even empty ones, and 64 MiB of
     * copied data and HMAC key bytes may be pending; a submission over either
     * bound rejects with `NEO_CRYPTO_BUSY`.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} data a string is encoded as UTF-8
     * @param {"hex" | "base64" | "base64url" | "latin1" | "binary"} [encoding]
     * @returns {Promise<Uint8Array | string>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function hashAsync(algorithm, data, encoding) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("hashAsync takes two or three arguments");
        }
        const code = outputEncodingCode(encoding);
        const id = unwrapId(opAsyncSubmit(algorithm, 0, undefined, data, code));
        return joinDigest(id, code);
    }

    /**
     * Compute an HMAC on the runtime's bounded worker pool.
     *
     * The asynchronous form of `createHmac(algorithm, key).update(data).digest
     * (encoding)`. The key and data are copied before the promise is returned.
     * The join starts immediately; ignoring the returned Promise does not
     * cancel the work. Only runtime teardown aborts a queued job; a running job
     * cannot be stopped.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} key a string is encoded as UTF-8
     * @param {Uint8Array | string} data a string is encoded as UTF-8
     * @param {"hex" | "base64" | "base64url" | "latin1" | "binary"} [encoding]
     * @returns {Promise<Uint8Array | string>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function hmacAsync(algorithm, key, data, encoding) {
        if (arguments.length < 3 || arguments.length > 4) {
            throw new makeTypeError("hmacAsync takes three or four arguments");
        }
        const code = outputEncodingCode(encoding);
        const id = unwrapId(opAsyncSubmit(algorithm, 1, key, data, code));
        return joinDigest(id, code);
    }

    // PBKDF2 and bcrypt. These share the hash worker's bounded executor and
    // pending-job budget. The sync forms block the isolate; the async forms
    // return a Promise resolved by the same blocking pool. See
    // `neojs-core/src/crypto_hash/password.rs`.

    /**
     * Await one asynchronous PBKDF2 or bcrypt job and decode its bytes.
     *
     * Calling the join op removes the job from the resource table immediately.
     * An empty result is the worker-failure sentinel. `kind` is 0 for raw
     * PBKDF2 bytes, 1 for an ASCII bcrypt hash, and 2 for a verification
     * boolean encoded as one byte.
     */
    async function joinPassword(id, kind) {
        const bytes = await opPasswordJoin(id);
        // An empty buffer means the join/resource was missing or the worker
        // panicked. A tagged buffer carries either success or a stable worker
        // failure category, so a failed CSPRNG draw is not flattened into an
        // internal error.
        if (bytes.length === 0) {
            throw hashError(
                "NEO_CRYPTO_INTERNAL",
                "the password hashing worker failed",
            );
        }
        const tag = bytes[0];
        if (tag === 1) {
            throw hashError(
                "NEO_CRYPTO_RANDOM",
                "operating system random number generator failed",
            );
        }
        if (tag === 2) {
            throw hashError(
                "NEO_CRYPTO_LIMIT",
                "input exceeds the password hashing size limit",
            );
        }
        if (tag !== 0) {
            throw hashError(
                "NEO_CRYPTO_INTERNAL",
                "the password hashing worker failed",
            );
        }
        if (kind === 1) {
            // The bcrypt hash is ASCII. Build the string from the payload
            // without slicing the host buffer through a mutable prototype.
            let text = "";
            for (let index = 1; index < bytes.length; index += 1) {
                text += fromCharCode(bytes[index]);
            }
            return text;
        }
        if (kind === 2) return bytes[1] !== 0;
        // PBKDF2 output. Copy the payload without `subarray`/`slice`, whose
        // species lookup could run user JavaScript.
        const output = new makeUint8Array(bytes.length - 1);
        for (let index = 1; index < bytes.length; index += 1) {
            output[index - 1] = bytes[index];
        }
        return output;
    }

    /**
     * Derive key material with PBKDF2.
     *
     * Synchronous: it blocks the isolate for the whole derivation. Use
     * `pbkdf2Async` to keep the event loop moving. `password` and `salt` are
     * exact `Uint8Array` bytes or strings encoded as UTF-8. This is not Node's
     * `crypto.pbkdf2Sync` and does not accept a digest name such as `sha1`.
     *
     * Limits: `iterations` is 1 through 1,000,000, `length` is 1 through 1024
     * bytes, and one password or salt is at most 1 MiB. These are neojs limits,
     * not PBKDF2 limits. A string is measured by its UTF-8 length before it is
     * copied.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} password
     * @param {Uint8Array | string} salt
     * @param {number} iterations integer 1 through 1,000,000
     * @param {number} length integer 1 through 1024
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly five.
     * @throws {RangeError} when `iterations` or `length` is outside its range.
     * @throws {Error} with `code` `NEO_CRYPTO_ALGORITHM`,
     * `NEO_CRYPTO_LIMIT`, or `NEO_CRYPTO_DETACHED`.
     */
    function pbkdf2(algorithm, password, salt, iterations, length) {
        if (arguments.length !== 5) {
            throw new makeTypeError("pbkdf2 takes exactly five arguments");
        }
        return unwrapCrypto(opPbkdf2(algorithm, password, salt, iterations, length));
    }

    /**
     * Derive key material with PBKDF2 on the runtime's bounded worker pool.
     *
     * Genuinely asynchronous: validation, budget admission, and one bounded
     * copy into a module-owned zeroizing buffer run on the isolate thread, then
     * the derivation runs on a blocking worker. It is not a
     * resolved-`Promise` wrapper around blocking work. The
     * inputs are copied before the promise is returned. The join starts
     * immediately; ignoring the returned Promise does not cancel the work, and
     * dropping only the `run_file_async` entry future does not either. Only
     * runtime teardown aborts a queued job; a running job cannot be stopped and
     * stays charged until it ends.
     *
     * Limits are the same as `pbkdf2`. PBKDF2 and bcrypt share the hash
     * worker's bounds: at most 64 jobs and 64 MiB of copied input may be
     * pending. A submission over either bound rejects with
     * `NEO_CRYPTO_BUSY`.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} password
     * @param {Uint8Array | string} salt
     * @param {number} iterations integer 1 through 1,000,000
     * @param {number} length integer 1 through 1024
     * @returns {Promise<Uint8Array>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function pbkdf2Async(algorithm, password, salt, iterations, length) {
        if (arguments.length !== 5) {
            throw new makeTypeError("pbkdf2Async takes exactly five arguments");
        }
        const id = unwrapId(
            opPbkdf2Async(algorithm, password, salt, iterations, length),
        );
        return joinPassword(id, 0);
    }

    /**
     * Hash a password with bcrypt.
     *
     * Returns a 60-character modular-crypt string in the `$2b$` format. The
     * salt is 16 bytes from the operating system CSPRNG. The password is exact
     * bytes or a UTF-8 string; at most 72 bytes are accepted, and exactly 72
     * bytes are hashed as 72 bytes with no added NUL. A longer password throws
     * `NEO_CRYPTO_LIMIT`; it is never silently truncated.
     *
     * `cost` is the logarithmic work factor, an integer from 4 through 16,
     * defaulting to 10. The upper bound is a neojs policy that keeps a single
     * call bounded; it is deliberately lower than bcrypt's format maximum.
     *
     * This is synchronous and blocks the isolate. Use `bcryptHashAsync` to
     * keep the event loop moving.
     *
     * @param {Uint8Array | string} password
     * @param {number} [cost=10] integer 4 through 16
     * @returns {string}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not one or two.
     * @throws {RangeError} when `cost` is not an integer in its range.
     * @throws {Error} with `code` `NEO_CRYPTO_LIMIT`, `NEO_CRYPTO_RANDOM`, or
     * `NEO_CRYPTO_DETACHED`.
     */
    function bcryptHash(password, cost) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("bcryptHash takes one or two arguments");
        }
        return unwrapCrypto(opBcryptHash(password, cost));
    }

    /**
     * Hash a password with bcrypt on the runtime's bounded worker pool.
     *
     * The asynchronous form of `bcryptHash`, with the same password and cost
     * bounds. Argument validation, budget admission, and the copy run before
     * the promise is returned; the password is hashed on a blocking worker. The
     * join starts immediately; ignoring the returned Promise does not cancel
     * the work. Only runtime teardown aborts a queued job; a running job cannot
     * be stopped.
     *
     * @param {Uint8Array | string} password
     * @param {number} [cost=10] integer 4 through 16
     * @returns {Promise<string>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function bcryptHashAsync(password, cost) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("bcryptHashAsync takes one or two arguments");
        }
        const id = unwrapId(opBcryptHashAsync(password, cost));
        return joinPassword(id, 1);
    }

    /**
     * Verify a password against a bcrypt modular-crypt string.
     *
     * Accepts `$2a$`, `$2b$`, and `$2y$` hashes. A malformed string, an
     * unsupported version (`$2x$` included), or an encoded cost outside 4
     * through 16 returns `false` without computing a hash, so a malicious hash
     * cannot force an unbounded work factor. A password longer than 72 bytes
     * also returns `false`; it is not an error and is never truncated to a
     * partial match.
     *
     * The comparison inside the bcrypt primitive is constant-time. This is
     * synchronous and blocks the isolate for the work factor in `hash`; use
     * `bcryptVerifyAsync` to keep the event loop moving.
     *
     * @param {Uint8Array | string} password
     * @param {string} hash
     * @returns {boolean}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly two.
     * @throws {Error} with `code` `NEO_CRYPTO_LIMIT` or
     * `NEO_CRYPTO_DETACHED` for an input that is too large or detached.
     */
    function bcryptVerify(password, hash) {
        if (arguments.length !== 2) {
            throw new makeTypeError("bcryptVerify takes exactly two arguments");
        }
        return unwrapCrypto(opBcryptVerify(password, hash));
    }

    /**
     * Verify a bcrypt hash on the runtime's bounded worker pool.
     *
     * A well-formed hash runs on a blocking worker. An oversized password or a
     * malformed or over-cost hash resolves to `false` without starting a
     * worker, because there is no expensive work to do. An invalid argument
     * type still rejects. When a worker is started, the join starts
     * immediately; ignoring the returned Promise does not cancel the work. Only
     * runtime teardown aborts a queued job; a running job cannot be stopped.
     *
     * @param {Uint8Array | string} password
     * @param {string} hash
     * @returns {Promise<boolean>}
     * @throws rejects with `TypeError` or an `Error` carrying a stable `code`.
     */
    async function bcryptVerifyAsync(password, hash) {
        if (arguments.length !== 2) {
            throw new makeTypeError("bcryptVerifyAsync takes exactly two arguments");
        }
        const response = opBcryptVerifyAsync(password, hash);
        if (!response.ok) throwCrypto(response);
        if (typeof response.value === "boolean") return response.value;
        return joinPassword(response.id, 2);
    }

    // A frozen module object, like `chars`: the properties cannot be replaced,
    // but the function objects themselves are shared with the imports.
    const crypto = Object.freeze({
        generateEnvelopeKey,
        randomBytes,
        encrypt,
        decrypt,
        createEncryptor,
        createDecryptor,
        createHash,
        createHmac,
        hashAsync,
        hmacAsync,
        pbkdf2,
        pbkdf2Async,
        bcryptHash,
        bcryptHashAsync,
        bcryptVerify,
        bcryptVerifyAsync,
    });

    /**
     * Turn an age host result object into a value or the documented exception.
     *
     * A host-side argument error is `TypeError`; every other failure is `Error`
     * with an own `code` defined through the captured `Object.defineProperty`,
     * so an inherited `Object.prototype.code` accessor cannot swallow it. The
     * host result object has a null prototype, so its properties cannot be
     * intercepted either.
     *
     * @param {{ok: boolean, value?: string, identity?: string, recipient?: string, name?: string, code?: string, message?: string}} response
     * @returns {string | Uint8Array | undefined}
     */
    function unwrapAge(response) {
        if (response.ok) return response.value;
        let error;
        if (response.name === "TypeError") {
            error = new makeTypeError(response.message);
        } else {
            error = new makeError(response.message);
            defineProperty(error, "code", {
                __proto__: null,
                value: response.code,
                writable: true,
                enumerable: true,
                configurable: true,
            });
        }
        throw error;
    }

    /**
     * Generate a new X25519 identity and its recipient.
     *
     * The secret comes from the operating system CSPRNG through the
     * `neotales-age` backend; an entropy failure throws rather than returning
     * predictable bytes. The identity is `AGE-SECRET-KEY-1...` and the recipient
     * is `age1...`, both in standard Bech32 text.
     *
     * Keep the identity secret. This module stores no keys, does not derive or
     * rotate them for the caller, and does not promise zeroization.
     *
     * @returns {{identity: string, recipient: string}}
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `NEO_AGE_RANDOM` when the OS CSPRNG fails.
     */
    function generateIdentity() {
        if (arguments.length !== 0) {
            throw new makeTypeError("generateIdentity takes no arguments");
        }
        const response = opAgeGenerateIdentity();
        if (!response.ok) unwrapAge(response);
        return { identity: response.identity, recipient: response.recipient };
    }

    /**
     * Derive the X25519 recipient for an identity.
     *
     * Only standard age X25519 identities are accepted. A valid age key of
     * another kind that parses within the 512-byte bound (for example a
     * post-quantum identity) is rejected with `NEO_AGE_UNSUPPORTED`, not
     * re-interpreted. Text that is not an age identity at all, including SSH or
     * passphrase text, is `NEO_AGE_IDENTITY`. Leading and trailing whitespace
     * is trimmed; the text is bounded to 512 bytes before parsing.
     *
     * @param {string} identity an `AGE-SECRET-KEY-1...` identity
     * @returns {string} the matching `age1...` recipient
     * @throws {TypeError} when the argument is not a string or the argument
     * count is not exactly one.
     * @throws {Error} with `code` `NEO_AGE_IDENTITY`, `NEO_AGE_UNSUPPORTED`, or
     * `NEO_AGE_INTERNAL`.
     */
    function identityToRecipient(identity) {
        if (arguments.length !== 1) {
            throw new makeTypeError("identityToRecipient takes exactly one argument");
        }
        if (typeof identity !== "string") {
            throw new makeTypeError("identity must be a string");
        }
        return unwrapAge(opAgeIdentityToRecipient(identity));
    }

    /**
     * Encrypt bytes to one X25519 recipient as a binary age v1 file.
     *
     * The result is a standard binary age file, decryptable by the `age` or
     * `rage` command-line tools. Inputs and output are `Uint8Array`s; the
     * recipient is Bech32 text and is never interpreted as bytes. A recipient
     * whose X25519 public key is a low-order point cannot produce a usable
     * shared secret and is rejected with `NEO_AGE_RECIPIENT`.
     *
     * Limits: `plaintext` is limited to 8 MiB and the ciphertext to 8 MiB plus
     * 1 MiB for the header and per-chunk tags. A key or plaintext over the bound
     * throws before copying or allocating. This is a neojs limit, not an age
     * format limit.
     *
     * Security: the age crate draws the file key and payload nonce from the
     * operating system CSPRNG. A failure throws `NEO_AGE_RANDOM`; there is no
     * weaker fallback. The module does not store keys or promise zeroization.
     *
     * @param {string} recipient an `age1...` X25519 recipient
     * @param {Uint8Array} plaintext
     * @returns {Uint8Array} a binary age v1 file
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly two.
     * @throws {Error} with `code` `NEO_AGE_RECIPIENT`, `NEO_AGE_UNSUPPORTED`,
     * `NEO_AGE_LIMIT`, `NEO_AGE_RANDOM`, `NEO_AGE_DETACHED`, or
     * `NEO_AGE_INTERNAL`.
     */
    function ageEncrypt(recipient, plaintext) {
        if (arguments.length !== 2) {
            throw new makeTypeError("encrypt takes exactly two arguments");
        }
        if (typeof recipient !== "string") {
            throw new makeTypeError("recipient must be a string");
        }
        return unwrapAge(opAgeEncrypt(recipient, plaintext));
    }

    /**
     * Verify and decrypt one binary age v1 file with one X25519 identity.
     *
     * The header MAC and every payload chunk tag are verified by the backend.
     * No plaintext is returned when authentication fails, and a truncated or
     * tampered file throws instead of returning partial bytes. Only the binary
     * format is accepted; ASCII armor, passphrases, SSH keys, and other
     * identity kinds are not implemented in this slice.
     *
     * Limits: the ciphertext is limited to 8 MiB + 1 MiB and the recovered
     * plaintext to 8 MiB. The payload limit is enforced while chunks are
     * authenticated, so an oversized result fails rather than buffering.
     *
     * @param {string} identity an `AGE-SECRET-KEY-1...` X25519 identity
     * @param {Uint8Array} ciphertext a binary age v1 file
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly two.
     * @throws {Error} with `code` `NEO_AGE_AUTH`, `NEO_AGE_FORMAT`,
     * `NEO_AGE_IDENTITY`, `NEO_AGE_IDENTITY_MISMATCH`, `NEO_AGE_UNSUPPORTED`,
     * `NEO_AGE_LIMIT`, `NEO_AGE_DETACHED`, or `NEO_AGE_INTERNAL`.
     */
    function ageDecrypt(identity, ciphertext) {
        if (arguments.length !== 2) {
            throw new makeTypeError("decrypt takes exactly two arguments");
        }
        if (typeof identity !== "string") {
            throw new makeTypeError("identity must be a string");
        }
        return unwrapAge(opAgeDecrypt(identity, ciphertext));
    }

    // A frozen module object, like `chars` and `crypto`: the properties cannot
    // be replaced, but the function objects themselves are shared with imports.
    const age = Object.freeze({
        generateIdentity,
        identityToRecipient,
        encrypt: ageEncrypt,
        decrypt: ageDecrypt,
    });

    // The ssh module is a synchronous command-execution slice. It is not a
    // shell, PTY, SCP, SFTP, port forwarder, or agent client, and it never
    // reads a file, environment variable, or default known-hosts path. Host
    // trust is mandatory and fail-closed: exactly one of `hostKey` or
    // `knownHosts` is required, and there is no unknown-host mode. The limits
    // below must match the constants in `neojs-core/src/ssh.rs`; the boundary
    // tests exercise both sides.
    const MAX_COMMAND_BYTES = 65536;
    const MAX_OUTPUT_BYTES = 8388608;
    const DEFAULT_CONNECT_TIMEOUT_MS = 15000;
    const DEFAULT_EXEC_TIMEOUT_MS = 30000;

    // The option names `connect` accepts. An unknown name is rejected rather
    // than ignored, so a misspelled or invented option cannot silently weaken
    // host verification or authentication. Keep this list in step with the
    // documentation in `neojs-core/src/ssh.rs`.
    const SSH_OPTION_KEYS = Object.freeze([
        "host",
        "port",
        "username",
        "password",
        "privateKey",
        "agent",
        "passphrase",
        "hostKey",
        "knownHosts",
        "connectTimeoutMs",
        "execTimeoutMs",
    ]);

    // The token that makes `SshConnection` constructible only by this shim.
    // It is module-private, so a caller cannot fabricate a connection handle.
    const sshConnectionToken = Symbol("neojs.ssh.Connection");

    /**
     * Create the ordinary `Error` this module reports, with a stable own
     * `code` and `name = "SshError"`. `code` is defined rather than assigned
     * so an inherited `Object.prototype.code` accessor cannot swallow it. The
     * message never contains a password, passphrase, key byte, command, or
     * output byte.
     */
    function sshError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "name", {
            __proto__: null,
            value: "SshError",
            writable: true,
            enumerable: false,
            configurable: true,
        });
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /** Throw the exception described by one host result object. */
    function throwSsh(response) {
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        if (response.name === "RangeError") {
            throw new makeRangeError(response.message);
        }
        throw sshError(response.code, response.message);
    }

    /**
     * One synchronous, authenticated SSH connection.
     *
     * Returned by `connect`, not constructible by user code. `exec` runs one
     * command and returns `{ stdout, status }`; stderr is discarded. Both
     * methods block the isolate. `close` is idempotent and closes the socket by
     * dropping the native connection; it does not send a graceful SSH
     * disconnect. A fatal `exec` failure also closes the connection.
     */
    class SshConnection {
        #id;
        #closed = false;

        constructor(token, id) {
            if (token !== sshConnectionToken) {
                throw new makeTypeError(
                    "SSH connections cannot be constructed directly; use connect()",
                );
            }
            this.#id = id;
        }

        /** Whether this connection can no longer run a command. */
        get closed() {
            return this.#closed;
        }

        /**
         * Run one command and return its captured stdout and exit status.
         *
         * `command` is a string encoded as UTF-8 or a `Uint8Array` of exact
         * bytes. The server's stderr is discarded. The command is bounded by
         * MAX_COMMAND_BYTES and captured stdout by MAX_OUTPUT_BYTES. The call
         * blocks until the exit status arrives and the session channel closes,
         * or until the connection's `execTimeoutMs` expires.
         *
         * @param {string | Uint8Array} command
         * @returns {{stdout: Uint8Array, status: number}}
         * @throws {TypeError} when the argument count is not one or the value
         * is neither a string nor a `Uint8Array`.
         * @throws {Error} with `code` `NEO_SSH_CLOSED`, `NEO_SSH_LIMIT`,
         * `NEO_SSH_TIMEOUT`, `NEO_SSH_IO`, `NEO_SSH_SESSION`, or
         * `NEO_SSH_INTERNAL`. A failure after the connection is engaged
         * (transport, session, timeout, or output over the limit) closes it.
         * Argument validation, including a command over MAX_COMMAND_BYTES,
         * throws `TypeError`, `RangeError`, or `NEO_SSH_LIMIT` without
         * closing the connection; inspect `closed` after catching.
         */
        exec(command) {
            if (arguments.length !== 1) {
                throw new makeTypeError("exec takes exactly one argument");
            }
            if (this.#closed) {
                throw sshError("NEO_SSH_CLOSED", "the SSH connection is closed");
            }
            const response = opSshExec(this.#id, command);
            if (!response.ok) {
                if (response.closed === true) this.#closed = true;
                throwSsh(response);
            }
            return { stdout: response.stdout, status: response.status };
        }

        /**
         * Close this connection. Idempotent.
         *
         * Dropping the native connection closes its TCP socket; no SSH
         * disconnect message is sent. Later `exec` calls throw
         * `NEO_SSH_CLOSED`.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            opSshClose(this.#id);
        }
    }

    /**
     * Dial, verify, authenticate, and return a synchronous SSH connection.
     *
     * Exactly one of `hostKey` (an SSH-format Ed25519 public-key blob as a
     * `Uint8Array`) or `knownHosts` (the text of a known-hosts document) is
     * required. There is no insecure default, no first-use acceptance, and no
     * fallback to a file: a missing or mismatched host key refuses to connect.
     *
     * Exactly one of `password`, `privateKey`, or `agent: true` is required.
     * `privateKey` is an in-memory `Uint8Array` containing unencrypted PKCS#8
     * DER, a PKCS#8 PEM block, a binary `openssh-key-v1` container, or an
     * OpenSSH PEM block. A `passphrase` is accepted only with `privateKey`, is
     * used only for OpenSSH containers, and is wiped by the SSH backend after
     * parsing; supplying one with a PKCS#8 key is refused. Encrypted PKCS#8 is
     * not supported.
     *
     * `agent` is an explicit opt-in and is supported on Unix only. When true,
     * `SSH_AUTH_SOCK` is read and its identities are tried, at most 16, after
     * the server host key has been verified. At most two authentication
     * requests are made per identity, 32 total. The agent timeout is not a
     * hard wall clock: `UnixStream::connect` may block before any timeout is
     * installed, and the backend's 30-second read/write timeout is
     * inactivity-based, so a peer that trickles bytes can prolong enumeration
     * or a signing request beyond 30 seconds. It is independent of
     * `connectTimeoutMs`; this slice rejects Windows named pipes rather than
     * claiming a bounded pipe timeout. There is no agent forwarding and no
     * implicit agent use: other modes never read `SSH_AUTH_SOCK`. Apart from
     * that opt-in, this module never reads a file, environment variable, agent,
     * or terminal.
     *
     * `connectTimeoutMs` (1..120000, default 15000) bounds each TCP connect
     * attempt and is the socket inactivity timeout during the blocking
     * handshake and authentication; DNS resolution is outside it. The
     * handshake is one library call without an incremental deadline, so a
     * peer that stalls fails but one that trickles bytes can extend it.
     * `execTimeoutMs` (1..600000, default 30000) is checked between session
     * events and before their socket I/O for each later `exec`.
     *
     * The call blocks the isolate until the handshake and authentication
     * finish. It returns no `Promise` and takes no callback.
     *
     * @param {object} options
     * @param {string} options.host
     * @param {number} [options.port=22]
     * @param {string} options.username
     * @param {string} [options.password]
     * @param {Uint8Array} [options.privateKey]
     * @param {boolean} [options.agent]
     * @param {string} [options.passphrase]
     * @param {Uint8Array} [options.hostKey]
     * @param {string} [options.knownHosts]
     * @param {number} [options.connectTimeoutMs=15000]
     * @param {number} [options.execTimeoutMs=30000]
     * @returns {SshConnection}
     * @throws {TypeError} when the argument count is not one, `options` is not
     * a plain object, an unknown option name is supplied, or a value has the
     * wrong type.
     * @throws {RangeError} when a number is not an integer in its documented
     * range.
     * @throws {Error} with `code` `NEO_SSH_HOST_KEY_REQUIRED`,
     * `NEO_SSH_HOST_KEY`, `NEO_SSH_KNOWN_HOSTS`, `NEO_SSH_AUTH_METHOD`,
     * `NEO_SSH_CREDENTIAL`, `NEO_SSH_AGENT`, `NEO_SSH_CONNECT`,
     * `NEO_SSH_TIMEOUT`, `NEO_SSH_HANDSHAKE`, `NEO_SSH_AUTH`, `NEO_SSH_LIMIT`,
     * or `NEO_SSH_INTERNAL`.
     */
    function sshConnect(options) {
        if (arguments.length !== 1) {
            throw new makeTypeError("connect takes exactly one argument");
        }
        if (options === null || typeof options !== "object" || arrayIsArray(options)) {
            throw new makeTypeError("connect takes one options object");
        }
        const keys = objectKeys(options);
        for (let index = 0; index < keys.length; index += 1) {
            const key = keys[index];
            if (!arrayIncludes(SSH_OPTION_KEYS, key)) {
                throw new makeTypeError(`unknown SSH option \`${key}\``);
            }
        }
        const response = opSshConnect(
            options.host,
            options.port,
            options.username,
            options.password,
            options.privateKey,
            options.agent,
            options.passphrase,
            options.hostKey,
            options.knownHosts,
            options.connectTimeoutMs,
            options.execTimeoutMs,
        );
        if (!response.ok) throwSsh(response);
        if (typeof response.id !== "number") {
            throw sshError(
                "NEO_SSH_INTERNAL",
                "the SSH module returned an invalid connection handle",
            );
        }
        return new SshConnection(sshConnectionToken, response.id);
    }

    // A frozen module object, like `chars` and `crypto`: the properties cannot
    // be replaced, but `connect` is shared with the imports. Host trust and
    // credentials are per-call arguments; this module stores no keys.
    const ssh = Object.freeze({
        connect: sshConnect,
        MAX_COMMAND_BYTES,
        MAX_OUTPUT_BYTES,
        DEFAULT_CONNECT_TIMEOUT_MS,
        DEFAULT_EXEC_TIMEOUT_MS,
    });

    // The largest number of bytes one memory-backed stream retains, and the
    // largest single writeSync chunk. It must match `MAX_BYTES` in
    // `neojs-core/src/stdio.rs`; the boundary tests exercise both sides.
    const MAX_BYTES = 8388608;

    /**
     * Create the ordinary `Error` this module reports, with a stable own
     * `code` and `name = "StdioError"`. `code` is defined rather than assigned
     * so an inherited `Object.prototype.code` accessor cannot swallow it.
     */
    function stdioError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "name", {
            __proto__: null,
            value: "StdioError",
            writable: true,
            enumerable: false,
            configurable: true,
        });
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /**
     * Return the host result object on success, or throw the documented
     * exception. A `TypeError` result is a wrong type; everything else is a
     * `StdioError` with a stable code and a message that never includes the
     * rejected bytes.
     */
    function stdioResult(response) {
        if (response.ok) return response;
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        throw stdioError(response.code, response.message);
    }

    /**
     * A synchronous reader over an owned snapshot of a `Uint8Array`.
     *
     * The input view is copied at construction, so mutating, detaching, or
     * resizing the caller's buffer afterwards does not change the reader.
     * `readSync` copies at most `target.byteLength` bytes and returns the count,
     * or exactly `0` at end of file. It can return less than the target holds,
     * so loop until it returns `0`. An empty target throws
     * `NEO_STDIO_ARGUMENT` rather than returning an ambiguous zero. `close()`
     * releases the snapshot, is idempotent, and never detaches a caller's
     * buffer; reads after it throw `NEO_STDIO_CLOSED`.
     */
    class Readable {
        #bytes;
        #length = 0;
        #offset = 0;
        #closed = false;

        /**
         * Copy the current byte range of `bytes` into reader-owned storage.
         *
         * @param {Uint8Array} bytes an attached view of at most MAX_BYTES
         * @throws {TypeError} when the argument count is not one or the value
         * is not a `Uint8Array`.
         * @throws {Error} with `code` `NEO_STDIO_DETACHED` or
         * `NEO_STDIO_LIMIT`.
         */
        constructor(bytes) {
            if (arguments.length !== 1) {
                throw new makeTypeError("Readable takes exactly one argument");
            }
            if (typeof bytes === "string") {
                throw new makeTypeError("Readable takes a Uint8Array, not a string");
            }
            const copy = stdioResult(opStdioCopy(bytes, MAX_BYTES));
            this.#bytes = copy.value;
            this.#length = copy.length;
        }

        /** Bytes not yet read; `0` at end of file and after `close()`. */
        get remaining() {
            return this.#closed ? 0 : this.#length - this.#offset;
        }

        /** Whether `close()` has been called. */
        get closed() {
            return this.#closed;
        }

        /**
         * Copy up to `target.byteLength` remaining bytes into `target`.
         *
         * @param {Uint8Array} target an attached, non-empty view
         * @returns {number} the bytes copied, or `0` at end of file
         * @throws {TypeError} when the argument count is not one or the value
         * is not a `Uint8Array`.
         * @throws {Error} with `code` `NEO_STDIO_CLOSED` (before the target is
         * validated), `NEO_STDIO_DETACHED`, or `NEO_STDIO_ARGUMENT` for an
         * empty target, even at end of file.
         */
        readSync(target) {
            if (arguments.length !== 1) {
                throw new makeTypeError("readSync takes exactly one argument");
            }
            if (this.#closed) {
                throw stdioError("NEO_STDIO_CLOSED", "the reader is closed");
            }
            const targetLength = stdioResult(opStdioViewLength(target)).value;
            if (targetLength === 0) {
                throw stdioError("NEO_STDIO_ARGUMENT", "readSync target must not be empty");
            }
            const available = this.#length - this.#offset;
            if (available === 0) return 0;
            const count = targetLength < available ? targetLength : available;
            // The native slice op copies the range. Do not use `subarray`: its
            // `Symbol.species` lookup would run user JavaScript with the
            // private snapshot buffer and allow reentrancy.
            const slice = stdioResult(opStdioSlice(this.#bytes, this.#offset, this.#offset + count));
            typedArraySet(target, slice.value, 0);
            this.#offset += slice.length;
            return slice.length;
        }

        /** Release the owned snapshot. Idempotent. */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            this.#bytes = null;
            this.#length = 0;
            this.#offset = 0;
        }
    }

    // Assigned by the static block below. It is private to this shim, so user
    // code cannot construct a host-backed writer.
    let hostWritable;

    /**
     * A synchronous writer. `new Writable()` creates a memory-backed writer
     * over one coalesced, owned buffer whose capacity is bounded by MAX_BYTES;
     * empty writes retain nothing and there is no per-write record. The exported
     * `stdout` and `stderr` are host-backed writers for the current process's
     * standard streams.
     *
     * `writeSync` accepts a `Uint8Array` or a string encoded as UTF-8 and
     * returns the number of bytes accepted. A memory writer is all-or-nothing:
     * it appends the whole chunk or throws without changing the stream. A host
     * writer blocks until the operating system accepts every byte; a throw can
     * mean that a prefix was already written, and there is no retry or
     * rollback. `end()` retires the writer but keeps memory-backed content
     * readable; `close()` is terminal and releases it. Neither closes the
     * process descriptor. Both are idempotent. `stdout` and `stderr` are shared
     * per realm, so retiring one through any reference is visible through every
     * reference; `console` does not go through these objects.
     */
    class Writable {
        #kind;
        #stream;
        // Memory-backed writers keep one coalesced buffer. Its capacity is
        // bounded by MAX_BYTES and never grows because of empty writes; there
        // is no per-write record that could grow without bound.
        #buffer;
        #capacity;
        #length;
        #closed;
        #ended;

        /**
         * Create an empty memory-backed writer.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        constructor() {
            if (arguments.length !== 0) {
                throw new makeTypeError("Writable takes no arguments");
            }
            this.#kind = "memory";
            this.#stream = 0;
            this.#buffer = null;
            this.#capacity = 0;
            this.#length = 0;
            this.#closed = false;
            this.#ended = false;
        }

        static {
            // A class static block can install private state, so the host
            // writers are created without exposing a factory or constructor
            // form to user code.
            hostWritable = (stream, kind) => {
                const writer = new Writable();
                writer.#stream = stream;
                writer.#kind = kind;
                return writer;
            };
        }

        /**
         * Grow the coalesced buffer to hold at least `required` bytes.
         *
         * Capacity doubles from 1 KiB, is clamped to MAX_BYTES, and is only
         * allocated when a non-empty append needs room. The old contents are
         * copied before the new buffer is installed, so a failed allocation
         * leaves the writer unchanged. `#length` never includes unused
         * capacity, and `toUint8Array` copies only the used prefix.
         */
        #reserve(required) {
            if (required <= this.#capacity) return;
            let capacity = this.#capacity === 0 ? 1024 : this.#capacity * 2;
            if (capacity < required) capacity = required;
            if (capacity > MAX_BYTES) capacity = MAX_BYTES;
            if (capacity < required) {
                throw stdioError("NEO_STDIO_LIMIT", "byte data exceeds the stdio size limit");
            }
            const buffer = new makeUint8Array(capacity);
            if (this.#length > 0) typedArraySet(buffer, this.#buffer, 0);
            this.#buffer = buffer;
            this.#capacity = capacity;
        }

        /** `"memory"`, `"stdout"`, or `"stderr"`. */
        get kind() {
            return this.#kind;
        }

        /**
         * Bytes retained by a memory writer, or `0` for a host writer, which
         * retains nothing. `0` after `close()`.
         */
        get length() {
            return this.#closed ? 0 : this.#length;
        }

        /** Whether `close()` has been called. */
        get closed() {
            return this.#closed;
        }

        /** Whether `end()` or `close()` has been called. */
        get ended() {
            return this.#ended;
        }

        /**
         * Append a chunk, or write it to the host stream.
         *
         * @param {Uint8Array | string} chunk at most MAX_BYTES as UTF-8 bytes
         * @returns {number} the bytes accepted, never a short count
         * @throws {TypeError} when the argument count is not one or the value
         * is neither a `Uint8Array` nor a string.
         * @throws {Error} with `code` `NEO_STDIO_CLOSED`, `NEO_STDIO_ENDED`,
         * `NEO_STDIO_DETACHED`, `NEO_STDIO_LIMIT`, or `NEO_STDIO_WRITE`.
         */
        writeSync(chunk) {
            if (arguments.length !== 1) {
                throw new makeTypeError("writeSync takes exactly one argument");
            }
            if (this.#closed) {
                throw stdioError("NEO_STDIO_CLOSED", "the writer is closed");
            }
            if (this.#ended) {
                throw stdioError("NEO_STDIO_ENDED", "the writer has ended");
            }
            if (this.#kind !== "memory") {
                return stdioResult(opStdioWrite(this.#stream, chunk)).value;
            }
            const capacity = MAX_BYTES - this.#length;
            const copy = stdioResult(opStdioCopy(chunk, capacity));
            if (copy.length === 0) return 0;
            this.#reserve(this.#length + copy.length);
            typedArraySet(this.#buffer, copy.value, this.#length);
            this.#length += copy.length;
            return copy.length;
        }

        /** Stop accepting writes. Idempotent; does not release content. */
        end() {
            if (arguments.length !== 0) {
                throw new makeTypeError("end takes no arguments");
            }
            if (this.#closed) return;
            this.#ended = true;
        }

        /**
         * Return a fresh `Uint8Array` copy of the memory-backed content. The
         * caller owns the result; changing it does not change the writer.
         *
         * @throws {Error} with `code` `NEO_STDIO_CLOSED`, or
         * `NEO_STDIO_NOT_MEMORY` when the writer is `stdout` or `stderr`.
         */
        toUint8Array() {
            if (arguments.length !== 0) {
                throw new makeTypeError("toUint8Array takes no arguments");
            }
            if (this.#closed) {
                throw stdioError("NEO_STDIO_CLOSED", "the writer is closed");
            }
            if (this.#kind !== "memory") {
                throw stdioError("NEO_STDIO_NOT_MEMORY", "the stream is not memory-backed");
            }
            if (this.#length === 0) return new makeUint8Array(0);
            // The native slice op copies only the used prefix and returns a
            // fresh caller-owned array. It does not consult species.
            return stdioResult(opStdioSlice(this.#buffer, 0, this.#length)).value;
        }

        /** Release memory-backed content and retire the writer. Idempotent. */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            this.#ended = true;
            if (this.#kind === "memory") {
                this.#buffer = null;
                this.#capacity = 0;
                this.#length = 0;
            }
        }
    }

    // The two host writers are created once per realm and shared by the
    // registry and every import. They use the current process's own standard
    // descriptors; closing one retires the wrapper and leaves the descriptor
    // open for console and for the other references to this same object.
    const stdout = hostWritable(1, "stdout");
    const stderr = hostWritable(2, "stderr");

    // A frozen module object, like `chars`, `crypto`, and `age`: the properties
    // cannot be replaced, but the classes and the shared stream objects are the
    // same values every importer sees.
    const stdio = Object.freeze({
        Readable,
        Writable,
        stdout,
        stderr,
        MAX_BYTES,
    });

    // The console module is the realm's installed console object. The console
    // shim runs before this script, so the registry, the module, and the global
    // share the exact function objects at installation time. It is deliberately
    // not frozen: it is the live console object, and replacing a method stays
    // visible through every path that holds the object. That is a mutation
    // contract, not a security boundary.
    const consoleModule = globalThis.console;

    // The `neo:path` built-in module object.
    //
    // This is a lexical path implementation adapted from NeoLiteJS `neo:path` and
    // `@neotales/path`. See `neojs-core/THIRD_PARTY_NOTICES.md` for the exact
    // upstream revision, the adapted files, and the MIT notices. It is not a
    // drop-in replacement for Node's `path`, `@neotales/path`, or NeoLiteJS
    // `neo:path`.
    //
    // Scope: POSIX and Windows lexical operations only. The module does not read or
    // write files, check existence, resolve symlinks or junctions, read environment
    // variables, normalize Unicode, or validate Windows device names or alternate
    // data streams. It is not a containment check. The only host read is the
    // process working directory, through `opNeoPathCwd`, and only for
    // `resolve`/`relative`/`toNamespacedPath` when their inputs do not supply a
    // complete base. Glob and file URL helpers are deliberately absent.
    //
    // The two namespaces are frozen and available on every host. The named exports
    // select the build host's dialect from `opNeoPathIsWindows`, never from a
    // script-replaceable global. Every path argument must be a primitive string;
    // nothing is coerced. A NUL is rejected because it cannot appear in a usable
    // C path. Failures needing a working directory are `PathError` with an own
    // `code`: `PATH_CWD`, `PATH_CWD_DIALECT`, or `PATH_WINDOWS_ROOT`.
    //
    // `common` compares components as written and does not normalize or fold
    // case. On Windows it is the documented exception to the two-separator
    // input rule: it splits on `\` only, matching the reference, so `/` is not
    // a component boundary there.
    //
    // `relative` resolves its two endpoints against one lazy per-call cwd
    // snapshot. It reads the process cwd at most once per call, and not at all
    // when both endpoints already have a complete absolute base. `resolve` and
    // Windows `toNamespacedPath` likewise read at most once per call.
    //
    // This block is evaluated inside the `neo.js` IIFE before user code. It is
    // wrapped in its own scope so its captured intrinsics do not collide with the
    // other built-in modules.
    const path = (() => {
        "use strict";

        const pathOps = Deno.core.ops;
        // Capture the functions, not just the ops object. Replacing a property on
        // `Deno.core.ops` later must not change what this module calls.
        const opNeoPathIsWindows = pathOps.op_neo_path_is_windows;
        const opNeoPathCwd = pathOps.op_neo_path_cwd;
        const hostWindows = opNeoPathIsWindows();
        function readCwd() {
            const directory = opNeoPathCwd();
            return directory === undefined ? null : directory;
        }

        // The host builds this API in the fresh realm, before running user code.
        const NativeError = Error;
        const NativeTypeError = TypeError;
        const defineProperty = Object.defineProperty;
        const freeze = Object.freeze;
        const isArray = Array.isArray;
        const isSafeInteger = Number.isSafeInteger;
        const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
        const slice = Function.prototype.call.bind(String.prototype.slice);
        const split = Function.prototype.call.bind(String.prototype.split);
        const lower = Function.prototype.call.bind(String.prototype.toLowerCase);
        const indexOf = Function.prototype.call.bind(String.prototype.indexOf);
        const lastIndexOf = Function.prototype.call.bind(String.prototype.lastIndexOf);
        const joinArray = Function.prototype.call.bind(Array.prototype.join);
        const sliceArray = Function.prototype.call.bind(Array.prototype.slice);

        const SLASH = 47;
        const BACKSLASH = 92;
        const DOT = 46;
        const COLON = 58;

        function isPosixSeparator(code) {
          return code === SLASH;
        }

        function isWin32Separator(code) {
          return code === SLASH || code === BACKSLASH;
        }

        /** True for an ASCII letter, the only drive-letter form Windows accepts. */
        function isDeviceRoot(code) {
          return (code >= 65 && code <= 90) || (code >= 97 && code <= 122);
        }

        // True when the whole path is a recognized UNC root: exactly two leading
        // separators, a non-empty server, at least one separator, a non-empty share,
        // and nothing after the share except separators. This is the same root that
        // `parse` reports as `{root, dir, base: '', ext: '', name: ''}`. A run of
        // three or more leading separators is not a UNC root.
        function isWin32UncRootOnly(path) {
          const length = path.length;
          if (length < 4) return false;
          if (
            !isWin32Separator(charCodeAt(path, 0)) ||
            !isWin32Separator(charCodeAt(path, 1))
          ) {
            return false;
          }

          let index = 2;
          const serverStart = index;
          while (index < length && !isWin32Separator(charCodeAt(path, index))) ++index;
          if (index === serverStart || index >= length) return false;

          const separatorStart = index;
          while (index < length && isWin32Separator(charCodeAt(path, index))) ++index;
          if (index === separatorStart || index >= length) return false;

          const shareStart = index;
          while (index < length && !isWin32Separator(charCodeAt(path, index))) ++index;
          if (index === shareStart) return false;

          while (index < length) {
            if (!isWin32Separator(charCodeAt(path, index))) return false;
            ++index;
          }
          return true;
        }

        function assertString(value, name) {
          if (typeof value !== 'string')
            throw new NativeTypeError(name + ' must be a string');
          if (indexOf(value, '\u0000') !== -1)
            throw new NativeTypeError(name + ' must not contain a NUL character');
          return value;
        }

        function assertPath(value) {
          return assertString(value, 'path');
        }

        function assertSuffix(value) {
          return assertString(value, 'suffix');
        }

        // Resolve `.` and `..` segments. `allowAboveRoot` decides whether a `..` with
        // no preceding segment is kept. A trailing separator is not handled here.
        function normalizeString(path, allowAboveRoot, separator, isSeparator) {
          let result = '';
          let lastSegmentLength = 0;
          let lastSlash = -1;
          let dots = 0;
          let code;

          for (let i = 0; i <= path.length; ++i) {
            if (i < path.length) code = charCodeAt(path, i);
            else if (isSeparator(code)) break;
            else code = SLASH;

            if (isSeparator(code)) {
              if (lastSlash === i - 1 || dots === 1) {
                // Empty segment or `.`: nothing to add.
              } else if (lastSlash !== i - 1 && dots === 2) {
                if (
                  result.length < 2 ||
                  lastSegmentLength !== 2 ||
                  charCodeAt(result, result.length - 1) !== DOT ||
                  charCodeAt(result, result.length - 2) !== DOT
                ) {
                  if (result.length > 2) {
                    const lastSlashIndex = lastIndexOf(result, separator);
                    if (lastSlashIndex === -1) {
                      result = '';
                      lastSegmentLength = 0;
                    } else {
                      result = slice(result, 0, lastSlashIndex);
                      lastSegmentLength =
                        result.length - 1 - lastIndexOf(result, separator);
                    }
                    lastSlash = i;
                    dots = 0;
                    continue;
                  }
                  if (result.length === 2 || result.length === 1) {
                    result = '';
                    lastSegmentLength = 0;
                    lastSlash = i;
                    dots = 0;
                    continue;
                  }
                }
                if (allowAboveRoot) {
                  result = result.length > 0 ? result + separator + '..' : '..';
                  lastSegmentLength = 2;
                }
              } else {
                const segment = slice(path, lastSlash + 1, i);
                result = result.length > 0 ? result + separator + segment : segment;
                lastSegmentLength = i - lastSlash - 1;
              }
              lastSlash = i;
              dots = 0;
            } else if (code === DOT && dots !== -1) {
              ++dots;
            } else {
              dots = -1;
            }
          }

          return result;
        }

        // Remove trailing separators without shortening the root. `minimum` is the
        // number of leading code units that belong to the root.
        function stripTrailingSeparators(segment, isSeparator, minimum) {
          let end = segment.length;
          for (let i = segment.length - 1; i >= minimum; --i) {
            if (!isSeparator(charCodeAt(segment, i))) break;
            end = i;
          }
          return slice(segment, 0, end);
        }

        function stripSuffix(name, suffix) {
          if (suffix.length === 0 || suffix.length >= name.length) return name;
          const offset = name.length - suffix.length;
          for (let i = suffix.length - 1; i >= 0; --i) {
            if (charCodeAt(name, offset + i) !== charCodeAt(suffix, i)) return name;
          }
          return slice(name, 0, -suffix.length);
        }

        function basenamePath(path, suffix, isSeparator, hasDrivePrefix) {
          let end = path.length;
          while (end > 0 && isSeparator(charCodeAt(path, end - 1))) end--;

          let start = 0;
          if (
            hasDrivePrefix &&
            end >= 2 &&
            isDeviceRoot(charCodeAt(path, 0)) &&
            charCodeAt(path, 1) === COLON
          ) {
            start = 2;
          }

          if (end <= start) return '';

          let nameStart = end - 1;
          while (nameStart > start && !isSeparator(charCodeAt(path, nameStart - 1)))
            nameStart--;

          return stripSuffix(slice(path, nameStart, end), suffix);
        }

        function extnamePath(path, isSeparator, start) {
          let startDot = -1;
          let startPart = start;
          let end = -1;
          let matchedSlash = true;
          let preDotState = 0;

          for (let i = path.length - 1; i >= start; --i) {
            const code = charCodeAt(path, i);
            if (isSeparator(code)) {
              if (!matchedSlash) {
                startPart = i + 1;
                break;
              }
              continue;
            }
            if (end === -1) {
              matchedSlash = false;
              end = i + 1;
            }
            if (code === DOT) {
              if (startDot === -1) startDot = i;
              else if (preDotState !== 1) preDotState = 1;
            } else if (startDot !== -1) {
              preDotState = -1;
            }
          }

          if (
            startDot === -1 ||
            end === -1 ||
            preDotState === 0 ||
            (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
          ) {
            return '';
          }
          return slice(path, startDot, end);
        }

        function formatPath(separator, pathObject) {
          if (
            pathObject === null ||
            typeof pathObject !== 'object' ||
            isArray(pathObject)
          ) {
            throw new NativeTypeError('pathObject must be an object');
          }

          const root = readField(pathObject, 'root');
          const dirValue = readField(pathObject, 'dir');
          const baseValue = readField(pathObject, 'base');
          const name = readField(pathObject, 'name');
          const ext = readField(pathObject, 'ext');

          const dir = dirValue.length > 0 ? dirValue : root;
          const base = baseValue.length > 0 ? baseValue : name + ext;

          if (dir.length === 0) return base;
          if (dir === root) return dir + base;
          return dir + separator + base;
        }

        function readField(pathObject, name) {
          const value = pathObject[name];
          if (value === undefined) return '';
          return assertString(value, name);
        }

        function normalizePosix(path) {
          return normalizePosixText(assertPath(path));
        }

        function normalizePosixText(path) {
          const absolute = path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
          const trailing =
            path.length > 0 && isPosixSeparator(charCodeAt(path, path.length - 1));

          let tail = normalizeString(path, !absolute, '/', isPosixSeparator);
          if (tail.length === 0 && !absolute) tail = '.';
          if (tail.length > 0 && trailing) tail += '/';

          return absolute ? '/' + tail : tail;
        }

        function joinPosix(...paths) {
          const parts = [];
          for (let i = 0; i < paths.length; i++) {
            const part = assertPath(paths[i]);
            if (part.length > 0) parts[parts.length] = part;
          }
          if (parts.length === 0) return '.';
          return normalizePosix(joinArray(parts, '/'));
        }

        function basenamePosix(path, suffix = '') {
          assertPath(path);
          assertSuffix(suffix);
          return basenamePath(path, suffix, isPosixSeparator, false);
        }

        function dirnamePosix(path) {
          assertPath(path);

          let end = -1;
          let matchedNonSeparator = false;
          for (let i = path.length - 1; i >= 1; --i) {
            if (isPosixSeparator(charCodeAt(path, i))) {
              if (matchedNonSeparator) {
                end = i;
                break;
              }
            } else {
              matchedNonSeparator = true;
            }
          }

          const absolute = path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
          if (end === -1) return absolute ? '/' : '.';

          return stripTrailingSeparators(
            slice(path, 0, end),
            isPosixSeparator,
            absolute ? 1 : 0,
          );
        }

        function extnamePosix(path) {
          assertPath(path);
          return extnamePath(path, isPosixSeparator, 0);
        }

        function isAbsolutePosix(path) {
          assertPath(path);
          return path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
        }

        function parsePosix(path) {
          assertPath(path);

          const result = { root: '', dir: '', base: '', ext: '', name: '' };
          if (path.length === 0) return result;

          const absolute = isPosixSeparator(charCodeAt(path, 0));
          let start;
          if (absolute) {
            result.root = '/';
            start = 1;
          } else {
            start = 0;
          }

          let startDot = -1;
          let startPart = 0;
          let end = -1;
          let matchedSlash = true;
          let preDotState = 0;

          for (let i = path.length - 1; i >= start; --i) {
            const code = charCodeAt(path, i);
            if (isPosixSeparator(code)) {
              if (!matchedSlash) {
                startPart = i + 1;
                break;
              }
              continue;
            }
            if (end === -1) {
              matchedSlash = false;
              end = i + 1;
            }
            if (code === DOT) {
              if (startDot === -1) startDot = i;
              else if (preDotState !== 1) preDotState = 1;
            } else if (startDot !== -1) {
              preDotState = -1;
            }
          }

          if (
            startDot === -1 ||
            end === -1 ||
            preDotState === 0 ||
            (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
          ) {
            if (end !== -1) {
              const componentStart = startPart === 0 && absolute ? 1 : startPart;
              result.base = result.name = slice(path, componentStart, end);
            }
          } else {
            const componentStart = startPart === 0 && absolute ? 1 : startPart;
            result.name = slice(path, componentStart, startDot);
            result.base = slice(path, componentStart, end);
            result.ext = slice(path, startDot, end);
          }

          if (startPart > 0) {
            result.dir = stripTrailingSeparators(
              slice(path, 0, startPart - 1),
              isPosixSeparator,
              absolute ? 1 : 0,
            );
          } else if (absolute) {
            result.dir = '/';
          }

          return result;
        }

        function formatPosix(pathObject) {
          return formatPath('/', pathObject);
        }

        function normalizeWin32(path) {
          return normalizeWin32Text(assertPath(path));
        }

        function normalizeWin32Text(path) {
          const length = path.length;
          let rootEnd = 0;
          let device;
          let absolute = false;
          const code = charCodeAt(path, 0);

          if (length > 1) {
            if (isWin32Separator(code)) {
              // A leading separator is rooted. Two separators are a UNC root only when
              // a non-empty server and share follow; otherwise the run collapses.
              absolute = true;

              if (isWin32Separator(charCodeAt(path, 1))) {
                let j = 2;
                let last = j;
                while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                if (j < length && j !== last) {
                  const firstPart = slice(path, last, j);
                  last = j;
                  while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
                  if (j < length && j !== last) {
                    last = j;
                    while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                    if (j === length) {
                      return '\\\\' + firstPart + '\\' + slice(path, last) + '\\';
                    }
                    if (j !== last) {
                      device = '\\\\' + firstPart + '\\' + slice(path, last, j);
                      rootEnd = j;
                    }
                  }
                }
              } else {
                rootEnd = 1;
              }
            } else if (
              isDeviceRoot(code) &&
              charCodeAt(path, 1) === COLON
            ) {
              device = slice(path, 0, 2);
              rootEnd = 2;
              if (length > 2) {
                if (isWin32Separator(charCodeAt(path, 2))) {
                  absolute = true;
                  rootEnd = 3;
                }
              }
            }
          } else if (isWin32Separator(code)) {
            return '\\';
          }

          let tail;
          if (rootEnd < length) {
            tail = normalizeString(
              slice(path, rootEnd),
              !absolute,
              '\\',
              isWin32Separator,
            );
          } else {
            tail = '';
          }

          if (tail.length === 0 && !absolute) tail = '.';
          if (tail.length > 0 && isWin32Separator(charCodeAt(path, length - 1)))
            tail += '\\';

          if (device === undefined) {
            if (absolute) return tail.length > 0 ? '\\' + tail : '\\';
            return tail;
          }
          if (absolute) return tail.length > 0 ? device + '\\' + tail : device + '\\';
          return device + tail;
        }

        function joinWin32(...paths) {
          const parts = [];
          for (let i = 0; i < paths.length; i++) {
            const part = assertPath(paths[i]);
            if (part.length > 0) parts[parts.length] = part;
          }
          return joinWin32Parts(parts);
        }

        function joinWin32Parts(parts) {
          if (parts.length === 0) return '.';

          let needsReplace = true;
          let slashCount = 0;
          const firstPart = parts[0];
          if (isWin32Separator(charCodeAt(firstPart, 0))) {
            ++slashCount;
            if (firstPart.length > 1) {
              if (isWin32Separator(charCodeAt(firstPart, 1))) {
                ++slashCount;
                if (firstPart.length > 2) {
                  if (isWin32Separator(charCodeAt(firstPart, 2))) ++slashCount;
                  else needsReplace = false;
                }
              }
            }
          }

          let joined = joinArray(parts, '\\');
          if (needsReplace) {
            for (; slashCount < joined.length; ++slashCount) {
              if (!isWin32Separator(charCodeAt(joined, slashCount))) break;
            }
            if (slashCount >= 2) joined = '\\' + slice(joined, slashCount);
          }

          return normalizeWin32(joined);
        }

        function basenameWin32(path, suffix = '') {
          assertPath(path);
          assertSuffix(suffix);
          // A complete UNC root is a directory, not a file, even when the share name
          // contains a dot. `parse` reports an empty `base` for the same path.
          if (isWin32UncRootOnly(path)) return '';
          return basenamePath(path, suffix, isWin32Separator, true);
        }

        function dirnameWin32(path) {
          assertPath(path);

          const length = path.length;
          let rootEnd = -1;
          let end = -1;
          let matchedSlash = true;
          let offset = 0;
          const code = charCodeAt(path, 0);

          if (length > 1) {
            if (isWin32Separator(code)) {
              rootEnd = offset = 1;

              if (isWin32Separator(charCodeAt(path, 1))) {
                let j = 2;
                let last = j;
                while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                if (j < length && j !== last) {
                  last = j;
                  while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
                  if (j < length && j !== last) {
                    last = j;
                    while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                    if (j === length) return path;
                    if (j !== last) rootEnd = offset = j + 1;
                  }
                }
              }
            } else if (isDeviceRoot(code) && charCodeAt(path, 1) === COLON) {
              rootEnd = offset = 2;
              if (length > 2 && isWin32Separator(charCodeAt(path, 2)))
                rootEnd = offset = 3;
            }
          } else if (isWin32Separator(code)) {
            return path;
          }

          for (let i = length - 1; i >= offset; --i) {
            if (isWin32Separator(charCodeAt(path, i))) {
              if (!matchedSlash) {
                end = i;
                break;
              }
            } else {
              matchedSlash = false;
            }
          }

          if (end === -1) {
            if (rootEnd === -1) return '.';
            return slice(path, 0, rootEnd);
          }

          return stripTrailingSeparators(
            slice(path, 0, end),
            isWin32Separator,
            rootEnd === -1 ? 0 : rootEnd,
          );
        }

        function extnameWin32(path) {
          assertPath(path);
          // The share name of a complete UNC root is not a file component, so a dot
          // in it is not an extension. This matches `parse`, which reports an empty
          // `base` for such a path.
          if (isWin32UncRootOnly(path)) return '';

          let start = 0;
          if (
            path.length >= 2 &&
            charCodeAt(path, 1) === COLON &&
            isDeviceRoot(charCodeAt(path, 0))
          ) {
            start = 2;
          }
          return extnamePath(path, isWin32Separator, start);
        }

        function isAbsoluteWin32(path) {
          assertPath(path);

          const length = path.length;
          if (length === 0) return false;

          const code = charCodeAt(path, 0);
          if (isWin32Separator(code)) return true;
          if (
            isDeviceRoot(code) &&
            length > 2 &&
            charCodeAt(path, 1) === COLON &&
            isWin32Separator(charCodeAt(path, 2))
          ) {
            return true;
          }
          return false;
        }

        function parseWin32(path) {
          assertPath(path);

          const result = { root: '', dir: '', base: '', ext: '', name: '' };
          const length = path.length;
          if (length === 0) return result;

          let rootEnd = 0;
          let code = charCodeAt(path, 0);

          if (length > 1) {
            if (isWin32Separator(code)) {
              rootEnd = 1;

              if (isWin32Separator(charCodeAt(path, 1))) {
                let j = 2;
                let last = j;
                while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                if (j < length && j !== last) {
                  last = j;
                  while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
                  if (j < length && j !== last) {
                    last = j;
                    while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                    if (j === length) {
                      rootEnd = j;
                    } else if (j !== last) {
                      rootEnd = j + 1;
                    }
                  }
                }
              }
            } else if (isDeviceRoot(code) && charCodeAt(path, 1) === COLON) {
              rootEnd = 2;
              if (length > 2) {
                if (isWin32Separator(charCodeAt(path, 2))) {
                  if (length === 3) {
                    result.root = result.dir = path;
                    return result;
                  }
                  rootEnd = 3;
                }
              } else {
                result.root = result.dir = path;
                return result;
              }
            }
          } else if (isWin32Separator(code)) {
            result.root = result.dir = path;
            return result;
          }

          if (rootEnd > 0) result.root = slice(path, 0, rootEnd);

          let startDot = -1;
          let startPart = rootEnd;
          let end = -1;
          let matchedSlash = true;
          let preDotState = 0;

          for (let i = length - 1; i >= rootEnd; --i) {
            code = charCodeAt(path, i);
            if (isWin32Separator(code)) {
              if (!matchedSlash) {
                startPart = i + 1;
                break;
              }
              continue;
            }
            if (end === -1) {
              matchedSlash = false;
              end = i + 1;
            }
            if (code === DOT) {
              if (startDot === -1) startDot = i;
              else if (preDotState !== 1) preDotState = 1;
            } else if (startDot !== -1) {
              preDotState = -1;
            }
          }

          if (
            startDot === -1 ||
            end === -1 ||
            preDotState === 0 ||
            (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
          ) {
            if (end !== -1) result.base = result.name = slice(path, startPart, end);
          } else {
            result.name = slice(path, startPart, startDot);
            result.base = slice(path, startPart, end);
            result.ext = slice(path, startDot, end);
          }

          if (startPart > 0 && startPart !== rootEnd) {
            result.dir = stripTrailingSeparators(
              slice(path, 0, startPart - 1),
              isWin32Separator,
              rootEnd,
            );
          } else {
            result.dir = result.root;
          }

          return result;
        }

        function formatWin32(pathObject) {
          return formatPath('\\', pathObject);
        }

        /** Read one length from a possibly proxied array, without coercion. */
        function arrayLength(value, name) {
          if (!isArray(value)) throw new NativeTypeError(name + ' must be an array');
          const length = value.length;
          if (!isSafeInteger(length) || length < 0 || length > 0xffffffff)
            throw new NativeTypeError(name + ' length must be a valid array length');
          return length;
        }

        /** Find the shared spelling at component boundaries, without normalizing. */
        function commonPath(paths, separator) {
          const length = arrayLength(paths, 'paths');
          if (length === 0) return '';
          const first = split(assertPath(paths[0]), separator);
          let end = first.length;
          let append = '';
          for (let j = 1; j < length; ++j) {
            const parts = split(assertPath(paths[j]), separator);
            if (parts.length <= end) {
              end = parts.length;
              append = '';
            }
            for (let i = 0; i < end; ++i) {
              if (first[i] !== parts[i]) {
                end = i;
                append = i === 0 ? '' : separator;
                break;
              }
            }
          }
          return joinArray(sliceArray(first, 0, end), separator) + append;
        }

        function pathError(code, message) {
          const error = new NativeError(message);
          defineProperty(error, 'name', {
            __proto__: null,
            value: 'PathError',
            writable: true,
            enumerable: false,
            configurable: true,
          });
          defineProperty(error, 'code', {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
          });
          return error;
        }

        /** Obtain only the process cwd for the host dialect; never translate it. */
        function requireCwd(windows) {
          if (windows !== hostWindows) {
            throw pathError('PATH_CWD_DIALECT', 'provide an absolute base for the non-host path dialect');
          }
          const directory = readCwd();
          if (directory === null || directory === undefined) {
            throw pathError('PATH_CWD', 'the working directory is unavailable; provide an absolute base');
          }
          return directory;
        }

        /**
         * A lazy, per-call cwd snapshot for one dialect.
         *
         * `relative` resolves two endpoints; both must see the same directory
         * even if the host directory changes between reads. The first call that
         * actually needs a base reads cwd; later calls in the same operation
         * reuse it. A call whose inputs are all absolute never invokes it.
         */
        function cwdSnapshot(windows) {
          let read = false;
          let value;
          return function readCwdOnce() {
            if (!read) {
              value = requireCwd(windows);
              read = true;
            }
            return value;
          };
        }

        /** Resolve from the rightmost absolute segment, or one cwd snapshot. */
        function resolvePosix(...paths) {
          for (let i = 0; i < paths.length; ++i) assertPath(paths[i]);
          return resolvePosixInternal(paths, cwdSnapshot(false));
        }

        function resolvePosixInternal(paths, readCwdOnce) {
          const tails = [];
          for (let i = paths.length - 1; i >= -1; --i) {
            const path = i === -1 ? readCwdOnce() : paths[i];
            if (path === '') continue;
            tails[tails.length] = path;
            if (isPosixSeparator(charCodeAt(path, 0))) break;
          }
          return '/' + normalizedTail(tails, '/', isPosixSeparator);
        }

        function normalizedTail(tails, separator, isSeparator) {
          // Segments were collected right to left. Swap in place before normalizing.
          for (let i = 0, j = tails.length - 1; i < j; ++i, --j) {
            const tail = tails[i];
            tails[i] = tails[j];
            tails[j] = tail;
          }
          return normalizeString(joinArray(tails, separator), false, separator, isSeparator);
        }

        function skipWin32Separators(path, start) {
          while (isWin32Separator(charCodeAt(path, start))) ++start;
          return start;
        }

        function win32ComponentEnd(path, start) {
          while (start < path.length && !isWin32Separator(charCodeAt(path, start))) ++start;
          return start;
        }

        /** A namespace prefix such as `\\?\` or `\\.\`, including slash spellings. */
        function hasNamespacePrefix(path) {
          const marker = charCodeAt(path, 2);
          return isWin32Separator(charCodeAt(path, 0)) && isWin32Separator(charCodeAt(path, 1)) &&
            (marker === 63 || marker === DOT) && isWin32Separator(charCodeAt(path, 3));
        }

        /** Extract a device/share as a unit so `..` cannot climb above it. */
        function resolutionRootWin32(path) {
          const first = charCodeAt(path, 0);
          if (isDeviceRoot(first) && charCodeAt(path, 1) === COLON) {
            const device = slice(path, 0, 2);
            const absolute = isWin32Separator(charCodeAt(path, 2));
            return { device, drive: device, absolute, end: absolute ? 3 : 2 };
          }
          if (!isWin32Separator(first)) return { device: '', drive: '', absolute: false, end: 0 };
          if (isWin32Separator(charCodeAt(path, 1))) {
            const serverEnd = win32ComponentEnd(path, 2);
            const shareStart = skipWin32Separators(path, serverEnd);
            let end = win32ComponentEnd(path, shareStart);
            if (serverEnd > 2 && end > shareStart) {
              const server = slice(path, 2, serverEnd);
              const share = slice(path, shareStart, end);
              let device = '\\\\' + server + '\\' + share;
              let drive = '';
              if (server === '?' && lower(share) === 'unc') {
                const hostStart = skipWin32Separators(path, end);
                const hostEnd = win32ComponentEnd(path, hostStart);
                const volumeStart = skipWin32Separators(path, hostEnd);
                end = win32ComponentEnd(path, volumeStart);
                if (hostStart === hostEnd || volumeStart === end) {
                  throw pathError('PATH_WINDOWS_ROOT', 'provide a server and share for a namespaced UNC path');
                }
                device += '\\' + slice(path, hostStart, hostEnd) + '\\' + slice(path, volumeStart, end);
              } else if (server === '?' && share.length === 2 &&
                isDeviceRoot(charCodeAt(share, 0)) && charCodeAt(share, 1) === COLON) {
                drive = share;
              }
              return { device, drive, absolute: true, end };
            }
          }
          if (hasNamespacePrefix(path)) {
            throw pathError('PATH_WINDOWS_ROOT', 'provide a complete Windows namespace root');
          }
          return { device: '', drive: '', absolute: true, end: 1 };
        }

        /** Resolve on one drive/share, without consulting any per-drive environment. */
        function resolveWin32(...paths) {
          for (let i = 0; i < paths.length; ++i) assertPath(paths[i]);
          return resolveWin32Internal(paths, cwdSnapshot(true));
        }

        function resolveWin32Internal(paths, readCwdOnce) {
          let device = '';
          let driveRelative = false;
          let absolute = false;
          const tails = [];
          for (let i = paths.length - 1; i >= -1; --i) {
            const path = i === -1 ? readCwdOnce() : paths[i];
            if (path === '') continue;
            const root = resolutionRootWin32(path);
            if (root.device !== '') {
              if (device === '') {
                device = root.device;
                driveRelative = !root.absolute;
              } else if (lower(device) !== lower(root.device)) {
                // Windows canonicalization can store a verbatim drive cwd. Match a
                // drive-relative input to this drive, but keep the verbatim spelling.
                if (driveRelative && lower(device) === lower(root.drive)) device = root.device;
                else continue;
              }
            }
            if (!absolute) {
              tails[tails.length] = slice(path, root.end);
              absolute = root.absolute;
            }
            if (absolute && device !== '') break;
          }
          if (!absolute || device === '') {
            throw pathError('PATH_WINDOWS_ROOT', 'provide an absolute Windows base on the requested drive or share');
          }
          return device + '\\' + normalizedTail(tails, '\\', isWin32Separator);
        }

        /** Compare whole components, retaining the destination's original spelling. */
        function relativeTail(from, to, separator, foldCase) {
          const source = from === '' ? [] : split(from, separator);
          const target = to === '' ? [] : split(to, separator);
          let i = 0;
          while (i < source.length && i < target.length &&
            (foldCase ? lower(source[i]) === lower(target[i]) : source[i] === target[i])) ++i;
          const result = [];
          for (let j = i; j < source.length; ++j) result[result.length] = '..';
          for (; i < target.length; ++i) result[result.length] = target[i];
          const path = joinArray(result, separator);
          // A destination component that starts with a drive letter and colon
          // must not turn this relative result into a drive-relative path.
          return foldCase && isDeviceRoot(charCodeAt(path, 0)) && charCodeAt(path, 1) === COLON
            ? '.\\' + path : path;
        }

        function relativePosix(from, to) {
          assertPath(from);
          assertPath(to);
          const readCwdOnce = cwdSnapshot(false);
          const source = resolvePosixInternal([from], readCwdOnce);
          const target = resolvePosixInternal([to], readCwdOnce);
          return relativeTail(slice(source, 1), slice(target, 1), '/', false);
        }

        function relativeWin32(from, to) {
          assertPath(from);
          assertPath(to);
          const readCwdOnce = cwdSnapshot(true);
          const source = resolveWin32Internal([from], readCwdOnce);
          const target = resolveWin32Internal([to], readCwdOnce);
          const sourceRoot = resolutionRootWin32(source);
          const targetRoot = resolutionRootWin32(target);
          if (lower(sourceRoot.device) !== lower(targetRoot.device)) return target;
          return relativeTail(
            slice(source, skipWin32Separators(source, sourceRoot.end)),
            slice(target, skipWin32Separators(target, targetRoot.end)),
            '\\',
            true,
          );
        }

        function toNamespacedPathWin32(path) {
          assertPath(path);
          if (path === '' || hasNamespacePrefix(path)) return path;
          const resolved = resolveWin32(path);
          if (hasNamespacePrefix(resolved)) return resolved;
          if (charCodeAt(resolved, 0) === BACKSLASH) return '\\\\?\\UNC\\' + slice(resolved, 2);
          return '\\\\?\\' + resolved;
        }

        const posix = freeze({
          sep: '/',
          delimiter: ':',
          join: joinPosix,
          normalize: normalizePosix,
          basename: basenamePosix,
          dirname: dirnamePosix,
          extname: extnamePosix,
          isAbsolute: isAbsolutePosix,
          parse: parsePosix,
          format: formatPosix,
          common: (paths) => commonPath(paths, '/'),
          resolve: resolvePosix,
          relative: relativePosix,
          toNamespacedPath: assertPath,
        });

        const win32 = freeze({
          sep: '\\',
          delimiter: ';',
          join: joinWin32,
          normalize: normalizeWin32,
          basename: basenameWin32,
          dirname: dirnameWin32,
          extname: extnameWin32,
          isAbsolute: isAbsoluteWin32,
          parse: parseWin32,
          format: formatWin32,
          common: (paths) => commonPath(paths, '\\'),
          resolve: resolveWin32,
          relative: relativeWin32,
          toNamespacedPath: toNamespacedPathWin32,
        });

        const native = hostWindows ? win32 : posix;
        return freeze({
          posix,
          win32,
          join: native.join,
          normalize: native.normalize,
          basename: native.basename,
          dirname: native.dirname,
          extname: native.extname,
          isAbsolute: native.isAbsolute,
          parse: native.parse,
          format: native.format,
          common: native.common,
          resolve: native.resolve,
          relative: native.relative,
          toNamespacedPath: native.toNamespacedPath,
          SEPARATOR: native.sep,
          DELIMITER: native.delimiter,
          SEPARATOR_PATTERN: hostWindows ? /[\\/]+/ : /\/+/,
        });
    })();

    // `assert` is a small assertion surface inspired by Node's `assert/strict`
    // and Deno's `@std/assert`. It is an independent implementation informed by
    // the NeoLiteJS `neo:assert` module; it does not claim compatibility with
    // either and exports no aliases. In particular there is no `assertEquals`,
    // `deepEqual`, or `strictEqual`: structural equality is `equals` and
    // identity is `strictEquals`.
    //
    // `equals` is strict structural equality. Prototypes must match, own
    // enumerable string and symbol properties are compared, arrays also compare
    // length, and Dates, RegExps, Maps, Sets, ArrayBuffers, DataViews, typed
    // arrays, boxed primitives, and Errors compare their internal state as well
    // as own properties. Maps and Sets are compared without regard to iteration
    // order; functions, Promises, WeakMaps, WeakSets, and WeakRefs compare by
    // identity. `NaN` equals `NaN`, `-0` differs from `0`, and cycles are
    // handled. `strictEquals` is exactly `Object.is`: it never compares two
    // distinct objects structurally.
    //
    // `throws` and `doesNotThrow` take synchronous callbacks. A callback that
    // returns a native Promise is a `TypeError` that points at `rejects` or
    // `doesNotReject`, rather than a pass based on work that has not run.
    // `rejects` and `doesNotReject` return Promises and handle both a
    // synchronous throw and a returned thenable.
    //
    // Capture the intrinsics used by comparison, callback checks, and
    // reporting. Code in the same realm can replace these globals later; that
    // does not change whether an assertion passes or make the formatter throw.
    // This is not isolation from code that replaced them before this script ran.
    const objectIs = Object.is;
    const getOwnPropertySymbols = Object.getOwnPropertySymbols;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const getPrototypeOf = Object.getPrototypeOf;
    const propertyIsEnumerable = Object.prototype.propertyIsEnumerable;
    const hasOwnProperty = Object.prototype.hasOwnProperty;
    const stringify = JSON.stringify;
    const mathMin = Math.min;
    const arrayJoin = Function.prototype.call.bind(Array.prototype.join);
    const arrayBufferIsView = ArrayBuffer.isView;
    const NativeMap = Map;
    const NativeSet = Set;
    const NativeDate = Date;
    const NativeRegExp = RegExp;
    const NativeError = Error;
    const NativeBoolean = Boolean;
    const NativeNumber = Number;
    const NativeString = String;
    const NativeBigInt = typeof BigInt === "function" ? BigInt : null;
    const NativeSymbol = Symbol;
    const NativeArrayBuffer = ArrayBuffer;
    const NativeSharedArrayBuffer =
        typeof SharedArrayBuffer === "function" ? SharedArrayBuffer : null;
    const NativePromise = Promise;
    const NativeWeakMap = WeakMap;
    const NativeWeakSet = WeakSet;
    const NativeWeakRef = typeof WeakRef === "function" ? WeakRef : null;
    const NativeDataView = typeof DataView === "function" ? DataView : null;

    // `Function.prototype.call` is captured once so a later replacement cannot
    // change an equality result or make a guard call the wrong function.
    const uncurry = Function.prototype.call.bind(Function.prototype.call);
    const functionHasInstance = Function.prototype[Symbol.hasInstance];

    const dateGetTime = NativeDate.prototype.getTime;
    const dateToISOString = NativeDate.prototype.toISOString;
    const regexpSource = getOwnPropertyDescriptor(NativeRegExp.prototype, "source").get;
    const regexpFlags = getOwnPropertyDescriptor(NativeRegExp.prototype, "flags").get;
    const mapSize = getOwnPropertyDescriptor(NativeMap.prototype, "size").get;
    const mapForEach = NativeMap.prototype.forEach;
    const mapDelete = NativeMap.prototype.delete;
    const setSize = getOwnPropertyDescriptor(NativeSet.prototype, "size").get;
    const setForEach = NativeSet.prototype.forEach;
    const setDelete = NativeSet.prototype.delete;
    const booleanValueOf = NativeBoolean.prototype.valueOf;
    const numberValueOf = NativeNumber.prototype.valueOf;
    const stringValueOf = NativeString.prototype.valueOf;
    const bigintValueOf = NativeBigInt ? NativeBigInt.prototype.valueOf : null;
    const symbolValueOf = NativeSymbol.prototype.valueOf;
    const arrayBufferByteLength = getOwnPropertyDescriptor(
        NativeArrayBuffer.prototype,
        "byteLength",
    ).get;
    const sharedArrayBufferByteLength = NativeSharedArrayBuffer
        ? getOwnPropertyDescriptor(NativeSharedArrayBuffer.prototype, "byteLength").get
        : null;
    const dataViewBuffer = NativeDataView
        ? getOwnPropertyDescriptor(NativeDataView.prototype, "buffer").get
        : null;
    const dataViewByteOffset = NativeDataView
        ? getOwnPropertyDescriptor(NativeDataView.prototype, "byteOffset").get
        : null;
    const dataViewByteLength = NativeDataView
        ? getOwnPropertyDescriptor(NativeDataView.prototype, "byteLength").get
        : null;
    const TypedArrayPrototype = getPrototypeOf(makeUint8Array.prototype);
    const typedArrayTag = getOwnPropertyDescriptor(
        TypedArrayPrototype,
        NativeSymbol.toStringTag,
    ).get;
    const typedArrayLength = getOwnPropertyDescriptor(TypedArrayPrototype, "length").get;
    const typedArrayBuffer = getOwnPropertyDescriptor(TypedArrayPrototype, "buffer").get;
    const typedArrayByteOffset = getOwnPropertyDescriptor(
        TypedArrayPrototype,
        "byteOffset",
    ).get;
    const promiseThen = Function.prototype.call.bind(NativePromise.prototype.then);
    const captureStackTrace = NativeError.captureStackTrace;

    const MAX_TEXT = 400;
    const MAX_ITEMS = 50;
    const MAX_DEPTH = 8;
    const MAX_NAME = 64;
    const INDEX_KEY = /^(0|[1-9][0-9]*)$/;
    const IDENTIFIER_KEY = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

    // Create an own public field with the attributes plain assignment would
    // give it. An inherited setter for the field name must not run, and the
    // descriptor must not inherit Object.prototype.get or .set.
    function defineField(object, key, value) {
        defineProperty(object, key, {
            __proto__: null,
            value,
            enumerable: true,
            configurable: true,
            writable: true,
        });
    }

    // Create own index properties rather than assigning through an inherited
    // setter on Array.prototype.
    function defineIndex(array, index, value) {
        defineField(array, index, value);
    }

    function append(array, value) {
        defineIndex(array, array.length, value);
    }

    function contains(array, value) {
        for (let index = 0; index < array.length; index += 1) {
            if (array[index] === value) return true;
        }
        return false;
    }

    /** Thrown when an assertion fails. */
    class AssertionError extends NativeError {
        constructor(message, options) {
            super(message);
            // Own data properties: an inherited setter for a field name must
            // not replace this error or drop its code. The code is
            // neojs-specific; it is not Node's `ERR_ASSERTION`.
            defineField(this, "name", "AssertionError");
            defineField(this, "code", "NEO_ASSERT");
            const details = options || {};
            defineField(this, "operator", details.operator || "assert");
            if ("actual" in details) defineField(this, "actual", details.actual);
            if ("expected" in details) defineField(this, "expected", details.expected);
            if (typeof captureStackTrace === "function") {
                captureStackTrace(this, AssertionError);
            }
        }
    }

    function checkMessage(message) {
        if (message !== undefined && typeof message !== "string") {
            throw new makeTypeError("message must be a string");
        }
    }

    function checkArity(args, name, minimum, maximum) {
        if (args.length < minimum || args.length > maximum) {
            const expected =
                minimum === maximum ? NativeString(minimum) : `${minimum} to ${maximum}`;
            throw new makeTypeError(
                `${name} expects ${expected} argument${maximum === 1 ? "" : "s"}, got ${args.length}`,
            );
        }
    }

    function checkCallback(callback) {
        if (typeof callback !== "function") {
            throw new makeTypeError("callback must be a function");
        }
    }

    function isObjectLike(value) {
        return value !== null && (typeof value === "object" || typeof value === "function");
    }

    // OrdinaryHasInstance, called directly so a caller's own Symbol.hasInstance
    // hook cannot turn a wrong error class into a pass.
    function hasBrand(value, constructor) {
        try {
            return uncurry(functionHasInstance, constructor, value);
        } catch {
            return false;
        }
    }

    function isNativePromise(value) {
        return hasBrand(value, NativePromise);
    }

    function isThenable(value) {
        if (!isObjectLike(value)) return false;
        if (isNativePromise(value)) return true;
        try {
            return typeof value.then === "function";
        } catch {
            // Awaiting it will surface the same accessor error as a rejection.
            return true;
        }
    }

    // Consume a native Promise that a synchronous assertion is refusing, so a
    // later rejection is reported as handled rather than as a second failure.
    function settlePromise(promise) {
        try {
            promiseThen(promise, undefined, function () {});
        } catch {
            // A subclass with a hostile species constructor may refuse; the
            // TypeError below is still the useful result.
        }
    }

    function withMessage(message, detail) {
        return message === undefined ? detail : `${message}: ${detail}`;
    }

    function truncate(text) {
        const value = NativeString(text);
        if (value.length <= MAX_TEXT) return value;
        return (
            stringSlice(value, 0, MAX_TEXT) +
            `... (${value.length - MAX_TEXT} more characters)`
        );
    }

    function quote(text) {
        return stringify(truncate(text));
    }

    function formatKey(key) {
        if (typeof key === "symbol") return `[${NativeString(key)}]`;
        if (regexpExec(INDEX_KEY, key) !== null) return `[${key}]`;
        return regexpExec(IDENTIFIER_KEY, key) !== null ? `.${key}` : `[${quote(key)}]`;
    }

    /** Own enumerable string and symbol keys, in property order. */
    function ownKeys(value) {
        const keys = objectKeys(value);
        const symbols = getOwnPropertySymbols(value);
        for (let index = 0; index < symbols.length; index += 1) {
            if (uncurry(propertyIsEnumerable, value, symbols[index])) {
                append(keys, symbols[index]);
            }
        }
        return keys;
    }

    function constructorName(value) {
        const prototype = getPrototypeOf(value);
        if (prototype === null) return "null-prototype";
        let constructor;
        try {
            constructor = prototype.constructor;
        } catch {
            return "";
        }
        const name = constructor && typeof constructor.name === "string" ? constructor.name : "";
        if (name === "Object" || name.length === 0) return "";
        return name.length <= MAX_NAME ? name : `${stringSlice(name, 0, MAX_NAME)}...`;
    }

    function safeMessage(error) {
        try {
            if (error && typeof error.message === "string") return error.message;
            return NativeString(error);
        } catch {
            return "unprintable error";
        }
    }

    function formatObject(value, depth, seen) {
        if (arrayIsArray(value)) {
            const parts = [];
            const length = mathMin(value.length, MAX_ITEMS);
            for (let index = 0; index < length; index += 1) {
                append(
                    parts,
                    uncurry(hasOwnProperty, value, index)
                        ? formatValue(value[index], depth + 1, seen)
                        : "<empty>",
                );
            }
            if (value.length > MAX_ITEMS) append(parts, "...");
            return `[${arrayJoin(parts, ", ")}]`;
        }

        if (hasBrand(value, NativeDate)) {
            const time = uncurry(dateGetTime, value);
            return time === time ? `Date(${uncurry(dateToISOString, value)})` : "Invalid Date";
        }
        if (hasBrand(value, NativeRegExp)) {
            return `/${uncurry(regexpSource, value)}/${uncurry(regexpFlags, value)}`;
        }
        if (hasBrand(value, NativeMap)) {
            const parts = [];
            let count = 0;
            uncurry(mapForEach, value, function (entryValue, key) {
                if (count >= MAX_ITEMS) return;
                count += 1;
                append(
                    parts,
                    `${formatValue(key, depth + 1, seen)} => ${formatValue(
                        entryValue,
                        depth + 1,
                        seen,
                    )}`,
                );
            });
            const size = uncurry(mapSize, value);
            if (size > MAX_ITEMS) append(parts, "...");
            return `Map(${size}) {${parts.length ? ` ${arrayJoin(parts, ", ")} ` : ""}}`;
        }
        if (hasBrand(value, NativeSet)) {
            const parts = [];
            let count = 0;
            uncurry(setForEach, value, function (entryValue) {
                if (count >= MAX_ITEMS) return;
                count += 1;
                append(parts, formatValue(entryValue, depth + 1, seen));
            });
            const size = uncurry(setSize, value);
            if (size > MAX_ITEMS) append(parts, "...");
            return `Set(${size}) {${parts.length ? ` ${arrayJoin(parts, ", ")} ` : ""}}`;
        }
        if (hasBrand(value, NativeArrayBuffer)) {
            return `ArrayBuffer(${uncurry(arrayBufferByteLength, value)})`;
        }
        if (NativeSharedArrayBuffer && hasBrand(value, NativeSharedArrayBuffer)) {
            return `SharedArrayBuffer(${uncurry(sharedArrayBufferByteLength, value)})`;
        }
        if (NativeDataView && hasBrand(value, NativeDataView)) {
            return `DataView(${uncurry(dataViewByteLength, value)})`;
        }
        if (arrayBufferIsView(value)) {
            const tag = uncurry(typedArrayTag, value);
            const length = uncurry(typedArrayLength, value);
            const parts = [];
            const count = mathMin(length, MAX_ITEMS);
            for (let index = 0; index < count; index += 1) {
                append(parts, formatValue(value[index], depth + 1, seen));
            }
            if (length > MAX_ITEMS) append(parts, "...");
            return `${tag}(${length}) [${arrayJoin(parts, ", ")}]`;
        }
        if (hasBrand(value, NativeError)) {
            let name = "Error";
            let message = "<message unreadable>";
            try {
                if (typeof value.name === "string" && value.name) name = value.name;
            } catch {
                // Keep the fallback name.
            }
            try {
                message = truncate(value.message);
            } catch {
                // Keep the fallback message.
            }
            return `${name}: ${message}`;
        }
        if (hasBrand(value, NativeBoolean)) {
            return `[Boolean: ${formatValue(uncurry(booleanValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativeNumber)) {
            return `[Number: ${formatValue(uncurry(numberValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativeString)) {
            return `[String: ${formatValue(uncurry(stringValueOf, value), depth + 1, seen)}]`;
        }
        if (NativeBigInt && hasBrand(value, NativeBigInt)) {
            return `[BigInt: ${formatValue(uncurry(bigintValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativeSymbol)) {
            return `[Symbol: ${formatValue(uncurry(symbolValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativePromise)) return "Promise { <state unknown> }";
        if (hasBrand(value, NativeWeakMap)) return "WeakMap { <items unknown> }";
        if (hasBrand(value, NativeWeakSet)) return "WeakSet { <items unknown> }";
        if (NativeWeakRef && hasBrand(value, NativeWeakRef)) return "WeakRef { <target unknown> }";

        const name = constructorName(value);
        const keys = ownKeys(value);
        const parts = [];
        for (let index = 0; index < keys.length && index < MAX_ITEMS; index += 1) {
            const key = keys[index];
            let text;
            try {
                text = `${formatKey(key)}: ${formatValue(value[key], depth + 1, seen)}`;
            } catch (error) {
                text = `${formatKey(key)}: [threw ${truncate(safeMessage(error))}]`;
            }
            append(parts, text);
        }
        if (keys.length > MAX_ITEMS) append(parts, "...");
        return `${name ? `${name} ` : ""}{${parts.length ? ` ${arrayJoin(parts, ", ")} ` : ""}}`;
    }

    /**
     * Best-effort value formatting for assertion messages. Getters are invoked,
     * cycles print as `[Circular]`, and output is truncated by depth, item
     * count, and total text length. This never throws: a hostile accessor or a
     * mutated prototype yields a fallback string, so the assertion still
     * reports its failure.
     */
    function formatValue(value, depth, seen) {
        try {
            return formatValueUnsafe(value, depth, seen);
        } catch {
            return "[unprintable value]";
        }
    }

    function formatValueUnsafe(value, depth, seen) {
        const level = depth || 0;
        const path = seen || [];
        if (value === null) return "null";
        const kind = typeof value;
        if (kind === "string") return quote(value);
        if (kind === "number") return objectIs(value, -0) ? "-0" : NativeString(value);
        if (kind === "bigint") return `${NativeString(value)}n`;
        if (kind === "boolean" || kind === "undefined") return NativeString(value);
        if (kind === "symbol") return NativeString(value);
        if (kind === "function") {
            let name;
            try {
                name = value.name;
            } catch {
                name = "";
            }
            return typeof name === "string" && name
                ? `[Function: ${truncate(name)}]`
                : "[Function (anonymous)]";
        }
        if (contains(path, value)) return "[Circular]";
        if (level >= MAX_DEPTH) return arrayIsArray(value) ? "[Array]" : "[Object]";
        append(path, value);
        try {
            return formatObject(value, level, path);
        } catch (error) {
            return `[uninspectable: ${truncate(safeMessage(error))}]`;
        } finally {
            path.length -= 1;
        }
    }

    function mismatch(path, reason, actual, expected) {
        return { path, reason, actual, expected };
    }

    function markPair(seen, actual, expected) {
        let expectedSet = mapGet(seen.map, actual);
        if (expectedSet === undefined) {
            expectedSet = new NativeSet();
            mapSet(seen.map, actual, expectedSet);
        }
        if (setHas(expectedSet, expected)) return true;
        setAdd(expectedSet, expected);
        append(seen.log, [actual, expected]);
        return false;
    }

    function rollback(seen, mark) {
        while (seen.log.length > mark) {
            const pair = seen.log[seen.log.length - 1];
            seen.log.length -= 1;
            const expectedSet = mapGet(seen.map, pair[0]);
            if (expectedSet !== undefined) {
                uncurry(setDelete, expectedSet, pair[1]);
                if (uncurry(setSize, expectedSet) === 0) {
                    uncurry(mapDelete, seen.map, pair[0]);
                }
            }
        }
    }

    /** Compare two values under the shared cycle-tracking state, rolling back on failure. */
    function sameValue(actual, expected, path, seen) {
        const mark = seen.log.length;
        if (findMismatch(actual, expected, path, seen) === null) return true;
        rollback(seen, mark);
        return false;
    }

    /**
     * Compare a Map key and value as one candidate, so a matching key whose
     * value differs leaves no cycle marks behind for the next candidate.
     */
    function sameEntry(actualEntry, expectedEntry, path, seen) {
        const mark = seen.log.length;
        if (
            findMismatch(actualEntry[0], expectedEntry[0], `${path}<Map key>`, seen) !== null ||
            findMismatch(actualEntry[1], expectedEntry[1], `${path}<Map value>`, seen) !== null
        ) {
            rollback(seen, mark);
            return false;
        }
        return true;
    }

    function kindOf(value) {
        if (arrayIsArray(value)) return "array";
        if (arrayBufferIsView(value)) {
            return NativeDataView && hasBrand(value, NativeDataView) ? "dataview" : "typedarray";
        }
        if (hasBrand(value, NativeDate)) return "date";
        if (hasBrand(value, NativeRegExp)) return "regexp";
        if (hasBrand(value, NativeMap)) return "map";
        if (hasBrand(value, NativeSet)) return "set";
        if (hasBrand(value, NativeArrayBuffer)) return "arraybuffer";
        if (NativeSharedArrayBuffer && hasBrand(value, NativeSharedArrayBuffer)) {
            return "sharedarraybuffer";
        }
        if (hasBrand(value, NativeBoolean)) return "boolean";
        if (hasBrand(value, NativeNumber)) return "number";
        if (hasBrand(value, NativeString)) return "string";
        if (NativeBigInt && hasBrand(value, NativeBigInt)) return "bigint";
        if (hasBrand(value, NativeSymbol)) return "symbol";
        if (hasBrand(value, NativeError)) return "error";
        if (hasBrand(value, NativePromise)) return "promise";
        if (hasBrand(value, NativeWeakMap)) return "weakmap";
        if (hasBrand(value, NativeWeakSet)) return "weakset";
        if (NativeWeakRef && hasBrand(value, NativeWeakRef)) return "weakref";
        return "object";
    }

    function bytesEqual(actual, expected) {
        if (actual.length !== expected.length) return false;
        for (let index = 0; index < actual.length; index += 1) {
            if (actual[index] !== expected[index]) return false;
        }
        return true;
    }

    function bufferBytes(value, length, byteOffset) {
        return new makeUint8Array(value, byteOffset || 0, length);
    }

    /**
     * Return the first structural difference, or `null` when the values are
     * deeply equal. `path` is a readable location in `actual`.
     */
    function findMismatch(actual, expected, path, seen) {
        if (objectIs(actual, expected)) return null;
        const actualObject = isObjectLike(actual);
        const expectedObject = isObjectLike(expected);
        if (!actualObject || !expectedObject) {
            return actualObject !== expectedObject
                ? mismatch(path, "an object and a primitive are never equal", actual, expected)
                : mismatch(path, "values differ", actual, expected);
        }
        if (typeof actual === "function" || typeof expected === "function") {
            return mismatch(path, "functions are compared by identity", actual, expected);
        }
        if (markPair(seen, actual, expected)) return null;
        let actualPrototype;
        let expectedPrototype;
        try {
            actualPrototype = getPrototypeOf(actual);
            expectedPrototype = getPrototypeOf(expected);
        } catch {
            return mismatch(path, "a prototype could not be read", actual, expected);
        }
        if (actualPrototype !== expectedPrototype) {
            return mismatch(path, "prototypes differ", actual, expected);
        }

        const actualKind = kindOf(actual);
        const expectedKind = kindOf(expected);
        if (actualKind !== expectedKind) {
            return mismatch(
                path,
                `values are different kinds (${actualKind} and ${expectedKind})`,
                actual,
                expected,
            );
        }

        try {
            if (actualKind === "array") {
                if (actual.length !== expected.length) {
                    return mismatch(path, "array lengths differ", actual, expected);
                }
            } else if (actualKind === "date") {
                if (!objectIs(uncurry(dateGetTime, actual), uncurry(dateGetTime, expected))) {
                    return mismatch(path, "dates differ", actual, expected);
                }
            } else if (actualKind === "regexp") {
                if (
                    uncurry(regexpSource, actual) !== uncurry(regexpSource, expected) ||
                    uncurry(regexpFlags, actual) !== uncurry(regexpFlags, expected) ||
                    !objectIs(actual.lastIndex, expected.lastIndex)
                ) {
                    return mismatch(path, "regular expressions differ", actual, expected);
                }
            } else if (actualKind === "map") {
                const actualSize = uncurry(mapSize, actual);
                if (actualSize !== uncurry(mapSize, expected)) {
                    return mismatch(path, "Map sizes differ", actual, expected);
                }
                const expectedEntries = [];
                uncurry(mapForEach, expected, function (entryValue, entryKey) {
                    append(expectedEntries, [entryKey, entryValue]);
                });
                const used = [];
                for (let index = 0; index < expectedEntries.length; index += 1) {
                    append(used, false);
                }
                let missing = false;
                uncurry(mapForEach, actual, function (entryValue, entryKey) {
                    if (missing) return;
                    for (let index = 0; index < expectedEntries.length; index += 1) {
                        if (used[index] === true) continue;
                        if (sameEntry([entryKey, entryValue], expectedEntries[index], path, seen)) {
                            defineIndex(used, index, true);
                            return;
                        }
                    }
                    missing = true;
                });
                if (missing) return mismatch(path, "Map entries differ", actual, expected);
            } else if (actualKind === "set") {
                const actualSize = uncurry(setSize, actual);
                if (actualSize !== uncurry(setSize, expected)) {
                    return mismatch(path, "Set sizes differ", actual, expected);
                }
                const expectedValues = [];
                uncurry(setForEach, expected, function (entryValue) {
                    append(expectedValues, entryValue);
                });
                const used = [];
                for (let index = 0; index < expectedValues.length; index += 1) {
                    append(used, false);
                }
                let missing = false;
                uncurry(setForEach, actual, function (entryValue) {
                    if (missing) return;
                    for (let index = 0; index < expectedValues.length; index += 1) {
                        if (used[index] === true) continue;
                        if (
                            sameValue(
                                entryValue,
                                expectedValues[index],
                                `${path}<Set value>`,
                                seen,
                            )
                        ) {
                            defineIndex(used, index, true);
                            return;
                        }
                    }
                    missing = true;
                });
                if (missing) return mismatch(path, "Set values differ", actual, expected);
            } else if (actualKind === "arraybuffer" || actualKind === "sharedarraybuffer") {
                const actualLength =
                    actualKind === "arraybuffer"
                        ? uncurry(arrayBufferByteLength, actual)
                        : uncurry(sharedArrayBufferByteLength, actual);
                const expectedLength =
                    actualKind === "arraybuffer"
                        ? uncurry(arrayBufferByteLength, expected)
                        : uncurry(sharedArrayBufferByteLength, expected);
                if (actualLength !== expectedLength) {
                    return mismatch(path, "buffer byte lengths differ", actual, expected);
                }
                if (!bytesEqual(bufferBytes(actual, actualLength), bufferBytes(expected, expectedLength))) {
                    return mismatch(path, "buffer bytes differ", actual, expected);
                }
            } else if (actualKind === "dataview") {
                const actualLength = uncurry(dataViewByteLength, actual);
                const expectedLength = uncurry(dataViewByteLength, expected);
                if (actualLength !== expectedLength) {
                    return mismatch(path, "DataView byte lengths differ", actual, expected);
                }
                const actualBytes = bufferBytes(
                    uncurry(dataViewBuffer, actual),
                    actualLength,
                    uncurry(dataViewByteOffset, actual),
                );
                const expectedBytes = bufferBytes(
                    uncurry(dataViewBuffer, expected),
                    expectedLength,
                    uncurry(dataViewByteOffset, expected),
                );
                if (!bytesEqual(actualBytes, expectedBytes)) {
                    return mismatch(path, "DataView bytes differ", actual, expected);
                }
            } else if (actualKind === "typedarray") {
                if (uncurry(typedArrayTag, actual) !== uncurry(typedArrayTag, expected)) {
                    return mismatch(path, "typed array types differ", actual, expected);
                }
                const actualLength = uncurry(typedArrayLength, actual);
                if (actualLength !== uncurry(typedArrayLength, expected)) {
                    return mismatch(path, "typed array lengths differ", actual, expected);
                }
                for (let index = 0; index < actualLength; index += 1) {
                    const elementMismatch = findMismatch(
                        actual[index],
                        expected[index],
                        `${path}[${index}]`,
                        seen,
                    );
                    if (elementMismatch !== null) return elementMismatch;
                }
            } else if (
                actualKind === "boolean" ||
                actualKind === "number" ||
                actualKind === "string" ||
                actualKind === "bigint" ||
                actualKind === "symbol"
            ) {
                const valueOf =
                    actualKind === "boolean"
                        ? booleanValueOf
                        : actualKind === "number"
                          ? numberValueOf
                          : actualKind === "string"
                            ? stringValueOf
                            : actualKind === "bigint"
                              ? bigintValueOf
                              : symbolValueOf;
                if (!objectIs(uncurry(valueOf, actual), uncurry(valueOf, expected))) {
                    return mismatch(path, "boxed primitive values differ", actual, expected);
                }
            } else if (actualKind === "error") {
                let actualName = actual.name;
                let expectedName = expected.name;
                let actualMessage = actual.message;
                let expectedMessage = expected.message;
                if (actualName !== expectedName) {
                    return mismatch(path, "error names differ", actual, expected);
                }
                if (actualMessage !== expectedMessage) {
                    return mismatch(path, "error messages differ", actual, expected);
                }
            } else if (
                actualKind === "promise" ||
                actualKind === "weakmap" ||
                actualKind === "weakset" ||
                actualKind === "weakref"
            ) {
                return mismatch(
                    path,
                    "Promise, WeakMap, WeakSet, and WeakRef values are compared by identity",
                    actual,
                    expected,
                );
            }
        } catch {
            return mismatch(path, `a ${actualKind} value could not be inspected`, actual, expected);
        }

        let actualKeys;
        let expectedKeys;
        try {
            actualKeys = ownKeys(actual);
            expectedKeys = ownKeys(expected);
        } catch {
            return mismatch(path, "own properties could not be read", actual, expected);
        }
        if (actualKeys.length !== expectedKeys.length) {
            return mismatch(
                path,
                "objects have different numbers of own enumerable properties",
                actual,
                expected,
            );
        }
        for (let index = 0; index < actualKeys.length; index += 1) {
            const key = actualKeys[index];
            let present;
            try {
                present = uncurry(propertyIsEnumerable, expected, key);
            } catch {
                return mismatch(
                    `${path}${formatKey(key)}`,
                    "own properties could not be read",
                    actual,
                    expected,
                );
            }
            if (!present) {
                return mismatch(
                    `${path}${formatKey(key)}`,
                    `property ${formatKey(key)} is missing`,
                    actual,
                    expected,
                );
            }
            let actualValue;
            let expectedValue;
            try {
                actualValue = actual[key];
                expectedValue = expected[key];
            } catch {
                return mismatch(
                    `${path}${formatKey(key)}`,
                    "a property could not be read",
                    actual,
                    expected,
                );
            }
            const propertyMismatch = findMismatch(
                actualValue,
                expectedValue,
                `${path}${formatKey(key)}`,
                seen,
            );
            if (propertyMismatch !== null) return propertyMismatch;
        }
        return null;
    }

    function describeMismatch(difference) {
        return (
            `values are not equal: ${difference.reason} at ${difference.path}` +
            `\nactual: ${formatValue(difference.actual)}` +
            `\nexpected: ${formatValue(difference.expected)}`
        );
    }

    function regexpText(regexp) {
        return `/${uncurry(regexpSource, regexp)}/${uncurry(regexpFlags, regexp)}`;
    }

    function checkExpected(expected) {
        if (expected === undefined || expected === null) return;
        if (hasBrand(expected, NativeRegExp)) {
            try {
                uncurry(regexpSource, expected);
                uncurry(regexpFlags, expected);
            } catch {
                throw new makeTypeError("expected must be a real RegExp, an error constructor, or omitted");
            }
            return;
        }
        if (typeof expected === "function") {
            let prototype;
            try {
                prototype = expected.prototype;
            } catch {
                prototype = undefined;
            }
            if (prototype !== null && typeof prototype === "object") return;
            throw new makeTypeError(
                "expected must be an error constructor, a RegExp, or omitted",
            );
        }
        throw new makeTypeError("expected must be an error constructor, a RegExp, or omitted");
    }

    function checkThrown(thrown, expected, message, operator) {
        if (expected === undefined || expected === null) return thrown;
        if (hasBrand(expected, NativeRegExp)) {
            const matcher = makeRegExp(uncurry(regexpSource, expected), uncurry(regexpFlags, expected));
            let text;
            try {
                text = hasBrand(thrown, NativeError)
                    ? NativeString(thrown.message)
                    : NativeString(thrown);
            } catch {
                text = "<unprintable thrown value>";
            }
            if (regexpExec(matcher, text) === null) {
                throw new AssertionError(
                    withMessage(
                        message,
                        `expected the thrown message to match ${regexpText(expected)}, ` +
                            `but it was ${formatValue(text)}`,
                    ),
                    { actual: text, expected, operator },
                );
            }
            return thrown;
        }
        if (!hasBrand(thrown, expected)) {
            const name =
                typeof expected.name === "string" && expected.name
                    ? expected.name
                    : "the expected error";
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the function to throw an instance of ${name}, ` +
                        `but it threw ${formatValue(thrown)}`,
                ),
                { actual: thrown, expected, operator },
            );
        }
        return thrown;
    }

    /**
     * Throw unless `value` is truthy.
     *
     * @param {unknown} value
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when `value` is falsy.
     */
    function assert(value, message) {
        checkArity(arguments, "assert", 1, 2);
        checkMessage(message);
        if (value) return;
        throw new AssertionError(
            withMessage(message, `expected a truthy value, got ${formatValue(value)}`),
            { actual: value, expected: true, operator: "assert" },
        );
    }

    /**
     * Throw unless the values are structurally equal.
     *
     * Equality is strict structural equality, not `==`: prototypes must match,
     * `NaN` equals `NaN`, `-0` differs from `0`, objects are compared by their
     * own enumerable properties and their built-in state, and cycles are
     * handled. See the module header for the complete rules.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when the values are not structurally equal.
     */
    function equals(actual, expected, message) {
        checkArity(arguments, "assert.equals", 2, 3);
        checkMessage(message);
        const difference = findMismatch(actual, expected, "$", {
            map: new NativeMap(),
            log: [],
        });
        if (difference === null) return;
        throw new AssertionError(withMessage(message, describeMismatch(difference)), {
            actual,
            expected,
            operator: "equals",
        });
    }

    /**
     * Throw when the values are structurally equal.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when the values are structurally equal.
     */
    function notEquals(actual, expected, message) {
        checkArity(arguments, "assert.notEquals", 2, 3);
        checkMessage(message);
        const difference = findMismatch(actual, expected, "$", {
            map: new NativeMap(),
            log: [],
        });
        if (difference !== null) return;
        throw new AssertionError(
            withMessage(
                message,
                `expected values to differ, but both are ${formatValue(actual)}`,
            ),
            { actual, expected, operator: "notEquals" },
        );
    }

    /**
     * Throw unless the values are the same value under `Object.is`.
     *
     * This is identity, not structural equality: two distinct objects are
     * never equal, `NaN` equals `NaN`, and `-0` differs from `0`.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when `Object.is(actual, expected)` is false.
     */
    function strictEquals(actual, expected, message) {
        checkArity(arguments, "assert.strictEquals", 2, 3);
        checkMessage(message);
        if (objectIs(actual, expected)) return;
        throw new AssertionError(
            withMessage(
                message,
                "expected values to be identical under Object.is" +
                    `\nactual: ${formatValue(actual)}` +
                    `\nexpected: ${formatValue(expected)}`,
            ),
            { actual, expected, operator: "strictEquals" },
        );
    }

    /**
     * Throw when the values are the same value under `Object.is`.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when `Object.is(actual, expected)` is true.
     */
    function notStrictEquals(actual, expected, message) {
        checkArity(arguments, "assert.notStrictEquals", 2, 3);
        checkMessage(message);
        if (!objectIs(actual, expected)) return;
        throw new AssertionError(
            withMessage(
                message,
                `expected values to differ under Object.is, but both are ${formatValue(actual)}`,
            ),
            { actual, expected, operator: "notStrictEquals" },
        );
    }

    /**
     * Throw an `AssertionError` unconditionally.
     *
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} always.
     */
    function fail(message) {
        checkArity(arguments, "assert.fail", 0, 1);
        checkMessage(message);
        throw new AssertionError(message === undefined ? "failed" : message, {
            operator: "fail",
        });
    }

    /**
     * Throw unless calling `callback` throws. Returns the thrown value.
     *
     * `expected` is an error constructor checked with the intrinsic
     * `instanceof`, a `RegExp` tested against the thrown error's message (or
     * the string form of a non-Error value), or omitted to accept any throw. A
     * string second argument is a `TypeError`, not a message; pass the message
     * third. The callback must be synchronous: a native Promise return is a
     * `TypeError` that points at `rejects`.
     *
     * @param {() => unknown} callback
     * @param {Function|RegExp|null} [expected]
     * @param {string} [message]
     * @throws {TypeError} for a bad argument or a Promise-returning callback.
     * @throws {AssertionError} when the callback does not throw or throws the wrong value.
     */
    function throws(callback, expected, message) {
        checkArity(arguments, "assert.throws", 1, 3);
        checkMessage(message);
        checkCallback(callback);
        checkExpected(expected);
        let didThrow = false;
        let thrown;
        let returned;
        try {
            returned = callback();
        } catch (error) {
            didThrow = true;
            thrown = error;
        }
        if (!didThrow) {
            if (isNativePromise(returned)) {
                settlePromise(returned);
                throw new makeTypeError(
                    "assert.throws expects a synchronous function; " +
                        "use assert.rejects for a Promise-returning function",
                );
            }
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the function to throw, but it returned ${formatValue(returned)}`,
                ),
                { actual: returned, operator: "throws" },
            );
        }
        return checkThrown(thrown, expected, message, "throws");
    }

    /**
     * Throw when calling `callback` throws, and otherwise return its result.
     *
     * The callback must be synchronous: a native Promise return is a
     * `TypeError` that points at `doesNotReject`.
     *
     * @param {() => unknown} callback
     * @param {string} [message]
     * @throws {TypeError} for a bad argument or a Promise-returning callback.
     * @throws {AssertionError} when the callback throws.
     */
    function doesNotThrow(callback, message) {
        checkArity(arguments, "assert.doesNotThrow", 1, 2);
        checkMessage(message);
        checkCallback(callback);
        let returned;
        try {
            returned = callback();
        } catch (error) {
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the function not to throw, but it threw ${formatValue(error)}`,
                ),
                { actual: error, operator: "doesNotThrow" },
            );
        }
        if (isNativePromise(returned)) {
            settlePromise(returned);
            throw new makeTypeError(
                "assert.doesNotThrow expects a synchronous function; " +
                    "use assert.doesNotReject for a Promise-returning function",
            );
        }
        return returned;
    }

    /**
     * Resolve with the rejection reason once `callback` rejects; reject with an
     * `AssertionError` when it fulfils.
     *
     * A synchronous throw is treated as a rejection. `expected` has the same
     * meaning as in `throws`. The returned Promise rejects for invalid
     * arguments. The callback's return value must be a native Promise or a
     * thenable; a plain value is an `AssertionError`.
     *
     * @param {() => unknown} callback
     * @param {Function|RegExp|null} [expected]
     * @param {string} [message]
     * @returns {Promise<unknown>} the rejection reason.
     */
    async function rejects(callback, expected, message) {
        checkArity(arguments, "assert.rejects", 1, 3);
        checkMessage(message);
        checkCallback(callback);
        checkExpected(expected);
        let result;
        try {
            result = callback();
        } catch (error) {
            return checkThrown(error, expected, message, "rejects");
        }
        if (!isThenable(result)) {
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the function to return a Promise, " +
                        `but it returned ${formatValue(result)}`,
                ),
                { actual: result, operator: "rejects" },
            );
        }
        let value;
        let fulfilled = false;
        try {
            value = await result;
            fulfilled = true;
        } catch (error) {
            return checkThrown(error, expected, message, "rejects");
        }
        if (fulfilled) {
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the Promise to reject, but it fulfilled with ${formatValue(value)}`,
                ),
                { actual: value, operator: "rejects" },
            );
        }
    }

    /**
     * Resolve with the fulfilment value once `callback` fulfils; reject with an
     * `AssertionError` when it throws or rejects.
     *
     * The callback's return value must be a native Promise or a thenable.
     *
     * @param {() => unknown} callback
     * @param {string} [message]
     * @returns {Promise<unknown>} the fulfilment value.
     */
    async function doesNotReject(callback, message) {
        checkArity(arguments, "assert.doesNotReject", 1, 2);
        checkMessage(message);
        checkCallback(callback);
        let result;
        try {
            result = callback();
        } catch (error) {
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the function not to throw or reject, " +
                        `but it threw ${formatValue(error)}`,
                ),
                { actual: error, operator: "doesNotReject" },
            );
        }
        if (!isThenable(result)) {
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the function to return a Promise, " +
                        `but it returned ${formatValue(result)}`,
                ),
                { actual: result, operator: "doesNotReject" },
            );
        }
        try {
            return await result;
        } catch (error) {
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the Promise not to reject, but it rejected with ${formatValue(error)}`,
                ),
                { actual: error, operator: "doesNotReject" },
            );
        }
    }

    // The module object is the callable `assert` itself. Its own properties are
    // the named assertions, and the module namespace re-exports the same
    // function objects. `ok` is the callable itself, as in Node's assert.
    defineField(assert, "AssertionError", AssertionError);
    defineField(assert, "ok", assert);
    defineField(assert, "equals", equals);
    defineField(assert, "notEquals", notEquals);
    defineField(assert, "strictEquals", strictEquals);
    defineField(assert, "notStrictEquals", notStrictEquals);
    defineField(assert, "fail", fail);
    defineField(assert, "throws", throws);
    defineField(assert, "doesNotThrow", doesNotThrow);
    defineField(assert, "rejects", rejects);
    defineField(assert, "doesNotReject", doesNotReject);
    Object.freeze(assert);

    // A Map avoids inherited-property lookups, so names such as `constructor`
    // and `__proto__` are unknown modules rather than functions.
    const modules = new Map([
        ["age", age],
        ["assert", assert],
        ["chars", chars],
        ["console", consoleModule],
        ["crypto", crypto],
        ["path", path],
        ["ssh", ssh],
        ["stdio", stdio],
        ["strings", strings],
    ]);

    /**
     * Return a built-in module by name, or `undefined` when the name is not
     * implemented. The same module object is returned for every lookup.
     *
     * @throws {TypeError} when the name is not a string.
     */
    function getBuiltInModule(name) {
        if (typeof name !== "string") {
            throw new makeTypeError("module name must be a string");
        }
        return mapGet(modules, name);
    }

    Object.defineProperty(globalThis, "Neo", {
        value: Object.freeze({ getBuiltInModule }),
        writable: false,
        enumerable: false,
        configurable: false,
    });
})();
