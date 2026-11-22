//! Integration tests for `neo:crypto` Shamir secret sharing.
//!
//! These run real ECMAScript modules through the public `Runtime`, so they
//! exercise the module registry, the ESM bindings, the host ops, the V8 share
//! validation, the bounded worker pool, and the JavaScript result shape
//! together. The fixed share sets were produced independently with a Python
//! implementation of GF(256) reduction polynomial `0x11b`.

use std::fs;

use neojs_core::Runtime;

/// Writes a module into a fresh temporary directory and runs it.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn shamir_fixed_vectors_and_all_threshold_combinations() {
    run_js(
        r#"
import { shamirCombine, shamirSplit } from "neo:crypto";

function bytes(hex) {
    const out = new Uint8Array(hex.length / 2);
    for (let index = 0; index < out.length; index += 1) {
        out[index] = parseInt(hex.slice(index * 2, index * 2 + 2), 16);
    }
    return out;
}
function equal(label, actual, expected) {
    if (actual.length !== expected.length) throw new Error(label + " length");
    for (let index = 0; index < actual.length; index += 1) {
        if (actual[index] !== expected[index]) {
            throw new Error(label + " byte " + index + ": " + actual[index]);
        }
    }
}

// The published PLAB test vector: two one-byte shares reconstruct 0x42.
equal(
    "PLAB vector",
    shamirCombine([
        { index: 1, value: Uint8Array.of(0x51) },
        { index: 2, value: Uint8Array.of(0x64) },
    ]),
    Uint8Array.of(0x42),
);

// An independent 3-of-5 set for the five bytes 00 01 80 ff 42. Every subset of
// at least three shares reconstructs the secret; the first two do not, which is
// the documented threshold limitation rather than an error.
const fixed = [
    { index: 1, value: bytes("3376b30071") },
    { index: 2, value: bytes("aa6ca907f5") },
    { index: 3, value: bytes("991b9af8c6") },
    { index: 4, value: bytes("52e1f5605e") },
    { index: 5, value: bytes("6196c69f6d") },
];
const fixedSecret = bytes("000180ff42");
const combinations = [
    [0, 1, 2], [0, 1, 3], [0, 1, 4], [0, 2, 3], [0, 2, 4],
    [0, 3, 4], [1, 2, 3], [1, 2, 4], [1, 3, 4], [2, 3, 4],
    [0, 1, 2, 3], [0, 1, 2, 4], [0, 1, 3, 4], [0, 2, 3, 4],
    [1, 2, 3, 4], [0, 1, 2, 3, 4],
];
for (const combination of combinations) {
    equal(
        "fixed combination " + combination,
        shamirCombine(combination.map((index) => fixed[index])),
        fixedSecret,
    );
}
const twoOfThree = shamirCombine([fixed[0], fixed[1]]);
if (twoOfThree.length !== fixedSecret.length) throw new Error("two-share length");
let same = true;
for (let index = 0; index < fixedSecret.length; index += 1) {
    if (twoOfThree[index] !== fixedSecret[index]) same = false;
}
if (same) throw new Error("two shares unexpectedly recovered a 3-of-5 secret");

// The optional threshold rejects a set smaller than the caller's threshold,
// even though it cannot authenticate the shares themselves.
let range = false;
try {
    shamirCombine([fixed[0], fixed[1]], 3);
} catch (error) {
    range = error instanceof RangeError && !("code" in error);
}
if (!range) throw new Error("optional threshold did not reject two shares");

// A fresh split uses indexes 1..shareCount and every threshold subset works.
const secret = Uint8Array.from([0, 1, 2, 0, 255, 128, 7, 9, 10, 11]);
const shares = shamirSplit(secret, 3, 5);
if (!Array.isArray(shares) || shares.length !== 5) throw new Error("split count");
for (let index = 0; index < shares.length; index += 1) {
    const share = shares[index];
    if (share.index !== index + 1) throw new Error("share index " + index);
    if (!(share.value instanceof Uint8Array)) throw new Error("share value type");
    if (share.value.length !== secret.length) throw new Error("share value length");
    if (!Object.isFrozen(share)) throw new Error("share object is not frozen");
}
for (const combination of combinations) {
    const subset = combination.map((index) => shares[index]);
    equal(
        "fresh combination " + combination,
        shamirCombine(subset),
        secret,
    );
}
equal("fresh all shares", shamirCombine(shares), secret);

// Two-of-255 and a four-byte secret exercise the maximum index and a small
// share set without a large allocation.
const many = shamirSplit(Uint8Array.of(1, 2, 3, 4), 2, 255);
if (many.length !== 255 || many[254].index !== 255) throw new Error("255 shares");
equal("255-share combine", shamirCombine([many[0], many[254]]), Uint8Array.of(1, 2, 3, 4));

// The synchronous results are not thenables.
for (const value of [shares, shamirCombine(shares)]) {
    if (typeof value.then === "function" || value instanceof Promise) {
        throw new Error("synchronous Shamir returned a thenable");
    }
}
"#,
    );
}

