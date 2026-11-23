    // The implementation of the `strings` built-in, included in this realm shim
    // by `neo_shim` only when the `neo-strings` feature is enabled. It calls no
    // host operation and adds no capability beyond the strings it is given.
    // Keeping the implementation in its own source file is what lets a disabled
    // build exclude its code rather than only hiding its name.
    const makeSet = Set;
    const makeUint32Array = Uint32Array;
    const stringTrim = Function.prototype.call.bind(String.prototype.trim);
    const stringTrimStart = Function.prototype.call.bind(String.prototype.trimStart);
    const stringTrimEnd = Function.prototype.call.bind(String.prototype.trimEnd);

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

