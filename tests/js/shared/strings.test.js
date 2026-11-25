import test from "vael:test";
import assert from "vael:assert";
import * as strings from "vael:strings";

test("vael:strings registry and module namespace share frozen exports", async () => {
    const registry = Vael.getBuiltInModule("vael:strings");
    const dynamic = await import("vael:strings");

    assert.ok(registry !== undefined && typeof registry === "object");
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(Vael.getBuiltInModule("vael:strings"), registry);
    assert.ok(strings !== registry, "namespace must be distinct from registry");
    assert.ok(!("default" in strings), "vael:strings must not have a default export");
    assert.strictEquals(dynamic.equalFold, strings.equalFold);

    const expected = [
        "endsWithFold",
        "equalFold",
        "indexOfFold",
        "lastIndexOfFold",
        "startsWithFold",
        "trim",
        "trimEnd",
        "trimStart",
    ];
    assert.equals(Object.keys(registry).sort(), expected);
    assert.equals(Object.keys(strings).sort(), expected);
    for (const name of expected) {
        assert.strictEquals(strings[name], registry[name]);
        assert.strictEquals(strings[name].name, name);
    }
});

test("equalFold, startsWithFold, and endsWithFold use Unicode 16.0 simple folding", () => {
    const foldCases = [
        ["Hello", "HELLO", true],
        ["café", "CAFÉ", true],
        ["K", "\u212a", true],
        ["S", "\u017f", true],
        ["\u03c3", "\u03c2", true],
        ["I", "\u0131", false],
        ["i", "\u0130", false],
        ["ss", "\u00df", false],
        ["a\ud800", "a\ud800", true],
        ["\ud83d", "\u{1f600}", false],
        ["a.b", "axb", false],
    ];
    for (const [left, right, expected] of foldCases) {
        assert.strictEquals(strings.equalFold(left, right), expected);
        assert.strictEquals(strings.equalFold(right, left), expected);
    }

    assert.strictEquals(strings.startsWithFold("Hello World", "HELLO"), true);
    assert.strictEquals(strings.startsWithFold("abc", ""), true);
    assert.strictEquals(strings.startsWithFold("(x)", "(x)"), true);
    assert.strictEquals(strings.endsWithFold("Hello World", "WORLD"), true);
    assert.strictEquals(strings.endsWithFold("a\n", "\n"), true);
    assert.strictEquals(strings.endsWithFold("a\n", "a"), false);
});

test("indexOfFold and lastIndexOfFold handle UTF-16 offsets, overlaps, and streaming windows", () => {
    assert.strictEquals(strings.indexOfFold("\u{1f600}a", "A"), 2);
    assert.strictEquals(strings.indexOfFold("\u{1f600}a", "a", 1), 2);
    assert.strictEquals(strings.indexOfFold("\u{1f600}", "\ude00"), -1);
    assert.strictEquals(strings.indexOfFold("aaab", "aab"), 1);
    assert.strictEquals(strings.indexOfFold("abc", "", Infinity), 3);

    assert.strictEquals(strings.lastIndexOfFold("aaaa", "aa"), 2);
    assert.strictEquals(strings.lastIndexOfFold("aAa", "a"), 2);
    assert.strictEquals(strings.lastIndexOfFold("\u212a\u212a", "k"), 1);
    assert.strictEquals(strings.lastIndexOfFold("abc", "", -Infinity), 0);

    // Surrogate pairs straddling 1024-code-unit streaming boundaries.
    const windowed = "a".repeat(1023) + "\u{1f600}" + "b".repeat(1022) + "\u{1f600}" + "c";
    assert.strictEquals(windowed.length, 2050);
    assert.strictEquals(strings.indexOfFold(windowed, "\u{1f600}", 0), 1023);
    assert.strictEquals(strings.indexOfFold(windowed, "\u{1f600}", 1024), 2047);
    assert.strictEquals(strings.lastIndexOfFold(windowed, "\u{1f600}", 1024), 1023);
    assert.strictEquals(strings.lastIndexOfFold(windowed, "\u{1f600}", 2046), 1023);
});

test("trim, trimStart, and trimEnd match contract and reject bad argument types", () => {
    assert.strictEquals(strings.trim("...hi...", "."), "hi");
    assert.strictEquals(strings.trim("xyhelloyx", "xy"), "hello");
    assert.strictEquals(strings.trim("\u{1f600}x\u{1f600}", "\u{1f600}"), "x");
    assert.strictEquals(strings.trimStart("///a/b", "/"), "a/b");
    assert.strictEquals(strings.trimEnd("a/b///", "/"), "a/b");

    const ws = "\ufeff\t a \n\u0085";
    assert.strictEquals(strings.trim(ws), ws.trim());
    assert.strictEquals(strings.trim("\ufeff a \ufeff"), "a");
    assert.strictEquals(strings.trim("\u0085a\u0085"), "\u0085a\u0085");

    assert.throws(() => strings.equalFold(1, "a"), TypeError);
    assert.throws(() => strings.startsWithFold("a", null), TypeError);
    assert.throws(() => strings.trim("a", 1), TypeError);
    assert.throws(() => strings.indexOfFold("a", "a", 1.5), RangeError);
    assert.throws(() => strings.indexOfFold("a", "a", NaN), RangeError);
});
