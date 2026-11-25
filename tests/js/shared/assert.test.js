import test from "vael:test";
import assert, {
    AssertionError,
    assert as namedAssert,
    doesNotReject,
    doesNotThrow,
    equals,
    fail,
    notEquals,
    notStrictEquals,
    ok,
    rejects,
    strictEquals,
    throws,
} from "vael:assert";

test("vael:assert exports the frozen callable and named assertions", async () => {
    const registry = Vael.getBuiltInModule("vael:assert");
    const namespace = await import("vael:assert");

    strictEquals(typeof registry, "function");
    strictEquals(Object.isFrozen(registry), true);
    strictEquals(assert, registry);
    strictEquals(namedAssert, registry);
    strictEquals(ok, registry);
    strictEquals(registry.ok, registry);
    strictEquals(namespace.equals, equals);
    strictEquals(registry.equals, equals);
    strictEquals(registry.AssertionError, AssertionError);
    assert.ok(namespace !== registry, "namespace must be distinct from registry");
    assert.ok(!("assertEquals" in registry), "assertEquals must not exist");
    assert.ok(!("assertEquals" in namespace), "assertEquals export must not exist");
});

test("AssertionError carries VAEL_ASSERT code and comparison details", () => {
    const thrown = throws(
        () => equals({ a: { b: [0, 1] } }, { a: { b: [0, 2] } }, "parser check"),
        AssertionError,
    );
    assert.ok(thrown instanceof Error);
    strictEquals(thrown.name, "AssertionError");
    strictEquals(thrown.code, "VAEL_ASSERT");
    strictEquals(thrown.operator, "equals");
    strictEquals(thrown.actual.a.b[1], 1);
    strictEquals(thrown.expected.a.b[1], 2);
    assert.ok(thrown.message.startsWith("parser check: "));
    assert.ok(thrown.message.includes("$.a.b[1]"));

    const failed = throws(() => fail("explicit stop"), AssertionError);
    strictEquals(failed.operator, "fail");
    strictEquals(failed.message, "explicit stop");
});

test("strictEquals and notStrictEquals follow Object.is identity", () => {
    const shared = { id: 1 };
    strictEquals(shared, shared);
    strictEquals(NaN, NaN);
    strictEquals(Symbol.for("neo"), Symbol.for("neo"));

    notStrictEquals(0, -0);
    notStrictEquals({ id: 1 }, { id: 1 });
    notStrictEquals(Symbol("neo"), Symbol("neo"));

    throws(() => strictEquals(0, -0), AssertionError);
    throws(() => strictEquals({}, {}), AssertionError);
    throws(() => notStrictEquals(NaN, NaN), AssertionError);
});

test("equals and notEquals handle structural equality, collections, binary views, and cycles", () => {
    equals({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] });
    equals(NaN, NaN);
    notEquals(0, -0);
    notEquals(Object.create(null), {});
    notEquals([1, , 3], [1, undefined, 3]);

    const cycleA = { value: 1 };
    cycleA.self = cycleA;
    const cycleB = { value: 1 };
    cycleB.self = cycleB;
    equals(cycleA, cycleB);

    equals(
        new Map([
            ["a", 1],
            [{ k: 2 }, 3],
        ]),
        new Map([
            [{ k: 2 }, 3],
            ["a", 1],
        ]),
    );
    notEquals(new Map([["a", 1]]), new Map([["a", 2]]));

    equals(new Set([1, { a: 2 }]), new Set([{ a: 2 }, 1]));
    notEquals(new Set([1]), new Set([1, 2]));

    equals(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2, 3]));
    equals(new Float64Array([NaN, -0]), new Float64Array([NaN, -0]));
    equals(new BigInt64Array([1n, -2n]), new BigInt64Array([1n, -2n]));
    notEquals(new Float32Array([0]), new Float32Array([-0]));
    notEquals(new Uint8Array([1, 2]), new Uint8ClampedArray([1, 2]));

    const bytes = new Uint8Array([9, 1, 2, 8]);
    const other = new Uint8Array([0, 1, 2, 7]);
    equals(new DataView(bytes.buffer, 1, 2), new DataView(other.buffer, 1, 2));
    notEquals(new DataView(bytes.buffer, 1, 2), new DataView(bytes.buffer, 0, 2));

    equals(new Date(12345), new Date(12345));
    equals(new Date(NaN), new Date(NaN));
    equals(/abc/gi, /abc/gi);
    notEquals(/abc/g, /abc/i);
    equals(new TypeError("bad"), new TypeError("bad"));
    notEquals(new TypeError("bad"), new RangeError("bad"));
    notEquals(() => 1, () => 1);
    notEquals(Promise.resolve(1), Promise.resolve(1));
});

