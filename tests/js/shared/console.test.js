import test from "neo:test";
import assert from "neo:assert";
import defaultConsole, { debug, error, info, log, warn } from "neo:console";
import * as consoleModule from "neo:console";

test("neo:console exports the live global console and its five methods", async () => {
    const list = Neo.getBuiltInModulesList();
    assert.ok(list.includes("neo:console"), `neo:console missing from list: ${JSON.stringify(list)}`);
    assert.ok(!list.includes("console"), "bare console must not be listed");

    const registry = Neo.getBuiltInModule("neo:console");
    assert.strictEquals(registry, globalThis.console);
    assert.strictEquals(defaultConsole, globalThis.console);
    assert.strictEquals(consoleModule.default, globalThis.console);
    assert.strictEquals(Object.isFrozen(registry), false);
    assert.strictEquals(Neo.getBuiltInModule("console"), undefined);

    const dynamic = await import("neo:console");
    assert.strictEquals(dynamic, consoleModule);

    assert.equals(Object.keys(consoleModule).sort(), [
        "debug",
        "default",
        "error",
        "info",
        "log",
        "warn",
    ]);

    const named = { log, debug, info, warn, error };
    for (const name of ["log", "debug", "info", "warn", "error"]) {
        assert.strictEquals(typeof consoleModule[name], "function");
        assert.strictEquals(consoleModule[name], globalThis.console[name]);
        assert.strictEquals(registry[name], consoleModule[name]);
        assert.strictEquals(named[name], consoleModule[name]);
    }

    await assert.rejects(() => import("console"));
});

test("neo:console default export is live while named exports capture evaluation-time bindings", () => {
    const registry = Neo.getBuiltInModule("neo:console");
    const original = defaultConsole.log;
    let replacedCalls = 0;
    const replacement = () => {
        replacedCalls += 1;
    };

    defaultConsole.log = replacement;
    try {
        assert.strictEquals(globalThis.console.log, replacement);
        assert.strictEquals(registry.log, replacement);
        assert.strictEquals(consoleModule.default.log, replacement);
        assert.strictEquals(log, original);
        assert.strictEquals(consoleModule.log, original);

        globalThis.console.log("intercepted");
        assert.strictEquals(replacedCalls, 1);
    } finally {
        defaultConsole.log = original;
    }

    assert.strictEquals(globalThis.console.log, original);
    assert.strictEquals(registry.log, original);

    assert.throws(() => {
        consoleModule.log = replacement;
    }, TypeError);

    log.marker = 42;
    assert.strictEquals(globalThis.console.log.marker, 42);
    assert.strictEquals(registry.log.marker, 42);
    delete log.marker;
    assert.ok(!("marker" in original));
});

test("neo:console registry keeps the installed console when globalThis.console is replaced", () => {
    const installed = globalThis.console;
    const registry = Neo.getBuiltInModule("neo:console");
    const replacement = { log() {}, error() {} };

    globalThis.console = replacement;
    try {
        assert.strictEquals(globalThis.console, replacement);
        assert.strictEquals(registry, installed);
        assert.strictEquals(defaultConsole, installed);
        assert.strictEquals(Neo.getBuiltInModule("neo:console"), installed);
    } finally {
        globalThis.console = installed;
    }
    assert.strictEquals(Neo.getBuiltInModule("neo:console"), globalThis.console);
});

test("neo:console methods format all JS value kinds without throwing", () => {
    const circular = {};
    circular.self = circular;

    const values = [
        "text",
        42,
        true,
        null,
        undefined,
        123n,
        Symbol("s"),
        function namedFn() {},
        new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
        new BigInt64Array([1n, 2n]),
        new DataView(new ArrayBuffer(4)),
        { ok: true, nested: [1, 2] },
        circular,
    ];

    for (const fn of [log, debug, info, warn, error]) {
        fn();
        fn(...values);
    }
});

test("neo:console resists hostile prototypes, null-proto circular/typed values, throwing proxies, and lone surrogates", () => {
    const circularNull = Object.create(null);
    circularNull.self = circularNull;

    const nullProtoTyped = Object.setPrototypeOf(new Uint8Array([7, 8, 9]), null);

    const mutatedDataView = new DataView(new ArrayBuffer(4));
    Object.setPrototypeOf(mutatedDataView, {
        length: 2,
        constructor: { name: "EvilView" },
    });

    const throwingProxy = new Proxy(
        {},
        {
            get() {
                throw new Error("proxy get trap");
            },
            getPrototypeOf() {
                throw new Error("proxy proto trap");
            },
            ownKeys() {
                throw new Error("proxy keys trap");
            },
        },
    );

    const originalToString = Object.prototype.toString;
    const originalZero = Object.getOwnPropertyDescriptor(Object.prototype, "0");
    const originalSpecies = Object.getOwnPropertyDescriptor(Array, Symbol.species);

    Object.prototype.toString = () => {
        throw new Error("tampered Object.prototype.toString");
    };
    Object.defineProperty(Object.prototype, "0", {
        configurable: true,
        set() {
            throw new Error("tampered Object.prototype[0] setter");
        },
    });
    Object.defineProperty(Array, Symbol.species, {
        configurable: true,
        get() {
            throw new Error("tampered Array[Symbol.species]");
        },
    });

    try {
        for (const fn of [log, debug, info, warn, error]) {
            fn(circularNull);
            fn(nullProtoTyped);
            fn(mutatedDataView);
            fn(throwingProxy);
            fn("a\uD800b");
            fn(new Uint8Array([1, 2]), circularNull, "a\uD800b");
        }
    } finally {
        Object.prototype.toString = originalToString;
        if (originalZero) Object.defineProperty(Object.prototype, "0", originalZero);
        else delete Object.prototype["0"];
        if (originalSpecies) Object.defineProperty(Array, Symbol.species, originalSpecies);
        else delete Array[Symbol.species];
    }
});
