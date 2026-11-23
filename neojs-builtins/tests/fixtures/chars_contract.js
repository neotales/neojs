// Shared `chars` cross-engine contract.
//
// Both `neojs-core/tests/chars_shared.rs` and
// `neojs-lite-core/tests/chars.rs` include this file and replace two tokens with
// the engine's adapter spellings:
//
//   __CHARS_SPECIFIER__      the import specifier the engine accepts
//   __CHARS_REGISTRY_KEY__   the key its registry currently uses
//
// The assertions below are about the module contract, not about a particular
// engine's legacy alias policy. Engine-specific tests cover the canonical
// registry and list rules.
import * as chars from "__CHARS_SPECIFIER__";

function fail(message) {
    throw new Error(message);
}

function equal(actual, expected, label) {
    if (!Object.is(actual, expected)) {
        fail(`${label}: expected ${String(expected)}, got ${String(actual)}`);
    }
}

function is(actual, label) {
    if (actual !== true) fail(`${label}: expected true, got ${String(actual)}`);
}

function isNot(actual, label) {
    if (actual !== false) fail(`${label}: expected false, got ${String(actual)}`);
}

const registry = Neo.getBuiltInModule("__CHARS_REGISTRY_KEY__");

// The registry exposes one frozen object with exactly the exported functions.
const exported = Object.keys(chars).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    fail(`export list ${exported} does not match registry ${registered}`);
}
if (registry === undefined || registry === null || typeof registry !== "object") {
    fail("the registry did not return the chars object");
}
if (chars === registry) {
    fail("the ES module namespace must be a distinct object from the registry object");
}
is(Object.isFrozen(registry), "the registry object is frozen");
if ("default" in chars) fail("chars must not have a default export");
for (const name of registered) {
    equal(typeof chars[name], "function", `${name} is a function`);
    equal(chars[name], registry[name], `${name} identity`);
}

// One module instance for static, dynamic, and repeated dynamic imports.
const dynamic = await import("__CHARS_SPECIFIER__");
const again = await import("__CHARS_SPECIFIER__");
equal(dynamic, again, "repeated dynamic import identity");
equal(dynamic.isLetter, chars.isLetter, "static and dynamic import identity");
equal(registry.isLetter, dynamic.isLetter, "dynamic import and registry identity");
if (dynamic === registry) {
    fail("the dynamic namespace must be a distinct object from the registry object");
}

// Scalar and category boundaries. These code points are long-assigned, so the
// contract does not depend on which Unicode release an engine ships.
const invalid = ["A", null, undefined, NaN, Infinity, -1, 1.5, 0x110000, 0xD800, 0xDFFF];
for (const name of [
    "isChar", "isAscii", "isLatin1", "isControl", "isDigit", "isLetter",
    "isLetterOrDigit", "isLower", "isUpper", "isPunc", "isSpace", "isSymbol",
]) {
    for (const value of invalid) {
        equal(chars[name](value), false, `${name}(${String(value)})`);
    }
}

const cases = [
    ["isChar", 0x0000, true],
    ["isChar", 0x10FFFF, true],
    ["isChar", 0xD800, false],
    ["isChar", 0xDFFF, false],
    ["isAscii", 0x7F, true],
    ["isAscii", 0x80, false],
    ["isLatin1", 0xFF, true],
    ["isLatin1", 0x100, false],
    ["isControl", 0x00, true],
    ["isControl", 0x0A, true],
    ["isControl", 0x7F, true],
    ["isControl", 0x85, true],
    ["isControl", 0x9F, true],
    ["isControl", 0xA0, false],
    ["isDigit", 0x30, true],
    ["isDigit", 0x0663, true],
    ["isDigit", 0x2168, false],
    ["isLetter", 0x41, true],
    ["isLetter", 0x03BB, true],
    ["isLetter", 0x0416, true],
    ["isLetter", 0x30, false],
    ["isLetterOrDigit", 0x30, true],
    ["isLetterOrDigit", 0x41, true],
    ["isLetterOrDigit", 0x5F, false],
    ["isLower", 0x61, true],
    ["isLower", 0x03B1, true],
    ["isLower", 0x41, false],
    ["isUpper", 0x41, true],
    ["isUpper", 0x0391, true],
    ["isUpper", 0x61, false],
    ["isPunc", 0x21, true],
    ["isPunc", 0x2C, true],
    ["isPunc", 0x24, false],
    ["isSpace", 0x20, true],
    ["isSpace", 0x09, true],
    ["isSpace", 0x85, true],
    ["isSpace", 0xA0, true],
    ["isSpace", 0x1680, true],
    ["isSpace", 0x3000, true],
    ["isSpace", 0x200B, false],
    ["isSpace", 0xFEFF, false],
    ["isSymbol", 0x2B, true],
    ["isSymbol", 0x20AC, true],
    ["isSymbol", 0x1F600, true],
    ["isSymbol", 0x41, false],
];
for (const [name, value, expected] of cases) {
    equal(chars[name](value), expected, `${name}(${value})`);
}

