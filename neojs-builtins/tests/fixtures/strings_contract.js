// Cross-engine contract for `neo:strings`.
//
// The V8 and QuickJS tests replace `__STRINGS_SPECIFIER__` and
// `__STRINGS_REGISTRY_KEY__` with the canonical `neo:strings` spelling. The
// fixture throws on the first divergence. It deliberately does not assert an
// engine-specific error class identity beyond `instanceof`, because both
// engines expose the language intrinsics.

import * as strings from "__STRINGS_SPECIFIER__";

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

const foldCases = [
    ["Hello", "HELLO", true, "ASCII case"],
    ["café", "CAFÉ", true, "accented Latin"],
    ["K", "\u212A", true, "Kelvin sign"],
    ["S", "\u017F", true, "long s"],
    ["\u03C3", "\u03C2", true, "sigma and final sigma"],
    ["I", "\u0131", false, "dotless i"],
    ["i", "\u0130", false, "dotted I"],
    ["ss", "\u00DF", false, "sharp s is full folding only"],
    ["a\n", "A\n", true, "newline inside the text"],
    ["a\uD800", "a\uD800", true, "the same lone surrogate"],
    ["a\uD800", "a", false, "a lone surrogate is not absent"],
    ["\uD83D", "\u{1F600}", false, "half a pair is not the pair"],
    ["\u{1F600}", "\u{1F600}", true, "the same astral character"],
    ["", "", true, "both empty"],
    ["", "\u0000", false, "empty is not NUL"],
    ["a.b", "a.b", true, "a literal dot"],
    ["a.b", "axb", false, "the dot is not a wildcard"],
    ["[a-z]", "[a-z]", true, "a literal character class"],
    ["\\d", "\\d", true, "a literal backslash"],
    ["a*", "aaa", false, "a literal star"],
    ["\uA7CE", "\uA7CF", false, "a Unicode 17 addition is identity in the pinned release"],
];
for (const [left, right, expected, description] of foldCases) {
    assert(strings.equalFold(left, right) === expected, `equalFold failed: ${description}`);
    assert(strings.equalFold(right, left) === expected, `equalFold is not symmetric: ${description}`);
}

assert(strings.startsWithFold("Hello World", "HELLO"), "startsWithFold missed");
assert(!strings.startsWithFold("Hello", "World"), "startsWithFold overmatched");
assert(strings.startsWithFold("abc", ""), "startsWithFold rejected an empty prefix");
assert(strings.startsWithFold("(x)", "(x)"), "startsWithFold treated a paren as a group");
assert(strings.startsWithFold("|x", "|"), "startsWithFold treated a pipe as alternation");

assert(strings.endsWithFold("Hello World", "WORLD"), "endsWithFold missed");
assert(!strings.endsWithFold("Hello", "Hell"), "endsWithFold overmatched");
assert(strings.endsWithFold("abc", ""), "endsWithFold rejected an empty suffix");
assert(!strings.endsWithFold("a\n", "a"), "endsWithFold matched before a final newline");
assert(strings.endsWithFold("a\n", "\n"), "endsWithFold missed a final newline");
assert(strings.endsWithFold("a\n", "a\n"), "endsWithFold missed the newline in the suffix");
assert(!strings.endsWithFold("a\r\n", "a"), "endsWithFold matched before CRLF");

