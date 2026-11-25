import test from "vael:test";
import assert from "vael:assert";
import * as slices from "vael:slices";
import {
    CharSlice,
    ReadOnlyCharSlice,
    ReadOnlySlice,
    Slice,
    charSlice,
    readonlyCharSlice,
    readonlySlice,
    slice,
} from "vael:slices";

test("vael:slices registry and module namespace share frozen identity", async () => {
    const list = Vael.getBuiltInModulesList();
    assert.ok(list.includes("vael:slices"), `vael:slices missing from list: ${JSON.stringify(list)}`);
    assert.ok(!list.includes("slices"), "bare slices must not be listed");

    const registry = Vael.getBuiltInModule("vael:slices");
    assert.ok(registry !== undefined && typeof registry === "object");
    assert.strictEquals(Vael.getBuiltInModule("vael:slices"), registry);
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(Object.isExtensible(slices), false);
    assert.ok(!("default" in slices), "vael:slices must not have a default export");
    assert.strictEquals(Vael.getBuiltInModule("slices"), undefined);

    const dynamic = await import("vael:slices");
    assert.strictEquals(dynamic, slices);
    assert.ok(slices !== registry, "module namespace must be distinct from registry");

    const expected = [
        "CharSlice",
        "ReadOnlyCharSlice",
        "ReadOnlySlice",
        "Slice",
        "charSlice",
        "readonlyCharSlice",
        "readonlySlice",
        "slice",
    ];
    assert.equals(Object.keys(slices).sort(), expected);
    assert.equals(Object.keys(registry).sort(), expected);

    for (const name of expected) {
        assert.strictEquals(slices[name], registry[name], `${name} differs from registry`);
    }

    assert.throws(() => {
        slices.slice = () => "replaced";
    }, TypeError);

    await assert.rejects(() => import("slices"));
});

test("Slice and ReadOnlySlice provide live non-copying views over Arrays", () => {
    const base = [0, 1, 2, 3, 4];
    const outer = slice(base, 1, 5);
    assert.ok(outer instanceof Slice);
    assert.strictEquals(outer.length, 4);
    assert.strictEquals(outer.isEmpty, false);
    assert.strictEquals(outer.at(0), 1);
    assert.strictEquals(outer.at(3), 4);

    const inner = outer.slice(1, 3);
    assert.strictEquals(inner.length, 2);
    assert.strictEquals(inner.at(0), 2);
    assert.strictEquals(inner.at(1), 3);

    assert.strictEquals(inner.set(0, 99), inner);
    assert.strictEquals(base[2], 99);
    assert.strictEquals(outer.at(1), 99);
    assert.strictEquals(inner.at(0), 99);

    base[3] = 77;
    assert.strictEquals(inner.at(1), 77);

    const all = outer.slice();
    assert.strictEquals(all.length, outer.length);
    all.set(2, 55);
    assert.strictEquals(base[3], 55);

    const copy = inner.copy();
    assert.ok(Array.isArray(copy));
    assert.equals(copy, [99, 55]);
    copy[0] = -1;
    base[2] = 12;
    assert.strictEquals(copy[0], -1);
    assert.strictEquals(inner.at(0), 12);

    const ro = readonlySlice(base, 1, 4);
    assert.ok(ro instanceof ReadOnlySlice);
    assert.strictEquals(ro.set, undefined);
    assert.strictEquals(ro.at(0), 1);
    assert.strictEquals(ro.at(1), 12);
    base[2] = 13;
    assert.strictEquals(ro.at(1), 13);

    const nestedRo = ro.slice(1, 3);
    assert.ok(nestedRo instanceof ReadOnlySlice);
    assert.strictEquals(nestedRo.set, undefined);
    assert.equals(nestedRo.copy(), [13, 55]);

    assert.equals([...outer], [1, 13, 55, 4]);
    assert.strictEquals(outer.indexOf(13), 1);
    assert.strictEquals(outer.indexOf(999), -1);
    assert.strictEquals(outer.includes(55), true);
    assert.strictEquals(outer.includes(999), false);

    const nanView = slice([NaN, 1]);
    assert.strictEquals(nanView.indexOf(NaN), -1);
    assert.strictEquals(nanView.includes(NaN), true);

    assert.strictEquals(slice(base, base.length, base.length).length, 0);
    assert.strictEquals(new Slice([], 0, 0).isEmpty, true);

    // Holes, structural mutations, accessors, frozen arrays, proxies, species.
    const holes = [1, , 3];
    const holeView = slice(holes);
    assert.strictEquals(holeView.at(1), undefined);
    const holeCopy = holeView.copy();
    assert.ok(1 in holeCopy && holeCopy[1] === undefined);

    const frozen = Object.freeze([1, 2, 3]);
    const frozenView = slice(frozen, 1, 3);
    assert.strictEquals(frozenView.at(0), 2);
    assert.throws(() => frozenView.set(0, 9), TypeError);

    let proxyGets = 0;
    const proxy = new Proxy([10, 20, 30], {
        get(target, key, receiver) {
            if (typeof key === "string" && /^\d+$/.test(key)) proxyGets += 1;
            return Reflect.get(target, key, receiver);
        },
    });
    const proxyView = slice(proxy, 1, 3);
    assert.strictEquals(proxyView.at(0), 20);
    assert.ok(proxyGets > 0);
    assert.equals(proxyView.copy(), [20, 30]);

    class Evil extends Array {
        static get [Symbol.species]() {
            throw new Error("species was consulted");
        }
    }
    const evil = new Evil();
    evil.push(1, 2, 3);
    assert.equals(slice(evil, 1, 3).copy(), [2, 3]);

    assert.throws(() => slice(base, -1), RangeError);
    assert.throws(() => slice(base, 0, -1), RangeError);
    assert.throws(() => slice(base, 0.5), RangeError);
    assert.throws(() => slice(base, 3, 2), RangeError);
    assert.throws(() => slice(base, 0, base.length + 1), RangeError);
    assert.throws(() => slice(base, "0"), TypeError);
    assert.throws(() => outer.at(-1), RangeError);
    assert.throws(() => outer.at(outer.length), RangeError);
    assert.throws(() => outer.set("0", 1), TypeError);
    assert.throws(() => slice(null), TypeError);
    assert.throws(() => slice({ length: 0 }), TypeError);
    assert.throws(() => slice("abc"), TypeError);
});