// One-code-point case mapping. Expanding mappings are returned unchanged.
const mapping = [
    ["toLower", 0x00, 0x00],
    ["toUpper", 0x00, 0x00],
    ["toLower", 0x41, 0x61],
    ["toLower", 0x61, 0x61],
    ["toUpper", 0x61, 0x41],
    ["toUpper", 0x41, 0x41],
    ["toLower", 0x0391, 0x03B1],
    ["toUpper", 0x03B1, 0x0391],
    ["toLower", 0x1E9E, 0x00DF],
    ["toLower", 0x0130, 0x0130],
    ["toUpper", 0x00DF, 0x00DF],
];
for (const [method, value, expected] of mapping) {
    equal(chars[method](value), expected, `${method}(${value})`);
}

const IntrinsicTypeError = TypeError;
const IntrinsicRangeError = RangeError;
for (const method of ["toLower", "toUpper"]) {
    for (const value of ["A", null, undefined, NaN, Infinity, -1, 1.5, 0x110000, 0xD800, 0xDFFF]) {
        let error;
        try {
            chars[method](value);
        } catch (thrown) {
            error = thrown;
        }
        const expected = typeof value === "number" ? IntrinsicRangeError : IntrinsicTypeError;
        if (!(error instanceof expected)) {
            fail(`${method}(${String(value)}) did not throw ${expected.name}`);
        }
    }
}

// Replacing the captured intrinsics after installation must not change the
// module. The counters prove the replacements were never consulted.
const originals = {
    exec: RegExp.prototype.exec,
    fromCodePoint: String.fromCodePoint,
    isInteger: Number.isInteger,
    toLowerCase: String.prototype.toLowerCase,
    toUpperCase: String.prototype.toUpperCase,
    freeze: Object.freeze,
};
let replacementCalls = 0;
RegExp.prototype.exec = function () {
    replacementCalls += 1;
    return null;
};
String.fromCodePoint = function () {
    replacementCalls += 1;
    return "";
};
Number.isInteger = function () {
    replacementCalls += 1;
    return false;
};
String.prototype.toLowerCase = function () {
    replacementCalls += 1;
    return "";
};
String.prototype.toUpperCase = function () {
    replacementCalls += 1;
    return "";
};
Object.freeze = function (value) {
    replacementCalls += 1;
    return value;
};
try {
    is(chars.isLetter(0x41), "isLetter after intrinsic replacement");
    is(chars.isDigit(0x30), "isDigit after intrinsic replacement");
    is(chars.isSpace(0x20), "isSpace after intrinsic replacement");
    is(chars.isSymbol(0x1F600), "isSymbol after intrinsic replacement");
    equal(chars.toLower(0x41), 0x61, "toLower after intrinsic replacement");
    equal(chars.toUpper(0x61), 0x41, "toUpper after intrinsic replacement");
} finally {
    RegExp.prototype.exec = originals.exec;
    String.fromCodePoint = originals.fromCodePoint;
    Number.isInteger = originals.isInteger;
    String.prototype.toLowerCase = originals.toLowerCase;
    String.prototype.toUpperCase = originals.toUpperCase;
    Object.freeze = originals.freeze;
}
equal(replacementCalls, 0, "a replaced intrinsic was consulted");
