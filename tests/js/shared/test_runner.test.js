import testDefault, { MAX_TESTS, TestFailureError, run, test } from "neo:test";
import assert from "neo:assert";

const executionOrder = [];

test("neo:test exports frozen registry and module bindings", async () => {
    const registry = Neo.getBuiltInModule("neo:test");
    const namespace = await import("neo:test");

    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(test, registry.test);
    assert.strictEquals(testDefault, registry.test);
    assert.strictEquals(run, registry.run);
    assert.strictEquals(MAX_TESTS, 10000);
    assert.strictEquals(registry.MAX_TESTS, MAX_TESTS);
    assert.strictEquals(TestFailureError, registry.TestFailureError);
    assert.strictEquals(namespace.test, registry.test);
    assert.ok(namespace !== registry, "namespace must be distinct from registry");
    executionOrder.push("surface");
});

test("synchronous test runs in registration order", () => {
    executionOrder.push("sync-1");
    assert.equals([1, 2], [1, 2]);
});

test("async microtask test settles before subsequent tests", async () => {
    const steps = [];
    steps.push("start");
    const value = await Promise.resolve(21).then((x) => {
        steps.push("microtask");
        return x * 2;
    });
    steps.push("end");
    assert.strictEquals(value, 42);
    assert.equals(steps, ["start", "microtask", "end"]);
    executionOrder.push("async-2");
});

test("custom thenable uses first settlement sequentially", () => {
    executionOrder.push("thenable-3");
    return {
        then(resolve, reject) {
            resolve({
                then(innerResolve) {
                    innerResolve("nested-ok");
                },
            });
            reject(new Error("second settlement must be ignored"));
        },
    };
});

test("tests execute strictly sequentially after earlier async work", () => {
    assert.equals(executionOrder, ["surface", "sync-1", "async-2", "thenable-3"]);
});

// Registration validation runs at top level before tests begin executing.
assert.throws(() => test(), TypeError);
assert.throws(() => test(""), TypeError);
assert.throws(() => test("   ", () => {}), TypeError);
assert.throws(() => test("not-a-fn", 123), TypeError);
assert.throws(() => test("bad-opts", () => {}, null), TypeError);
assert.throws(() => test("unknown-opt", () => {}, { extra: true }), TypeError);

function* topLevelGenerator() {
    yield 1;
}
const genErr = assert.throws(() => test("generator", topLevelGenerator), TypeError);
assert.strictEquals(genErr.code, "NEO_TEST_UNSUPPORTED");

test("registering a test while tests are running throws NEO_TEST_DURING_RUN", () => {
    const duringRun = assert.throws(() => test("nested registration", () => {}), Error);
    assert.strictEquals(duringRun.code, "NEO_TEST_DURING_RUN");
});

const awaitedValue = await Promise.resolve("after-tla");

test("tests registered after top-level await are executed", () => {
    assert.strictEquals(awaitedValue, "after-tla");
});
