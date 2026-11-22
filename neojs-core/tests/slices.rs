//! Integration tests for the `slices` built-in, driven through `Runtime`.

use std::fs;
use std::path::PathBuf;

use neojs_core::Runtime;

fn write_module(dir: &tempfile::TempDir, name: &str, source: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, source).expect("write source");
    path
}

/// Runs a JavaScript module whose own assertions throw on failure.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", source);
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn slices_module_identity_and_exports() {
    run_js(
        r#"
import * as slices from "slices";
import * as alias from "neo:slices";
import * as upper from "NEO:slices";

const registry = Neo.getBuiltInModule("slices");
if (registry !== Neo.getBuiltInModule("slices")) {
    throw new Error("slices lookup is not cached");
}
if (!Object.isFrozen(registry)) throw new Error("slices is not frozen");
if (Neo.getBuiltInModule("neo:slices") !== undefined) {
    throw new Error("the registry accepted the prefixed spelling");
}
if (alias !== slices) throw new Error("slices and neo:slices differ");
if (upper !== slices) throw new Error("the scheme is not case-insensitive");
if (import.meta.resolve("slices") !== "neo:slices") {
    throw new Error("slices did not resolve canonically");
}
if (import.meta.resolve("neo:slices") !== "neo:slices") {
    throw new Error("neo:slices did not resolve canonically");
}

const exported = Object.keys(slices).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`export list ${exported} does not match ${registered}`);
}
const expected = [
    "CharSlice", "ReadOnlyCharSlice", "ReadOnlySlice", "Slice",
    "charSlice", "readonlyCharSlice", "readonlySlice", "slice",
];
if (JSON.stringify(registered) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exports: ${registered}`);
}
for (const name of registered) {
    if (slices[name] !== registry[name]) {
        throw new Error(`${name} is not the registry object`);
    }
}
if (typeof slices.Slice !== "function") throw new Error("Slice is not a class");
if (typeof slices.ReadOnlySlice !== "function") throw new Error("ReadOnlySlice is not a class");
if (typeof slices.CharSlice !== "function") throw new Error("CharSlice is not a class");
if (typeof slices.ReadOnlyCharSlice !== "function") {
    throw new Error("ReadOnlyCharSlice is not a class");
}
if (typeof slices.slice !== "function") throw new Error("slice is not a function");
if (typeof slices.readonlySlice !== "function") {
    throw new Error("readonlySlice is not a function");
}
if (typeof slices.charSlice !== "function") throw new Error("charSlice is not a function");
if (typeof slices.readonlyCharSlice !== "function") {
    throw new Error("readonlyCharSlice is not a function");
}
if ("default" in slices) throw new Error("slices must not have a default export");

let rejected = false;
try {
    slices.slice = () => "replaced";
} catch (error) {
    rejected = error instanceof TypeError;
}
if (!rejected) throw new Error("a namespace export was assignable");
if (slices.slice([1], 0, 1).at(0) !== 1) throw new Error("slice was replaced");
"#,
    );
}

#[test]
fn array_views_alias_and_nest_without_copying() {
    run_js(
        r#"
import { ReadOnlySlice, Slice, readonlySlice, slice } from "slices";

const base = [0, 1, 2, 3, 4];
const outer = slice(base, 1, 5);
if (!(outer instanceof Slice)) throw new Error("slice did not return a Slice");
if (outer.length !== 4 || outer.isEmpty) throw new Error("outer length is wrong");
if (outer.at(0) !== 1 || outer.at(3) !== 4) throw new Error("outer reads are wrong");

// Nested offsets are relative to the parent, not treated as lengths.
const inner = outer.slice(1, 3);
if (inner.length !== 2) throw new Error(`inner length ${inner.length}`);
if (inner.at(0) !== 2 || inner.at(1) !== 3) throw new Error("nested offset is wrong");

inner.set(0, 99);
if (base[2] !== 99) throw new Error("nested write did not reach the base");
if (outer.at(1) !== 99) throw new Error("parent did not see the nested write");
if (inner.at(0) !== 99) throw new Error("the view did not see its own write");

base[3] = 77;
if (inner.at(1) !== 77) throw new Error("direct base write was not visible");

// slice() with no bounds is a full alias, not a copy.
const all = outer.slice();
if (all.length !== outer.length || all.at(2) !== 77) throw new Error("slice() is not a full view");
all.set(2, 55);
if (base[3] !== 55) throw new Error("full view did not alias");

// A snapshot is independent in both directions.
const copy = inner.copy();
if (!Array.isArray(copy) || copy.length !== 2) throw new Error("copy is not a plain Array");
if (copy[0] !== 99 || copy[1] !== 55) throw new Error("copy values are wrong");
copy[0] = -1;
base[2] = 12;
if (copy[0] !== -1 || inner.at(0) !== 12) throw new Error("copy aliases the base");

// Read-only views read live data and have no set.
const readonly = readonlySlice(base, 1, 4);
if (!(readonly instanceof ReadOnlySlice)) throw new Error("readonlySlice returned the wrong class");
if (typeof readonly.set !== "undefined") throw new Error("read-only view exposes set");
if (readonly.at(0) !== 1 || readonly.at(1) !== 12) throw new Error("read-only reads are wrong");
base[2] = 13;
if (readonly.at(1) !== 13) throw new Error("read-only view is not live");
const nestedReadOnly = readonly.slice(1, 3);
if (!(nestedReadOnly instanceof ReadOnlySlice)) throw new Error("nested read-only class is wrong");
if (nestedReadOnly.length !== 2 || nestedReadOnly.at(0) !== 13 || nestedReadOnly.at(1) !== 55) {
    throw new Error("nested read-only view is wrong");
}
if (typeof nestedReadOnly.set !== "undefined") throw new Error("nested read-only view exposes set");
const readonlyCopy = nestedReadOnly.copy();
readonlyCopy[0] = -5;
if (readonly.at(1) !== 13 || base[2] !== 13) {
    throw new Error("read-only copy aliases the base");
}

// Iterator and search helpers operate on the live range.
const seen = [...outer];
if (JSON.stringify(seen) !== JSON.stringify([1, 13, 55, 4])) {
    throw new Error(`iterator saw ${JSON.stringify(seen)}`);
}
if (outer.indexOf(13) !== 1) throw new Error("indexOf missed");
if (outer.indexOf(999) !== -1) throw new Error("indexOf found an absent value");
if (!outer.includes(55) || outer.includes(999)) throw new Error("includes is wrong");
const nanSlice = slice([NaN, 1]);
if (nanSlice.indexOf(NaN) !== -1) throw new Error("indexOf should use strict equality");
if (!nanSlice.includes(NaN)) throw new Error("includes should use SameValueZero");

// The end of the target is a valid empty range; anything else is rejected.
if (slice(base, base.length, base.length).length !== 0) {
    throw new Error("an empty view at the end was rejected");
}
if (new Slice([], 0, 0).isEmpty !== true) throw new Error("empty default view is wrong");

const failures = [
    [() => slice(base, -1), RangeError],
    [() => slice(base, 0, -1), RangeError],
    [() => slice(base, 0.5), RangeError],
    [() => slice(base, 0, 1.5), RangeError],
    [() => slice(base, 3, 2), RangeError],
    [() => slice(base, 0, base.length + 1), RangeError],
    [() => slice(base, "0"), TypeError],
    [() => slice(base, 0, "1"), TypeError],
    [() => outer.at(-1), RangeError],
    [() => outer.at(1.5), RangeError],
    [() => outer.at(outer.length), RangeError],
    [() => outer.set(outer.length, 1), RangeError],
    [() => outer.set("0", 1), TypeError],
    [() => slice(null), TypeError],
    [() => slice({ length: 0 }), TypeError],
    [() => slice("abc"), TypeError],
];
for (const [fn, type] of failures) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof type)) {
        throw new Error(`${fn} threw ${error && error.constructor.name}, expected ${type.name}`);
    }
}
"#,
    );
}

#[test]
fn array_holes_accessors_frozen_and_splice_semantics() {
    run_js(
        r#"
import { slice } from "slices";

// Holes read as undefined and become own undefined elements in a copy.
const holes = [1, , 3];
const holeView = slice(holes);
if (holeView.at(1) !== undefined) throw new Error("a hole did not read undefined");
const holeCopy = holeView.copy();
if (holeCopy.length !== 3 || !(1 in holeCopy) || holeCopy[1] !== undefined) {
    throw new Error("a hole was not copied as an undefined element");
}

// Structural changes are not tracked, but element access sees the target.
const base = [0, 1, 2, 3, 4];
const view = slice(base, 1, 4);
base.splice(0, 1);
if (view.length !== 3) throw new Error("the view length moved after splice");
if (view.at(0) !== 2 || view.at(1) !== 3) throw new Error("splice aliasing is wrong");
base.length = 1;
if (view.at(2) !== undefined) throw new Error("a truncated read is not undefined");
view.set(2, 42);
if (base.length !== 4 || base[3] !== 42) throw new Error("a write did not extend the Array");
if (view.length !== 3 || view.at(2) !== 42) throw new Error("the fixed range changed");

// Accessor elements run their getter and setter.
const accessor = [];
const writes = [];
Object.defineProperty(accessor, 1, {
    configurable: true,
    enumerable: true,
    get() {
        return 40;
    },
    set(value) {
        writes[writes.length] = value;
    },
});
accessor.length = 3;
const accessorView = slice(accessor, 1, 3);
if (accessorView.at(0) !== 40) throw new Error("an accessor getter did not run");
accessorView.set(0, 7);
if (writes.length !== 1 || writes[0] !== 7) {
    throw new Error("an accessor setter was not called");
}

// A frozen Array rejects writes and still reads.
const frozen = Object.freeze([1, 2, 3]);
const frozenView = slice(frozen, 1, 3);
if (frozenView.at(0) !== 2) throw new Error("frozen read failed");
let writeError;
try {
    frozenView.set(0, 9);
} catch (error) {
    writeError = error;
}
if (!(writeError instanceof TypeError)) throw new Error("frozen write did not throw TypeError");
if (frozen[1] !== 2) throw new Error("frozen target was modified");

// A Proxy of an Array is an Array; its traps run and a copy is a plain Array.
let gets = 0;
const proxy = new Proxy([10, 20, 30], {
    get(target, key, receiver) {
        if (typeof key === "string" && /^\d+$/.test(key)) gets += 1;
        return Reflect.get(target, key, receiver);
    },
});
const proxyView = slice(proxy, 1, 3);
if (proxyView.at(0) !== 20 || proxyView.at(1) !== 30) throw new Error("proxy reads are wrong");
if (gets === 0) throw new Error("the Proxy get trap did not run");
const proxyCopy = proxyView.copy();
if (proxyCopy.constructor !== Array || proxyCopy.length !== 2) {
    throw new Error("proxy copy is not a plain Array");
}

// A species override must not run during view construction or copy.
class Evil extends Array {
    static get [Symbol.species]() {
        throw new Error("species was consulted");
    }
}
const evil = new Evil();
evil.push(1, 2, 3);
const evilView = slice(evil, 1, 3);
const evilCopy = evilView.copy();
if (evilCopy.constructor !== Array || evilCopy.length !== 2) {
    throw new Error("species changed the copy");
}
"#,
    );
}

#[test]
fn typed_array_views_preserve_type_offset_and_alias() {
    run_js(
        r#"
import { slice } from "slices";

// Two outer views of one buffer, and a nested view of one of them.
const buffer = new ArrayBuffer(8);
const first = new Uint8Array(buffer, 1, 4);
const second = new Uint8Array(buffer);
first.set([10, 11, 12, 13]);
const view = slice(first, 1, 3);
if (view.length !== 2 || view.at(0) !== 11 || view.at(1) !== 12) {
    throw new Error("offset view reads are wrong");
}
view.set(0, 200);
if (first[1] !== 200 || second[2] !== 200) throw new Error("buffer aliasing is wrong");
second[3] = 222;
if (view.at(1) !== 222) throw new Error("an outer write was not visible");
const nested = view.slice(1, 2);
if (nested.at(0) !== 222) throw new Error("nested typed-array offset is wrong");
nested.set(0, 33);
if (first[2] !== 33) throw new Error("nested typed-array write did not alias");

const copy = view.copy();
if (!(copy instanceof Uint8Array) || copy.length !== 2) throw new Error("typed copy type is wrong");
if (copy.buffer === buffer) throw new Error("typed copy shares the source buffer");
if (copy[0] !== 200 || copy[1] !== 33) throw new Error("typed copy values are wrong");
copy[0] = 1;
if (view.at(0) !== 200) throw new Error("typed copy aliases the source");

const types = [
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
for (const [source, name] of types) {
    const copy = slice(source).copy();
    if (copy.constructor.name !== name) {
        throw new Error(`copy of ${name} is ${copy.constructor.name}`);
    }
    if (copy.length !== source.length || copy[0] !== source[0]) {
        throw new Error(`copy of ${name} lost data`);
    }
}

const big = new BigInt64Array([1n, 2n, 3n]);
const bigView = slice(big, 1, 3);
if (bigView.at(0) !== 2n) throw new Error("BigInt read failed");
bigView.set(0, 20n);
if (big[1] !== 20n) throw new Error("BigInt write failed");
let bigTypeError;
try {
    bigView.set(0, 1);
} catch (error) {
    bigTypeError = error;
}
if (!(bigTypeError instanceof TypeError)) throw new Error("number was accepted for a BigInt view");

// A species override on a typed-array subclass must not run.
class EvilBytes extends Uint8Array {
    static get [Symbol.species]() {
        throw new Error("species was consulted");
    }
}
const evil = new EvilBytes([4, 5, 6]);
const evilCopy = slice(evil, 1, 3).copy();
if (Object.getPrototypeOf(evilCopy) !== Uint8Array.prototype || evilCopy.length !== 2) {
    throw new Error("typed-array species changed the copy");
}

const failures = [
    [() => slice(new DataView(new ArrayBuffer(4))), TypeError],
    [() => slice(new Proxy(new Uint8Array(3), {})), TypeError],
    [() => slice({ length: 3, 0: 1, [Symbol.toStringTag]: "Uint8Array" }), TypeError],
    [() => slice(new Uint8Array(3), 0, 4), RangeError],
];
for (const [fn, type] of failures) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof type)) {
        throw new Error(`${fn} threw ${error && error.constructor.name}, expected ${type.name}`);
    }
}
"#,
    );
}

#[test]
fn typed_array_detachment_resizable_and_shared_buffers() {
    run_js(
        r#"
import { slice } from "slices";

// Detaching after construction leaves the fixed range readable as undefined
// and writable as a no-op; copying refuses to invent zeros.
const detached = new Uint8Array([1, 2, 3]);
const detachedView = slice(detached);
if (typeof detached.buffer.transfer === "function") {
    detached.buffer.transfer();
} else if (typeof structuredClone === "function") {
    structuredClone(detached.buffer, { transfer: [detached.buffer] });
} else {
    throw new Error("this V8 has no way to detach an ArrayBuffer");
}
if (detached.length !== 0) throw new Error("detach did not empty the target");
if (detachedView.length !== 3) throw new Error("the fixed length changed after detach");
if (detachedView.at(0) !== undefined) throw new Error("a detached read is not undefined");
detachedView.set(0, 9);
if (detached.length !== 0) throw new Error("a detached write extended the view");
let copyError;
try {
    detachedView.copy();
} catch (error) {
    copyError = error;
}
if (!(copyError instanceof RangeError)) throw new Error("a detached copy did not throw RangeError");
if (slice(detached, 0, 0).length !== 0) throw new Error("an empty view of a detached target failed");

// A resizable ArrayBuffer shrinks out of view; growing it back restores the
// fixed range without changing the view's length.
const resizable = new ArrayBuffer(8, { maxByteLength: 16 });
const typed = new Uint8Array(resizable);
typed.set([1, 2, 3, 4, 5, 6, 7, 8]);
const resizableView = slice(typed);
if (resizableView.length !== 8) throw new Error("resizable view length is wrong");
resizable.resize(4);
if (resizableView.length !== 8) throw new Error("the fixed length changed after shrink");
if (resizableView.at(5) !== undefined) throw new Error("a shrunk read is not undefined");
resizableView.set(5, 9);
let shrinkCopyError;
try {
    resizableView.copy();
} catch (error) {
    shrinkCopyError = error;
}
if (!(shrinkCopyError instanceof RangeError)) {
    throw new Error("a shrunk copy did not throw RangeError");
}
resizable.resize(8);
if (resizableView.at(5) !== 0) throw new Error("regrown bytes are not readable");
if (resizableView.copy().length !== 8) throw new Error("a regrown copy failed");

// SharedArrayBuffer views share storage with every other view of the buffer.
if (typeof SharedArrayBuffer === "function") {
    const shared = new SharedArrayBuffer(4);
    const left = new Uint8Array(shared);
    const right = new Uint8Array(shared, 1, 2);
    const sharedView = slice(right, 1, 2);
    sharedView.set(0, 77);
    if (left[2] !== 77 || sharedView.at(0) !== 77) {
        throw new Error("a SharedArrayBuffer view did not alias");
    }
}
"#,
    );
}

#[test]
fn char_slices_roundtrip_and_transform_in_place() {
    run_js(
        r#"
import { CharSlice, ReadOnlyCharSlice, charSlice, readonlyCharSlice } from "slices";

const chars = CharSlice.fromString("Hello, 😀!");
if (chars.length !== 9) throw new Error(`code point length is ${chars.length}`);
if (chars.at(7) !== 0x1f600) throw new Error("astral code point is wrong");
if (chars.at(0) !== 72) throw new Error("first code point is wrong");
if (chars.toString() !== "Hello, 😀!") throw new Error("roundtrip failed");
if (chars.isEmpty) throw new Error("non-empty view is empty");

// A lone surrogate is preserved as one code unit.
const lone = CharSlice.fromString("a\uD800b");
if (lone.length !== 3 || lone.toString() !== "a\uD800b") {
    throw new Error("a lone surrogate did not roundtrip");
}

// Nested character views use relative offsets and alias the same buffer.
const text = CharSlice.fromString("abcdef");
const middle = text.slice(1, 5);
if (middle.length !== 4) throw new Error(`nested char length ${middle.length}`);
if (middle.toString() !== "bcde") throw new Error("nested char offset is wrong");
middle.toUpper();
if (middle.toString() !== "BCDE") throw new Error("in-place upper failed");
if (text.toString() !== "aBCDEf") throw new Error("in-place upper did not alias");
text.slice(0, 1).toLower();
if (text.toString() !== "aBCDEf") throw new Error("a lowercase no-op changed the text");

// The in-place transforms use simple case mapping and skip non-scalars.
const simple = CharSlice.fromString("ßé\uD800");
simple.toUpper();
if (simple.toString() !== "ßÉ\uD800") {
    throw new Error("simple case mapping or surrogate handling changed");
}
simple.toLower();
if (simple.toString() !== "ßé\uD800") {
    throw new Error("in-place lower changed an expanding mapping or surrogate");
}

// set() writes a code point through to the buffer and validates its argument.
text.set(0, 90);
if (text.toString() !== "ZBCDEf") throw new Error("set did not write through");
const badValues = [
    [() => text.set(0, "A"), TypeError],
    [() => text.set(0, 65.5), RangeError],
    [() => text.set(0, -1), RangeError],
    [() => text.set(0, 0x110000), RangeError],
    [() => text.set(-1, 65), RangeError],
    [() => text.set(text.length, 65), RangeError],
];
for (const [fn, type] of badValues) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof type)) {
        throw new Error(`${fn} threw ${error && error.constructor.name}, expected ${type.name}`);
    }
}

// A code point view can start at a byte offset inside a larger buffer.
const backing = new Uint32Array([1, 2, 3, 4]);
const offsetBuffer = new Uint32Array(backing.buffer, 4, 2);
const offsetView = charSlice(offsetBuffer);
if (offsetView.length !== 2 || offsetView.at(0) !== 2 || offsetView.at(1) !== 3) {
    throw new Error("offset character view is wrong");
}
offsetView.set(0, 50);
if (backing[1] !== 50) throw new Error("offset character write did not alias");

// A snapshot is independent in both directions.
const snapshot = text.toArray();
if (!(snapshot instanceof Uint32Array) || snapshot.length !== text.length) {
    throw new Error("toArray is not a Uint32Array snapshot");
}
snapshot[0] = 65;
text.set(1, 67);
if (snapshot[0] !== 65 || text.at(1) !== 67) throw new Error("toArray aliases the buffer");

// Read-only character views read live data and cannot transform.
const readonly = readonlyCharSlice(text.toArray(), 0, 3);
if (!(readonly instanceof ReadOnlyCharSlice)) {
    throw new Error("readonlyCharSlice returned the wrong class");
}
if (typeof readonly.set !== "undefined" || typeof readonly.toUpper !== "undefined") {
    throw new Error("read-only character view exposes a mutator");
}
if (readonly.toString() !== text.toString().slice(0, 3)) {
    throw new Error("read-only character text is wrong");
}

// A large buffer must not overflow fromCodePoint's argument list.
const large = CharSlice.fromString("a".repeat(200000) + "\u{1F600}");
if (large.length !== 200001) throw new Error("large buffer length is wrong");
if (large.toString() !== "a".repeat(200000) + "\u{1F600}") {
    throw new Error("large buffer roundtrip failed");
}

// Constructor buffers are caller-owned and unchecked; a conversion reports an
// invalid value instead of producing a malformed string.
const invalid = new Uint32Array([0x110000]);
let rangeError;
try {
    new CharSlice(invalid).toString();
} catch (error) {
    rangeError = error;
}
if (!(rangeError instanceof RangeError)) {
    throw new Error("an invalid code point did not throw RangeError");
}

const failures = [
    [() => charSlice(new Uint8Array(3)), TypeError],
    [() => charSlice([1, 2, 3]), TypeError],
    [() => charSlice("abc"), TypeError],
    [() => CharSlice.fromString(1), TypeError],
    [() => charSlice(new Uint32Array(3), 0, 4), RangeError],
];
for (const [fn, type] of failures) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof type)) {
        throw new Error(`${fn} threw ${error && error.constructor.name}, expected ${type.name}`);
    }
}
"#,
    );
}