#[test]
fn shamir_rejects_malformed_inputs_and_documented_limits() {
    run_js(
        r#"
import { shamirCombine, shamirSplit } from "neo:crypto";

function expectTypeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw new Error(label + ": not TypeError: " + error);
        if ("code" in error) throw new Error(label + ": TypeError carried a code");
        return;
    }
    throw new Error(label + ": expected TypeError");
}
function expectRangeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof RangeError)) throw new Error(label + ": not RangeError: " + error);
        if ("code" in error) throw new Error(label + ": RangeError carried a code");
        return;
    }
    throw new Error(label + ": expected RangeError");
}
function expectCode(fn, code, label) {
    try {
        fn();
    } catch (error) {
        if (error.code !== code) throw new Error(label + ": expected " + code + ", got " + error.code);
        return;
    }
    throw new Error(label + ": expected " + code);
}

const secret = Uint8Array.of(1, 2, 3);
const shares = shamirSplit(secret, 2, 3);

expectTypeError(() => shamirSplit(), "split no args");
expectTypeError(() => shamirSplit(secret, 2), "split two args");
expectTypeError(() => shamirSplit(secret, 2, 3, undefined), "split extra");
expectTypeError(() => shamirSplit("secret", 2, 3), "split string");
expectTypeError(() => shamirSplit(secret, "2", 3), "split threshold string");
expectTypeError(() => shamirSplit(secret, 2, "3"), "split count string");
expectTypeError(() => shamirSplit(new Proxy(secret, {}), 2, 3), "split proxy secret");
let coercionCalled = false;
const coercing = { toString() { coercionCalled = true; return "2"; } };
expectTypeError(() => shamirSplit(secret, coercing, 3), "split coercing threshold");
if (coercionCalled) throw new Error("threshold was coerced");
expectRangeError(() => shamirSplit(new Uint8Array(0), 2, 3), "split empty");
expectRangeError(() => shamirSplit(secret, 1, 3), "split threshold one");
expectRangeError(() => shamirSplit(secret, 256, 256), "split threshold 256");
expectRangeError(() => shamirSplit(secret, 3, 2), "split count below threshold");
expectRangeError(() => shamirSplit(secret, 2, 256), "split count 256");
expectRangeError(() => shamirSplit(secret, 2.5, 3), "split threshold fraction");
expectRangeError(() => shamirSplit(secret, 2, Infinity), "split count infinity");

// The secret limit is checked before the view is copied.
expectCode(
    () => shamirSplit(new Uint8Array(8 * 1024 * 1024 + 1), 2, 2),
    "NEO_CRYPTO_LIMIT",
    "split secret limit",
);
// The total share bytes are bounded independently of the secret maximum.
expectCode(
    () => shamirSplit(new Uint8Array(300000), 2, 255),
    "NEO_CRYPTO_LIMIT",
    "split total limit",
);

const detached = new Uint8Array(8);
detached.buffer.transfer();
expectCode(() => shamirSplit(detached, 2, 3), "NEO_CRYPTO_DETACHED", "split detached");

// A SharedArrayBuffer view cannot be copied as one coherent snapshot.
const shared = new SharedArrayBuffer(8);
const sharedView = new Uint8Array(shared);
sharedView.set([1, 2, 3]);
expectCode(
    () => shamirSplit(sharedView, 2, 3),
    "NEO_CRYPTO_SHAMIR_SHARED",
    "split shared",
);

expectTypeError(() => shamirCombine(), "combine no args");
expectTypeError(() => shamirCombine(shares, 2, 3), "combine extra");
expectTypeError(() => shamirCombine({}), "combine object");
expectTypeError(() => shamirCombine("shares"), "combine string");
expectTypeError(() => shamirCombine(new Proxy(shares, {})), "combine proxy array");
expectTypeError(() => shamirCombine([shares[0], null]), "combine null share");
expectTypeError(() => shamirCombine([shares[0], {}]), "combine empty share");
expectTypeError(() => shamirCombine([shares[0], { index: 2, value: [1] }]), "combine non-view");
expectTypeError(
    () => shamirCombine([shares[0], { index: "2", value: shares[1].value }]),
    "combine string index",
);
expectRangeError(() => shamirCombine([]), "combine empty array");
expectRangeError(() => shamirCombine([shares[0]]), "combine one share");
expectRangeError(() => shamirCombine([shares[0], { index: 0, value: shares[1].value }]), "combine zero index");
expectRangeError(() => shamirCombine([shares[0], { index: 256, value: shares[1].value }]), "combine index 256");
expectRangeError(
    () => shamirCombine([shares[0], { index: 3, value: shares[1].value }], 4),
    "combine threshold above count",
);
expectCode(
    () => shamirCombine([shares[0], { index: 1, value: shares[1].value }]),
    "NEO_CRYPTO_SHAMIR_DUPLICATE",
    "combine duplicate",
);
expectCode(
    () => shamirCombine([shares[0], { index: 2, value: Uint8Array.of(1) }]),
    "NEO_CRYPTO_SHAMIR_LENGTH",
    "combine length",
);
expectRangeError(
    () => shamirCombine([shares[0], { index: 2, value: new Uint8Array(0) }]),
    "combine empty value",
);
expectCode(
    () => shamirCombine([shares[0], { index: 2, value: detached }]),
    "NEO_CRYPTO_DETACHED",
    "combine detached",
);