test("Slice preserves TypedArray offsets, types, and rejects forged views", () => {
    const buffer = new ArrayBuffer(8);
    const first = new Uint8Array(buffer, 1, 4);
    const second = new Uint8Array(buffer);
    first.set([10, 11, 12, 13]);
    const view = slice(first, 1, 3);
    assert.strictEquals(view.length, 2);
    assert.strictEquals(view.at(0), 11);
    assert.strictEquals(view.at(1), 12);

    view.set(0, 200);
    assert.strictEquals(first[1], 200);
    assert.strictEquals(second[2], 200);

    const copy = view.copy();
    assert.ok(copy instanceof Uint8Array);
    assert.ok(copy.buffer !== buffer);
    assert.strictEquals(copy[0], 200);
    assert.strictEquals(copy[1], 12);

    const typedCases = [
        [new Int8Array([-1, 2]), "Int8Array"],
        [new Uint8ClampedArray([1, 300]), "Uint8ClampedArray"],
        [new Int16Array([-1, 2]), "Int16Array"],
        [new Uint16Array([1, 2]), "Uint16Array"],
        [new Int32Array([-1, 2]), "Int32Array"],
        [new Uint32Array([1, 2]), "Uint32Array"],
        [new Float32Array([1.5, 2.5]), "Float32Array"],
        [new Float64Array([1.5, 2.5]), "Float64Array"],
        [new BigInt64Array([1n, 2n]), "BigInt64Array"],
        [new BigUint64Array([1n, 2n]), "BigUint64Array"],
    ];
    for (const [source, name] of typedCases) {
        const cloned = slice(source).copy();
        assert.strictEquals(cloned.constructor.name, name);
        assert.strictEquals(cloned[0], source[0]);
    }

    const big = new BigInt64Array([1n, 2n, 3n]);
    const bigView = slice(big, 1, 3);
    assert.strictEquals(bigView.at(0), 2n);
    assert.throws(() => bigView.set(0, 1), TypeError);

    class EvilBytes extends Uint8Array {
        static get [Symbol.species]() {
            throw new Error("species was consulted");
        }
    }
    const evilBytes = new EvilBytes([4, 5, 6]);
    const evilCopy = slice(evilBytes, 1, 3).copy();
    assert.strictEquals(Object.getPrototypeOf(evilCopy), Uint8Array.prototype);

    assert.throws(() => slice(new DataView(new ArrayBuffer(4))), TypeError);
    assert.throws(() => slice(new Proxy(new Uint8Array(3), {})), TypeError);
    assert.throws(() => slice({ length: 3, 0: 1, [Symbol.toStringTag]: "Uint8Array" }), TypeError);
    assert.throws(() => slice(new Uint8Array(3), 0, 4), RangeError);

    if (typeof ArrayBuffer.prototype.transfer === "function") {
        const bytes = new Uint8Array([1, 2, 3]);
        const emptyAtTwo = slice(bytes, 2, 2);
        const emptyAtZero = slice(bytes, 0, 0);
        bytes.buffer.transfer();
        assert.throws(() => emptyAtTwo.copy(), RangeError);
        assert.strictEquals(emptyAtZero.copy().length, 0);
    }
});