test("throws and doesNotThrow validate synchronous callbacks and reject native Promises", () => {
    const err = throws(() => {
        throw new RangeError("out of range");
    }, RangeError);
    strictEquals(err.message, "out of range");

    throws(() => {
        throw new Error("matched text");
    }, /matched/);

    strictEquals(doesNotThrow(() => 42), 42);

    throws(() => throws(() => 1), AssertionError);
    throws(() => throws(() => {
        throw new TypeError("wrong");
    }, RangeError), AssertionError);
    throws(() => doesNotThrow(() => {
        throw new Error("boom");
    }), AssertionError);

    // Genuine native Promises (including async functions) must fail with TypeError.
    throws(() => throws(async () => {}), TypeError);
    throws(() => throws(() => Promise.resolve(1)), TypeError);
    throws(() => doesNotThrow(async () => 1), TypeError);

    // An object merely inheriting from Promise.prototype lacks a Promise slot.
    const spoofed = Object.create(Promise.prototype);
    strictEquals(doesNotThrow(() => spoofed), spoofed);
    throws(() => throws(() => spoofed), AssertionError);

    // A Proxy around a Promise also lacks the internal Promise slot.
    const proxied = new Proxy(Promise.resolve(1), {});
    throws(() => throws(() => proxied), AssertionError);
});

test("rejects and doesNotReject handle async callbacks, thenables, and Promise subclasses", async () => {
    const asyncErr = await rejects(async () => {
        await Promise.resolve();
        throw new RangeError("async boom");
    }, RangeError);
    strictEquals(asyncErr.message, "async boom");

    const syncErr = await rejects(() => {
        throw new TypeError("sync boom");
    }, /sync/);
    strictEquals(syncErr.message, "sync boom");

    const fulfilled = await doesNotReject(async () => {
        await Promise.resolve();
        return "done";
    });
    strictEquals(fulfilled, "done");

    let reads = 0;
    const oneShotThenable = {
        get then() {
            reads += 1;
            return (resolve) => resolve(99);
        },
    };
    strictEquals(await doesNotReject(() => oneShotThenable), 99);
    strictEquals(reads, 1);

    await rejects(() => doesNotReject(async () => {
        throw new Error("unexpected reject");
    }), AssertionError);
    await rejects(() => rejects(async () => 123), AssertionError);
    await rejects(() => rejects(() => 123), AssertionError);
    await rejects(() => new Proxy(Promise.resolve(1), {}), TypeError);

    // Polluted Object.prototype.then must not turn a non-thenable into a pass
    // or interfere with native Promise observation.
    const hadOwnThen = Object.prototype.hasOwnProperty.call(Object.prototype, "then");
    const originalThen = Object.prototype.then;
    Object.prototype.then = function () {
        throw new Error("polluted then");
    };
    try {
        let pollutedRejectsError;
        try {
            await rejects(() => 42);
        } catch (caught) {
            pollutedRejectsError = caught;
        }
        assert.ok(pollutedRejectsError instanceof AssertionError);
        let pollutedDoesNotRejectError;
        try {
            await doesNotReject(() => 42);
        } catch (caught) {
            pollutedDoesNotRejectError = caught;
        }
        assert.ok(pollutedDoesNotRejectError instanceof AssertionError);
        strictEquals(await doesNotReject(() => Promise.resolve(7)), 7);
        strictEquals(await rejects(() => Promise.reject("raw")), "raw");
    } finally {
        if (hadOwnThen) Object.prototype.then = originalThen;
        else delete Object.prototype.then;
    }

    // Native Promise subclass observation must ignore overridden `then` and `Symbol.species`.
    let touched = 0;
    class EvilPromise extends Promise {
        static get [Symbol.species]() {
            touched += 1;
            throw new Error("species touched");
        }
        then() {
            touched += 1;
            throw new Error("then touched");
        }
    }

    strictEquals(
        await doesNotReject(() => new EvilPromise((resolve) => resolve("safe"))),
        "safe",
    );
    const subErr = await rejects(
        () => new EvilPromise((_, reject) => reject(new SyntaxError("sub reject"))),
        SyntaxError,
    );
    strictEquals(subErr.message, "sub reject");
    strictEquals(touched, 0);
});
