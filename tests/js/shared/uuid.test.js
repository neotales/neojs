import test from "neo:test";
import assert from "neo:assert";
import * as uuid from "neo:uuid";
import { v4, v7 } from "neo:uuid";
import v4Default, { v4 as namedV4 } from "neo:uuid/v4";
import v7Default, { v7 as namedV7 } from "neo:uuid/v7";

const CANONICAL = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const VERSION_4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const VERSION_7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

function timestampOf(value) {
    const hex = value.slice(0, 8) + value.slice(9, 13);
    return Number.parseInt(hex, 16);
}

test("neo:uuid and subpaths share frozen objects and function identity", async () => {
    const list = Neo.getBuiltInModulesList();
    for (const specifier of ["neo:uuid", "neo:uuid/v4", "neo:uuid/v7"]) {
        assert.ok(list.includes(specifier), `${specifier} missing from list`);
        assert.ok(!list.includes(specifier.slice("neo:".length)), `bare ${specifier} listed`);
    }

    const regUuid = Neo.getBuiltInModule("neo:uuid");
    const regV4 = Neo.getBuiltInModule("neo:uuid/v4");
    const regV7 = Neo.getBuiltInModule("neo:uuid/v7");
    const dynUuid = await import("neo:uuid");
    const dynV4 = await import("neo:uuid/v4");
    const dynV7 = await import("neo:uuid/v7");

    assert.ok(regUuid !== undefined && typeof regUuid === "object");
    assert.ok(regV4 !== undefined && typeof regV4 === "object");
    assert.ok(regV7 !== undefined && typeof regV7 === "object");
    assert.strictEquals(Neo.getBuiltInModule("neo:uuid"), regUuid);
    assert.strictEquals(Neo.getBuiltInModule("neo:uuid/v4"), regV4);
    assert.strictEquals(Neo.getBuiltInModule("neo:uuid/v7"), regV7);

    assert.strictEquals(Object.isFrozen(regUuid), true);
    assert.strictEquals(Object.isFrozen(regV4), true);
    assert.strictEquals(Object.isFrozen(regV7), true);
    assert.strictEquals(Object.isExtensible(uuid), false);
    assert.ok(uuid !== regUuid, "namespace must be distinct from registry");
    assert.strictEquals(dynUuid, uuid);
    assert.ok(!("default" in uuid), "neo:uuid must not have a default export");

    assert.equals(Object.keys(uuid).sort(), ["v4", "v7"]);
    assert.equals(Object.keys(regUuid).sort(), ["v4", "v7"]);
    assert.equals(Object.keys(dynV4).sort(), ["default", "v4"]);
    assert.equals(Object.keys(regV4).sort(), ["default", "v4"]);
    assert.equals(Object.keys(dynV7).sort(), ["default", "v7"]);
    assert.equals(Object.keys(regV7).sort(), ["default", "v7"]);

    // Exact function-object identity across all import paths and registry lookups.
    assert.strictEquals(typeof regUuid.v4, "function");
    assert.strictEquals(typeof regUuid.v7, "function");
    assert.strictEquals(v4, regUuid.v4);
    assert.strictEquals(uuid.v4, regUuid.v4);
    assert.strictEquals(namedV4, regUuid.v4);
    assert.strictEquals(v4Default, regUuid.v4);
    assert.strictEquals(dynV4.v4, regUuid.v4);
    assert.strictEquals(dynV4.default, regUuid.v4);
    assert.strictEquals(regV4.v4, regUuid.v4);
    assert.strictEquals(regV4.default, regUuid.v4);

    assert.strictEquals(v7, regUuid.v7);
    assert.strictEquals(uuid.v7, regUuid.v7);
    assert.strictEquals(namedV7, regUuid.v7);
    assert.strictEquals(v7Default, regUuid.v7);
    assert.strictEquals(dynV7.v7, regUuid.v7);
    assert.strictEquals(dynV7.default, regUuid.v7);
    assert.strictEquals(regV7.v7, regUuid.v7);
    assert.strictEquals(regV7.default, regUuid.v7);
});

