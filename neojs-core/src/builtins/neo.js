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

    /**
     * Find the first occurrence of `search` in `text`, ignoring case.
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
        let index = codepointStart(text, from);
        while (index < text.length) {
            if (foldMatchAt(text, index, search)) return index;
            index += codepointWidth(codePointAt(text, index));
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
        let last = -1;
        let index = 0;
        while (index <= limit && index < text.length) {
            if (foldMatchAt(text, index, search)) last = index;
            index += codepointWidth(codePointAt(text, index));
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

    // A Map avoids inherited-property lookups, so names such as `constructor`
    // and `__proto__` are unknown modules rather than functions.
    const modules = new Map([
        ["age", age],
        ["chars", chars],
        ["console", consoleModule],
        ["crypto", crypto],
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