// A share with an accessor is rejected without invoking the getter.
let getterCalled = false;
const accessor = { value: shares[1].value };
Object.defineProperty(accessor, "index", {
    get() { getterCalled = true; return 2; },
    enumerable: true,
});
expectTypeError(() => shamirCombine([shares[0], accessor]), "combine accessor index");
if (getterCalled) throw new Error("share getter was invoked");

// A Proxy share is rejected.
expectTypeError(
    () => shamirCombine([shares[0], new Proxy({ index: 2, value: shares[1].value }, {})]),
    "combine proxy share",
);

// An accessor on an array element is rejected without invoking it, and a hole
// is rejected rather than read as undefined.
let elementGetterCalled = false;
const accessorElement = [shares[0]];
Object.defineProperty(accessorElement, 1, {
    get() { elementGetterCalled = true; return shares[1]; },
    enumerable: true,
});
expectTypeError(() => shamirCombine(accessorElement), "combine accessor element");
if (elementGetterCalled) throw new Error("array element getter was invoked");
const hole = [shares[0]];
hole.length = 2;
expectTypeError(() => shamirCombine(hole), "combine array hole");

// A detached shares array value is rejected before any interpolation.
const detachedShares = [
    { index: 1, value: Uint8Array.of(4) },
    { index: 2, value: Uint8Array.of(5) },
];
detachedShares[1].value.buffer.transfer();
expectCode(() => shamirCombine(detachedShares), "NEO_CRYPTO_DETACHED", "combine detached share");

const sharedShares = [
    { index: 1, value: sharedView },
    { index: 2, value: Uint8Array.of(5) },
];
expectCode(
    () => shamirCombine(sharedShares),
    "NEO_CRYPTO_SHAMIR_SHARED",
    "combine shared share",
);
"#,
    );
}

#[test]
fn shamir_async_is_promise_based_and_copies_its_inputs() {
    run_js(
        r#"
import { shamirCombine, shamirCombineAsync, shamirSplit, shamirSplitAsync } from "neo:crypto";

function equal(label, actual, expected) {
    if (actual.length !== expected.length) throw new Error(label + " length");
    for (let index = 0; index < actual.length; index += 1) {
        if (actual[index] !== expected[index]) throw new Error(label + " byte " + index);
    }
}

const secret = Uint8Array.from([0, 1, 2, 0, 255, 128, 7, 9, 10, 11]);
const promise = shamirSplitAsync(secret, 3, 5);
if (!(promise instanceof Promise) || typeof promise.then !== "function") {
    throw new Error("shamirSplitAsync did not return a Promise");
}
secret.fill(0);
const shares = await promise;
if (shares.length !== 5) throw new Error("async split count");
for (let index = 0; index < shares.length; index += 1) {
    if (!Object.isFrozen(shares[index])) throw new Error("async share not frozen");
}
equal(
    "async split snapshot",
    shamirCombine([shares[0], shares[2], shares[4]]),
    Uint8Array.from([0, 1, 2, 0, 255, 128, 7, 9, 10, 11]),
);

// Async combine copies its input before returning the Promise.
const syncShares = shamirSplit(Uint8Array.of(0x42, 0x43), 2, 3);
const combinePromise = shamirCombineAsync([syncShares[0], syncShares[1]]);
if (!(combinePromise instanceof Promise)) throw new Error("combineAsync not a Promise");
syncShares[0].value.fill(0);
equal("async combine snapshot", await combinePromise, Uint8Array.of(0x42, 0x43));

// Several jobs can be pending at once.
const jobs = [];
for (let index = 0; index < 8; index += 1) {
    jobs.push(shamirSplitAsync(Uint8Array.of(index, index + 1), 2, 3));
}
const sets = await Promise.all(jobs);
const results = await Promise.all(
    sets.map((set, index) => shamirCombineAsync([set[0], set[2]], 2)),
);
for (let index = 0; index < results.length; index += 1) {
    equal("parallel " + index, results[index], Uint8Array.of(index, index + 1));
}

// The asynchronous APIs reject with the same argument errors.
let typeError = false;
try {
    await shamirSplitAsync("secret", 2, 3);
} catch (error) {
    typeError = error instanceof TypeError && !("code" in error);
}
if (!typeError) throw new Error("async split did not reject with TypeError");

let rangeError = false;
try {
    await shamirCombineAsync([syncShares[0]], 2);
} catch (error) {
    rangeError = error instanceof RangeError && !("code" in error);
}
if (!rangeError) throw new Error("async combine did not reject with RangeError");

let duplicate = false;
try {
    await shamirCombineAsync([syncShares[0], { index: syncShares[0].index, value: syncShares[1].value }]);
} catch (error) {
    duplicate = error.code === "NEO_CRYPTO_SHAMIR_DUPLICATE";
}
if (!duplicate) throw new Error("async combine did not reject duplicate shares");

const shared = new SharedArrayBuffer(3);
const sharedView = new Uint8Array(shared);
sharedView.set([1, 2, 3]);
let sharedSplit = "none";
try {
    await shamirSplitAsync(sharedView, 2, 3);
} catch (error) {
    sharedSplit = error.code;
}
if (sharedSplit !== "NEO_CRYPTO_SHAMIR_SHARED") {
    throw new Error("async split shared: " + sharedSplit);
}
let sharedCombine = "none";
try {
    await shamirCombineAsync([
        { index: 1, value: sharedView },
        { index: 2, value: Uint8Array.of(5) },
    ]);
} catch (error) {
    sharedCombine = error.code;
}
if (sharedCombine !== "NEO_CRYPTO_SHAMIR_SHARED") {
    throw new Error("async combine shared: " + sharedCombine);
}

// Finished joins are consumed by the take op, not left as resources.
for (const name of Object.values(Deno.core.resources())) {
    if (name === "neo::crypto::AsyncShamir") {
        throw new Error("an async Shamir job resource was retained");
    }
}
"#,
    );
}

