import test from "vael:test";
import assert from "vael:assert";
import * as chars from "vael:chars";

test("vael:chars registry and module namespace share frozen identity", async () => {
    const registry = Vael.getBuiltInModule("vael:chars");
    const dynamic = await import("vael:chars");
    const again = await import("vael:chars");

    assert.ok(registry !== undefined && typeof registry === "object");
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.ok(chars !== registry, "module namespace must be distinct from registry");
    assert.strictEquals(dynamic, again);
    assert.ok(!("default" in chars), "vael:chars must not have a default export");

    const exported = Object.keys(chars).sort();
    const registered = Object.keys(registry).sort();
    assert.equals(exported, registered);

    for (const name of registered) {
        assert.strictEquals(typeof chars[name], "function");
        assert.strictEquals(chars[name], registry[name]);
    }
});

test("predicates return false for non-scalar and non-number inputs", () => {
    const invalid = ["A", null, undefined, NaN, Infinity, -1, 1.5, 0x110000, 0xd800, 0xdfff];
    const predicates = [
        "isChar",
        "isAscii",
        "isLatin1",
        "isControl",
        "isDigit",
        "isLetter",
        "isLetterOrDigit",
        "isLower",
        "isUpper",
        "isPunc",
        "isSpace",
        "isSymbol",
    ];
    for (const name of predicates) {
        for (const value of invalid) {
            assert.strictEquals(chars[name](value), false, `${name}(${String(value)})`);
        }
    }
});

test("category classification and Unicode 16.0 boundary match contract", () => {
    const cases = [
        ["isChar", 0x0000, true],
        ["isChar", 0x10ffff, true],
        ["isAscii", 0x7f, true],
        ["isAscii", 0x80, false],
        ["isLatin1", 0xff, true],
        ["isLatin1", 0x100, false],
        ["isControl", 0x0a, true],
        ["isControl", 0x85, true],
        ["isControl", 0xa0, false],
        ["isDigit", 0x30, true],
        ["isDigit", 0x0663, true],
        ["isDigit", 0x2168, false],
        ["isLetter", 0x41, true],
        ["isLetter", 0x03bb, true],
        ["isLetter", 0x1c89, true],
        ["isUpper", 0x1c89, true],
        ["isLower", 0x1c8a, true],
        ["isLetterOrDigit", 0x5f, false],
        ["isPunc", 0x21, true],
        ["isSpace", 0x20, true],
        ["isSpace", 0x85, true],
        ["isSpace", 0x3000, true],
        ["isSpace", 0x200b, false],
        ["isSpace", 0xfeff, false],
        ["isSymbol", 0x1f600, true],
    ];
    for (const [name, cp, expected] of cases) {
        assert.strictEquals(chars[name](cp), expected, `${name}(${cp})`);
    }
});

test("toLower and toUpper map single code points and validate arguments", () => {
    assert.strictEquals(chars.toLower(0x41), 0x61);
    assert.strictEquals(chars.toUpper(0x61), 0x41);
    assert.strictEquals(chars.toLower(0x1c89), 0x1c8a);
    assert.strictEquals(chars.toUpper(0x1c8a), 0x1c89);
    // Expanding mappings return input unchanged.
    assert.strictEquals(chars.toUpper(0x00df), 0x00df);
    assert.strictEquals(chars.toLower(0x0130), 0x0130);

    for (const method of ["toLower", "toUpper"]) {
        for (const badType of ["A", null, undefined]) {
            assert.throws(() => chars[method](badType), TypeError);
        }
        for (const badRange of [NaN, Infinity, -1, 1.5, 0x110000, 0xd800, 0xdfff]) {
            assert.throws(() => chars[method](badRange), RangeError);
        }
    }
});