test("v4 and v7 generate unique canonical RFC 9562 strings synchronously", () => {
    const fours = new Set();
    const sevens = new Set();

    for (let index = 0; index < 256; index += 1) {
        const four = v4();
        assert.strictEquals(typeof four, "string");
        assert.strictEquals(four.length, 36);
        assert.ok(CANONICAL.test(four), `v4 is not canonical: ${four}`);
        assert.ok(VERSION_4.test(four), `v4 version or variant bits wrong: ${four}`);
        assert.strictEquals(four, four.toLowerCase());
        fours.add(four);

        const seven = v7();
        assert.strictEquals(typeof seven, "string");
        assert.strictEquals(seven.length, 36);
        assert.ok(CANONICAL.test(seven), `v7 is not canonical: ${seven}`);
        assert.ok(VERSION_7.test(seven), `v7 version or variant bits wrong: ${seven}`);
        assert.strictEquals(seven, seven.toLowerCase());
        sevens.add(seven);
    }

    assert.strictEquals(fours.size, 256, "v4 repeated within 256 draws");
    assert.strictEquals(sevens.size, 256, "v7 repeated within 256 draws");
    assert.ok(typeof v4().then !== "function", "v4 must not return a thenable");
    assert.ok(typeof v7().then !== "function", "v7 must not return a thenable");
});

test("v7 embeds the 48-bit Unix millisecond timestamp and orders across milliseconds", () => {
    const before = Date.now();
    const current = v7();
    const after = Date.now();
    const decoded = timestampOf(current);
    assert.ok(
        decoded >= before - 1000 && decoded <= after + 1000,
        `decoded v7 timestamp ${decoded} not near ${before}..${after}`,
    );

    // Advance by at least one millisecond without timers (bounded spin).
    const startMs = Date.now();
    const earlier = v7();
    const deadlineMs = startMs + 50;
    while (Date.now() === startMs && Date.now() < deadlineMs) {
        // Spin briefly until Date.now() ticks to the next millisecond.
    }
    const afterTickMs = Date.now();
    if (afterTickMs > startMs) {
        const later = v7();
        assert.ok(
            timestampOf(later) > timestampOf(earlier),
            `expected later v7 timestamp (${timestampOf(later)}) > earlier (${timestampOf(earlier)})`,
        );
        assert.ok(earlier < later, `expected ${earlier} < ${later} across millisecond boundary`);
    }
});

test("v4 and v7 enforce zero-argument arity and frozen modules resist replacement", async () => {
    let called = false;
    for (const value of [undefined, null, 0, "", "v4", {}, [], () => { called = true; }]) {
        for (const fn of [v4, v4Default, v7, v7Default]) {
            const error = assert.throws(() => fn(value), TypeError);
            assert.ok(!("code" in error), "arity TypeError must not have code");
        }
    }
    assert.strictEquals(called, false, "callback argument must not be invoked");

    const regUuid = Neo.getBuiltInModule("neo:uuid");
    assert.throws(() => regUuid.v4(undefined), TypeError);
    assert.throws(() => regUuid.v7(undefined, undefined), TypeError);

    assert.throws(() => {
        uuid.v4 = () => "hijacked";
    }, TypeError);
    assert.throws(() => {
        Object.defineProperty(regUuid, "v4", { value: () => "hijacked" });
    }, TypeError);

    for (const bare of ["uuid", "uuid/v4", "uuid/v7"]) {
        assert.strictEquals(Neo.getBuiltInModule(bare), undefined);
        await assert.rejects(() => import(bare));
    }

    for (const name of ["uuid", "__neoUuidV4", "__neoUuidV7", "__neoUUID"]) {
        assert.ok(!(name in globalThis), `${name} leaked onto globalThis`);
    }
});