#[test]
fn shamir_exports_are_the_registry_functions_and_resolve_canonically() {
    run_js(
        r#"
import * as crypto from "crypto";
import {
    shamirCombine as combine,
    shamirCombineAsync as combineAsync,
    shamirSplit as split,
    shamirSplitAsync as splitAsync,
} from "neo:crypto";

const registry = Neo.getBuiltInModule("crypto");
const names = ["shamirSplit", "shamirSplitAsync", "shamirCombine", "shamirCombineAsync"];
for (const name of names) {
    if (typeof registry[name] !== "function") throw new Error(name + " missing");
    if (crypto[name] !== registry[name]) throw new Error(name + " identity");
}
if (split !== registry.shamirSplit) throw new Error("named split identity");
if (splitAsync !== registry.shamirSplitAsync) throw new Error("named splitAsync identity");
if (combine !== registry.shamirCombine) throw new Error("named combine identity");
if (combineAsync !== registry.shamirCombineAsync) throw new Error("named combineAsync identity");
if ("default" in crypto) throw new Error("crypto must not have a default export");
if (import.meta.resolve("crypto") !== "neo:crypto") throw new Error("resolve");
if (import.meta.resolve("neo:crypto") !== "neo:crypto") throw new Error("alias resolve");
"#,
    );
}

#[test]
fn shamir_ignores_poisoned_array_index_setters() {
    run_js(
        r#"
import { shamirCombine, shamirSplit, shamirSplitAsync } from "neo:crypto";

const secret = Uint8Array.of(1, 2, 3, 4);
const original = Object.getOwnPropertyDescriptor(Array.prototype, "0");
let poisoned = 0;
Object.defineProperty(Array.prototype, "0", {
    configurable: true,
    get() { return undefined; },
    set() { poisoned += 1; },
});
try {
    const syncShares = shamirSplit(secret, 2, 3);
    if (poisoned !== 0) throw new Error("sync split used the prototype setter");
    if (syncShares[0].index !== 1) throw new Error("sync share 0 was intercepted");
    if (shamirCombine([syncShares[0], syncShares[2]])[0] !== 1) {
        throw new Error("sync combine after poisoning");
    }

    const asyncShares = await shamirSplitAsync(secret, 2, 3);
    if (poisoned !== 0) throw new Error("async split used the prototype setter");
    if (asyncShares[0].index !== 1) throw new Error("async share 0 was intercepted");
    if (shamirCombine([asyncShares[1], asyncShares[2]])[0] !== 1) {
        throw new Error("async combine after poisoning");
    }
} finally {
    if (original === undefined) {
        delete Array.prototype[0];
    } else {
        Object.defineProperty(Array.prototype, "0", original);
    }
}
"#,
    );
}