const indexCases = [
    ["\u{1F600}a", "A", undefined, 2, "astral prefix counts two code units"],
    ["a\u{1F600}b", "B", undefined, 3, "astral in the middle"],
    ["abc", "z", undefined, -1, "absent"],
    ["abc", "", undefined, 0, "empty search at the start"],
    ["abc", "", 2, 2, "empty search at an index"],
    ["\u{1F600}", "", 1, 1, "empty search inside a surrogate pair"],
    ["\u{1F600}a", "a", 1, 2, "a search after a pair starts at the next boundary"],
    ["abc", "b", -5, 1, "a negative index clamps to zero"],
    ["abc", "c", Infinity, -1, "Infinity clamps to the length"],
    ["abc", "", Infinity, 3, "an empty search at Infinity is the length"],
    ["abc", "a", 99, -1, "an index above the length finds nothing"],
    ["x.a.y", "a.", undefined, 2, "regex metacharacters are literal"],
    ["aaaa", "aa", 0, 0, "overlapping first match"],
    ["aaab", "aab", undefined, 1, "KMP fallback after a failed prefix"],
    ["kK", "K\u212A", undefined, 0, "fold-equivalent code points in the prefix"],
    ["\u{1F600}a\u{1F600}a", "A\u{1F600}", undefined, 2, "astral match starts on a boundary"],
    ["\u{1F600}", "\uDE00", undefined, -1, "a lone surrogate never matches half a pair"],
];
const indexArguments = [
    ["\u{1F600}a", "A", undefined],
    ["a\u{1F600}b", "B", undefined],
    ["abc", "z", undefined],
    ["abc", "", undefined],
    ["abc", "", 2],
    ["\u{1F600}", "", 1],
    ["\u{1F600}a", "a", 1],
    ["abc", "b", -5],
    ["abc", "c", Infinity],
    ["abc", "", Infinity],
    ["abc", "a", 99],
    ["x.a.y", "a.", undefined],
    ["aaaa", "aa", 0],
    ["aaab", "aab", undefined],
    ["kK", "K\u212A", undefined],
    ["\u{1F600}a\u{1F600}a", "A\u{1F600}", undefined],
    ["\u{1F600}", "\uDE00", undefined],
];
for (let index = 0; index < indexCases.length; index += 1) {
    const [text, search, fromIndex, expected, description] = indexCases[index];
    const args = indexArguments[index];
    assert(
        strings.indexOfFold(...args) === expected,
        `indexOfFold failed: ${description}`,
    );
    assert(
        strings.indexOfFold(text, search, fromIndex) === expected,
        `indexOfFold explicit call failed: ${description}`,
    );
}

const lastCases = [
    ["aaa", "aa", undefined, 1, "overlapping occurrences"],
    ["aaaa", "aa", undefined, 2, "three overlapping occurrences"],
    ["aAa", "a", undefined, 2, "case-insensitive last occurrence"],
    ["a\u{1F600}b\u{1F600}", "B", undefined, 3, "astral text"],
    ["abc", "z", undefined, -1, "absent"],
    ["abc", "", undefined, 3, "empty search is the length"],
    ["abc", "", 1, 1, "empty search at an index"],
    ["aaaa", "aa", 2, 2, "a match starting at the limit"],
    ["aaaa", "aa", 1, 1, "a match starting below the limit"],
    ["aaa", "a", 0, 0, "a zero limit"],
    ["aaa", "a", -4, 0, "a negative limit clamps to zero"],
    ["\u212A\u212A", "k", undefined, 1, "folded last occurrence"],
    ["xababA", "abA", undefined, 3, "folded needle at the end"],
    ["\u{1F600}\u{1F600}", "\u{1F600}", undefined, 2, "astral last match"],
];
for (const [text, search, fromIndex, expected, description] of lastCases) {
    assert(
        strings.lastIndexOfFold(text, search, fromIndex) === expected,
        `lastIndexOfFold failed: ${description}`,
    );
}

const overlapText = "abababab";
for (let limit = -1; limit <= overlapText.length + 1; limit += 1) {
    assert(
        strings.lastIndexOfFold(overlapText, "abab", limit) ===
            overlapText.lastIndexOf("abab", limit),
        `overlap parity failed at ${limit}`,
    );
}