test("CharSlice and ReadOnlyCharSlice handle Unicode code points and case mapping", () => {
    const chars = CharSlice.fromString("Hello, 😀!");
    assert.strictEquals(chars.length, 9);
    assert.strictEquals(chars.at(7), 0x1f600);
    assert.strictEquals(chars.at(0), 72);
    assert.strictEquals(chars.toString(), "Hello, 😀!");
    assert.strictEquals(chars.isEmpty, false);

    const lone = CharSlice.fromString("a\uD800b");
    assert.strictEquals(lone.length, 3);
    assert.strictEquals(lone.toString(), "a\uD800b");

    const text = CharSlice.fromString("abcdef");
    const middle = text.slice(1, 5);
    assert.strictEquals(middle.toString(), "bcde");
    assert.strictEquals(middle.toUpper(), middle);
    assert.strictEquals(middle.toString(), "BCDE");
    assert.strictEquals(text.toString(), "aBCDEf");

    const simple = CharSlice.fromString("ßé\uD800");
    simple.toUpper();
    assert.strictEquals(simple.toString(), "ßÉ\uD800");
    simple.toLower();
    assert.strictEquals(simple.toString(), "ßé\uD800");

    text.set(0, 90);
    assert.strictEquals(text.toString(), "ZBCDEf");
    assert.throws(() => text.set(0, "A"), TypeError);
    assert.throws(() => text.set(0, 65.5), RangeError);
    assert.throws(() => text.set(0, -1), RangeError);
    assert.throws(() => text.set(0, 0x110000), RangeError);

    const snapshot = text.toArray();
    assert.ok(snapshot instanceof Uint32Array);
    snapshot[0] = 65;
    assert.strictEquals(text.at(0), 90);

    const ro = readonlyCharSlice(text.toArray(), 0, 3);
    assert.ok(ro instanceof ReadOnlyCharSlice);
    assert.strictEquals(ro.set, undefined);
    assert.strictEquals(ro.toUpper, undefined);
    assert.strictEquals(ro.toString(), "ZBC");

    assert.throws(() => new CharSlice(new Uint32Array([0x110000])).toString(), RangeError);
    assert.throws(() => charSlice(new Uint8Array(3)), TypeError);
    assert.throws(() => charSlice([1, 2, 3]), TypeError);
    assert.throws(() => CharSlice.fromString(1), TypeError);

    // Tampered Uint32Array.prototype.buffer and Function.prototype.call must not leak or fail.
    const originalBuffer = Object.getOwnPropertyDescriptor(Uint32Array.prototype, "buffer");
    let bufferCalls = 0;
    Object.defineProperty(Uint32Array.prototype, "buffer", {
        configurable: true,
        get() {
            bufferCalls += 1;
            return null;
        },
    });
    try {
        const astral = CharSlice.fromString("a\u{1F600}b");
        assert.strictEquals(bufferCalls, 0);
        assert.strictEquals(astral.toString(), "a\u{1F600}b");
    } finally {
        if (originalBuffer) Object.defineProperty(Uint32Array.prototype, "buffer", originalBuffer);
        else delete Uint32Array.prototype.buffer;
    }

    const originalCall = Function.prototype.call;
    let callCount = 0;
    Function.prototype.call = function (...args) {
        callCount += 1;
        return Reflect.apply(originalCall, this, args);
    };
    try {
        const astral = ReadOnlyCharSlice.fromString("a\u{1F600}b");
        assert.strictEquals(callCount, 0);
        assert.strictEquals(astral.toString(), "a\u{1F600}b");
    } finally {
        Function.prototype.call = originalCall;
    }
});