const trimCases = [
    ["...hi...", ".", "hi", "one character"],
    ["##a##", "#", "a", "hash"],
    ["xyhelloyx", "xy", "hello", "a set of characters"],
    ["\u{1F600}x\u{1F600}", "\u{1F600}", "x", "an astral code point"],
    ["  a  ", "", "  a  ", "an empty set trims nothing"],
    ["....", ".", "", "a fully trimmed string"],
    ["abc", ".", "abc", "no leading or trailing member"],
    ["", ".", "", "empty text"],
    ["a.b", ".", "a.b", "a member in the middle is kept"],
    ["\uD800x\uD800", "\uD800", "x", "a lone surrogate in the set"],
];
for (const [text, chars, expected, description] of trimCases) {
    assert(strings.trim(text, chars) === expected, `trim failed: ${description}`);
}
assert(strings.trimStart("///a/b", "/") === "a/b", "trimStart failed");
assert(strings.trimEnd("a/b///", "/") === "a/b", "trimEnd failed");
assert(strings.trimStart("\u{1F600}a", "\u{1F600}") === "a", "trimStart astral failed");
assert(strings.trimEnd("a\u{1F600}", "\u{1F600}") === "a", "trimEnd astral failed");

const jsWhitespace = "\uFEFF\t a \n\u0085";
assert(strings.trim(jsWhitespace) === jsWhitespace.trim(), "default trim differs from String.prototype.trim");
assert(strings.trim("\uFEFF a \uFEFF") === "a", "trim did not remove U+FEFF");
assert(strings.trim("\u0085a\u0085") === "\u0085a\u0085", "trim removed U+0085");
assert(strings.trimStart("  a  ") === "a  ", "trimStart default failed");
assert(strings.trimEnd("  a  ") === "  a", "trimEnd default failed");

const typeErrors = [
    () => strings.equalFold(1, "a"),
    () => strings.equalFold("a", 1),
    () => strings.startsWithFold("a", null),
    () => strings.endsWithFold(Symbol("x"), "a"),
    () => strings.indexOfFold("a", 1),
    () => strings.lastIndexOfFold("a", "a", "1"),
    () => strings.trim("a", 1),
    () => strings.trimStart(undefined, "a"),
    () => strings.trimEnd("a", {}),
];
for (const call of typeErrors) {
    let threw = false;
    try {
        call();
    } catch (error) {
        threw = error instanceof TypeError;
    }
    assert(threw, "a non-string argument was accepted");
}

for (const index of [1.5, NaN]) {
    let threw = false;
    try {
        strings.indexOfFold("a", "a", index);
    } catch (error) {
        threw = error instanceof RangeError;
    }
    assert(threw, `a non-integer fromIndex was accepted: ${index}`);
}

const registry = Neo.getBuiltInModule("__STRINGS_REGISTRY_KEY__");
assert(registry !== undefined, "the registry object is missing");
assert(registry === Neo.getBuiltInModule("__STRINGS_REGISTRY_KEY__"), "the lookup is not cached");
assert(Object.isFrozen(registry), "the registry object is not frozen");
assert(registry !== strings, "the ES module namespace must be distinct from the registry object");
const exported = Object.keys(strings).sort();
const registered = Object.keys(registry).sort();
assert(JSON.stringify(exported) === JSON.stringify(registered), "the export lists differ");
const expectedExports = [
    "endsWithFold", "equalFold", "indexOfFold", "lastIndexOfFold",
    "startsWithFold", "trim", "trimEnd", "trimStart",
];
const expectedArities = {
    endsWithFold: 2,
    equalFold: 2,
    indexOfFold: 3,
    lastIndexOfFold: 3,
    startsWithFold: 2,
    trim: 2,
    trimEnd: 2,
    trimStart: 2,
};
assert(JSON.stringify(registered) === JSON.stringify(expectedExports), `unexpected exports: ${registered}`);
for (const name of registered) {
    assert(typeof strings[name] === "function", `${name} is not a function`);
    assert(strings[name] === registry[name], `${name} is not the registry function`);
    assert(strings[name].length === expectedArities[name], `${name} has arity ${strings[name].length}`);
}
assert(!("default" in strings), "strings must not have a default export");

let rejected = false;
try {
    strings.trim = () => "replaced";
} catch (error) {
    rejected = error instanceof TypeError;
}
assert(rejected, "a namespace export was assignable");
assert(strings.trim("...x...", ".") === "x", "trim was replaced");
