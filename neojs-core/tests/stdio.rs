//! Tests for the `stdio` built-in module, driven through `Runtime`.
//!
//! The JavaScript modules assert on their own and throw on failure, so a test
//! is one module plus the Rust call that runs it. Binary content is checked
//! byte for byte; detached, forged, proxied, and oversized views are checked
//! for their documented failures.

use std::fs;

use neojs_core::Runtime;

/// Runs a JavaScript module whose own assertions throw on failure.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

/// Runs a JavaScript module that must fail, returning the error text.
fn run_js_error(source: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("module must fail");
    error.to_string()
}

/// JavaScript helpers shared by the assertions of one module.
///
/// They use only the language, not the module under test.
const HELPERS: &str = r#"
import { Readable, Writable, stdout, stderr, MAX_BYTES } from "neo:stdio";

function expectCode(code, action) {
    let error;
    try {
        action();
    } catch (thrown) {
        error = thrown;
    }
    if (!error) throw new Error(`expected ${code}, nothing was thrown`);
    if (error.name !== "StdioError") throw new Error(`expected StdioError, got ${error.name}`);
    if (error.code !== code) {
        throw new Error(`expected ${code}, got ${error.code}: ${error.message}`);
    }
}

function expectTypeError(action) {
    let error;
    try {
        action();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof TypeError)) {
        throw new Error(`expected TypeError, got ${error && error.name}: ${error && error.message}`);
    }
}

function bytesEqual(actual, expected) {
    if (actual.length !== expected.length) {
        throw new Error(`length ${actual.length} is not ${expected.length}`);
    }
    for (let index = 0; index < expected.length; index += 1) {
        if (actual[index] !== expected[index]) {
            throw new Error(`byte ${index} is ${actual[index]}, expected ${expected[index]}`);
        }
    }
}

function detach(view) {
    const buffer = view.buffer;
    buffer.transfer();
    if (view.byteLength !== 0) throw new Error("the view did not detach");
}
"#;

/// Prepends the shared JavaScript helpers. The placeholder is replaced rather
/// than formatted, because the JavaScript sources contain braces.
fn with_helpers(source: &str) -> String {
    source.replace("{HELPERS}", HELPERS)
}

#[test]
fn readable_returns_binary_content_and_reaches_eof() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const source = new Uint8Array(256);
for (let value = 0; value < 256; value += 1) source[value] = value;

const reader = new Readable(source);
if (reader.closed) throw new Error("a new reader is closed");
if (reader.remaining !== 256) throw new Error("remaining");

const target = new Uint8Array(100);
const first = reader.readSync(target);
if (first !== 100) throw new Error(`first read was ${first}`);
bytesEqual(target, source.subarray(0, 100));
if (reader.remaining !== 156) throw new Error("remaining after the first read");

const second = reader.readSync(target);
if (second !== 100) throw new Error(`second read was ${second}`);
bytesEqual(target, source.subarray(100, 200));
if (reader.remaining !== 56) throw new Error("remaining after the second read");

const short = new Uint8Array(200);
const third = reader.readSync(short);
if (third !== 56) throw new Error(`third read was ${third}`);
bytesEqual(short.subarray(0, 56), source.subarray(200));
if (reader.remaining !== 0) throw new Error("remaining at end of file");

for (let attempt = 0; attempt < 2; attempt += 1) {
    const result = reader.readSync(target);
    if (result !== 0) throw new Error("end of file must return 0");
}
reader.close();
"#,
    ));
}

#[test]
fn readable_copies_its_input_and_reads_only_the_view_range() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const backing = Uint8Array.from([9, 1, 2, 3, 9]);
const view = backing.subarray(1, 4);
const reader = new Readable(view);
backing.fill(7);
view.fill(8);

const target = new Uint8Array(3);
if (reader.readSync(target) !== 3) throw new Error("read count");
bytesEqual(target, Uint8Array.from([1, 2, 3]));

const detached = new Uint8Array(4);
detach(detached);
expectCode("NEO_STDIO_DETACHED", () => new Readable(detached));

expectTypeError(() => new Readable(backing.buffer));
expectTypeError(() => new Readable(new DataView(backing.buffer)));
expectTypeError(() => new Readable(new Proxy(new Uint8Array(2), {})));
expectTypeError(() => new Readable({}));
expectTypeError(() => new Readable(null));
expectTypeError(() => new Readable("text"));
expectTypeError(() => new Readable());

const offset = new Readable(new Uint8Array(4).subarray(2));
if (offset.remaining !== 2) throw new Error("an offset view must copy only its range");
"#,
    ));
}

#[test]
fn readable_rejects_empty_targets_and_closed_reads() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const reader = new Readable(Uint8Array.from([1, 2]));
const empty = new Uint8Array(0);
expectCode("NEO_STDIO_ARGUMENT", () => reader.readSync(empty));

const target = new Uint8Array(2);
reader.readSync(target);
reader.readSync(target);
if (reader.remaining !== 0) throw new Error("not at end of file");
expectCode("NEO_STDIO_ARGUMENT", () => reader.readSync(empty));

expectTypeError(() => reader.readSync());
expectTypeError(() => reader.readSync(target, 1));
expectTypeError(() => reader.readSync("text"));
expectTypeError(() => reader.readSync(new DataView(new ArrayBuffer(1))));

reader.close();
if (!reader.closed) throw new Error("closed");
if (reader.remaining !== 0) throw new Error("a closed reader keeps no bytes");
expectCode("NEO_STDIO_CLOSED", () => reader.readSync(target));
expectCode("NEO_STDIO_CLOSED", () => reader.readSync("not a view"));
expectTypeError(() => reader.close(1));
reader.close();

const detached = new Uint8Array(2);
detach(detached);
const live = new Readable(Uint8Array.from([1]));
expectCode("NEO_STDIO_DETACHED", () => live.readSync(detached));
"#,
    ));
}

#[test]
fn writable_accumulates_bytes_and_text_in_order() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const writer = new Writable();
if (writer.kind !== "memory") throw new Error("kind");
if (writer.length !== 0) throw new Error("a new writer is not empty");
if (writer.closed || writer.ended) throw new Error("a new writer is retired");

const binary = Uint8Array.from([0, 255, 128]);
if (writer.writeSync(binary) !== 3) throw new Error("binary count");
binary.fill(0);

const text = "Aé🐍";
if (writer.writeSync(text) !== 7) throw new Error("text must count UTF-8 bytes");
if (writer.writeSync("") !== 0) throw new Error("an empty string is a zero-byte write");

const backing = Uint8Array.from([4, 5, 6, 7]);
const offset = backing.subarray(1, 3);
if (writer.writeSync(offset) !== 2) throw new Error("offset count");
offset.fill(0);

if (writer.length !== 12) throw new Error(`length ${writer.length}`);
const expected = Uint8Array.from([
    0, 255, 128,
    65, 195, 169, 240, 159, 144, 141,
    5, 6,
]);
const first = writer.toUint8Array();
bytesEqual(first, expected);
first.fill(0);
bytesEqual(writer.toUint8Array(), expected);

if (writer.toUint8Array() === writer.toUint8Array()) {
    throw new Error("toUint8Array must return a fresh copy");
}
if (writer.writeSync("\uD800") !== 3) throw new Error("a lone surrogate is three bytes");
bytesEqual(writer.toUint8Array().subarray(12), Uint8Array.from([0xef, 0xbf, 0xbd]));
"#,
    ));
}

#[test]
fn writable_end_and_close_are_terminal_and_idempotent() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const writer = new Writable();
writer.writeSync("content");
writer.end();
writer.end();
if (!writer.ended) throw new Error("ended");
if (writer.closed) throw new Error("end is not close");
if (writer.length !== 7) throw new Error("end must keep memory-backed content");
if (writer.toUint8Array().length !== 7) throw new Error("readable after end");
expectCode("NEO_STDIO_ENDED", () => writer.writeSync("more"));

writer.close();
writer.close();
if (!writer.closed || !writer.ended) throw new Error("close must be terminal");
if (writer.length !== 0) throw new Error("close must release content");
expectCode("NEO_STDIO_CLOSED", () => writer.writeSync("more"));
expectCode("NEO_STDIO_CLOSED", () => writer.toUint8Array());
expectCode("NEO_STDIO_CLOSED", () => writer.writeSync(1));
expectTypeError(() => writer.end(1));
expectTypeError(() => writer.close(1));
expectTypeError(() => writer.toUint8Array(1));

const fresh = new Writable();
fresh.writeSync(Uint8Array.from([1]));
fresh.close();
if (fresh.length !== 0) throw new Error("closed writer length");
"#,
    ));
}

#[test]
fn writable_rejects_detached_and_invalid_chunks() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const writer = new Writable();
writer.writeSync(Uint8Array.from([1]));

const detached = new Uint8Array(4);
detach(detached);
expectCode("NEO_STDIO_DETACHED", () => writer.writeSync(detached));
if (writer.length !== 1) throw new Error("a failed write changed the writer");
bytesEqual(writer.toUint8Array(), Uint8Array.from([1]));

expectTypeError(() => writer.writeSync());
expectTypeError(() => writer.writeSync(Uint8Array.from([1]), 1));
expectTypeError(() => writer.writeSync(new Proxy(new Uint8Array(1), {})));
expectTypeError(() => writer.writeSync(new DataView(new ArrayBuffer(1))));
expectTypeError(() => writer.writeSync({}));
expectTypeError(() => writer.writeSync(null));
expectTypeError(() => writer.writeSync(1));
expectTypeError(() => writer.writeSync(true));

expectTypeError(() => new Writable(1));
const empty = new Writable();
if (empty.writeSync(new Uint8Array(0)) !== 0) throw new Error("empty chunk");
"#,
    ));
}

#[test]
fn memory_streams_are_bounded_by_max_bytes() {
    run_js(&with_helpers(
        r#"
{HELPERS}
if (MAX_BYTES !== 8388608) throw new Error(`MAX_BYTES is ${MAX_BYTES}`);

const full = new Uint8Array(MAX_BYTES);
const writer = new Writable();
if (writer.writeSync(full) !== MAX_BYTES) throw new Error("the full bound must be accepted");
if (writer.length !== MAX_BYTES) throw new Error("length at the bound");
expectCode("NEO_STDIO_LIMIT", () => writer.writeSync(Uint8Array.from([1])));
if (writer.length !== MAX_BYTES) throw new Error("a refused write changed the length");
if (writer.toUint8Array().length !== MAX_BYTES) throw new Error("content at the bound");

const oversized = new Uint8Array(MAX_BYTES + 1);
expectCode("NEO_STDIO_LIMIT", () => new Writable().writeSync(oversized));
expectCode("NEO_STDIO_LIMIT", () => new Readable(oversized));

const reader = new Readable(full);
if (reader.remaining !== MAX_BYTES) throw new Error("a reader at the bound");
"#,
    ));
}

#[test]
fn a_failed_write_does_not_change_the_writer() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const writer = new Writable();
writer.writeSync(Uint8Array.from([1, 2]));
const oversized = new Uint8Array(MAX_BYTES + 1);
expectCode("NEO_STDIO_LIMIT", () => writer.writeSync(oversized));
bytesEqual(writer.toUint8Array(), Uint8Array.from([1, 2]));
if (writer.length !== 2) throw new Error("length after a refused write");

const nearlyFull = new Writable();
nearlyFull.writeSync(new Uint8Array(MAX_BYTES - 1));
expectCode("NEO_STDIO_LIMIT", () => nearlyFull.writeSync(Uint8Array.from([1, 2])));
if (nearlyFull.length !== MAX_BYTES - 1) throw new Error("partial append on refusal");
"#,
    ));
}

/// `readSync` must not use `subarray`: its `Symbol.species` lookup would run
/// user JavaScript with the private snapshot buffer and allow reentrancy. The
/// same rule applies to `toUint8Array`.
#[test]
fn reads_and_copies_never_consult_species_or_prototype_methods() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const input = Uint8Array.from([1, 2, 3, 4]);
const chunk = Uint8Array.from([5, 6, 7]);
const expectedFirst = Uint8Array.from([1, 2]);
const expectedSecond = Uint8Array.from([3, 4]);
const reader = new Readable(input);

const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const speciesDescriptor = Object.getOwnPropertyDescriptor(Uint8Array, Symbol.species);
const subarrayDescriptor = Object.getOwnPropertyDescriptor(typedArrayPrototype, "subarray");
if (!subarrayDescriptor) throw new Error("the test needs the subarray descriptor");

let speciesCalls = 0;
let leaked = null;
let reentered = false;

Object.defineProperty(Uint8Array, Symbol.species, {
    configurable: true,
    get() {
        speciesCalls += 1;
        return class extends Uint8Array {
            constructor(buffer, offset, length) {
                leaked = buffer;
                reentered = true;
                if (buffer !== null) new Uint8Array(buffer).fill(9);
                reader.close();
                return super(buffer, offset, length);
            }
        };
    },
});
Object.defineProperty(typedArrayPrototype, "subarray", {
    configurable: true,
    writable: true,
    value() {
        reentered = true;
        throw new Error("subarray must not be used");
    },
});

try {
    const target = new Uint8Array(2);
    if (reader.readSync(target) !== 2) throw new Error("read count");
    bytesEqual(target, expectedFirst);
    if (speciesCalls !== 0) throw new Error("Symbol.species was consulted");
    if (leaked !== null) throw new Error("the private snapshot buffer was exposed");
    if (reentered) throw new Error("user code ran during readSync");
    if (reader.closed) throw new Error("reentrancy changed the reader");

    const next = new Uint8Array(2);
    if (reader.readSync(next) !== 2) throw new Error("second read count");
    bytesEqual(next, expectedSecond);

    const second = new Readable(input);
    if (second.remaining !== 4) throw new Error("the constructor consulted species");

    const writer = new Writable();
    writer.writeSync(chunk);
    bytesEqual(writer.toUint8Array(), chunk);
    if (speciesCalls !== 0) throw new Error("species was consulted by the writer");
    if (leaked !== null || reentered) throw new Error("writer ran user code");
} finally {
    if (speciesDescriptor) {
        Object.defineProperty(Uint8Array, Symbol.species, speciesDescriptor);
    } else {
        delete Uint8Array[Symbol.species];
    }
    Object.defineProperty(typedArrayPrototype, "subarray", subarrayDescriptor);
}
"#,
    ));
}

/// Exercises observable behavior under many empty and one-byte writes: counts,
/// length, byte-for-byte output, growth to `MAX_BYTES`, the capacity limit, and
/// an empty write at the bound. This is a behavior regression only. It does not
/// inspect private fields, measure allocated capacity, or prove the absence of
/// separate bookkeeping; the single coalesced buffer is established by source
/// review, not by this test.
#[test]
fn writable_handles_many_empty_and_small_writes() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const writer = new Writable();
const empty = new Uint8Array(0);
const one = new Uint8Array(1);

for (let index = 0; index < 10000; index += 1) {
    if (writer.writeSync(empty) !== 0) throw new Error("empty write count");
}
if (writer.length !== 0) throw new Error("empty writes retained bytes");
if (writer.toUint8Array().length !== 0) throw new Error("empty output");

const bytes = new Uint8Array(10000);
for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = index & 0xff;
}
for (let index = 0; index < bytes.length; index += 1) {
    one[0] = bytes[index];
    if (writer.writeSync(one) !== 1) throw new Error("small write count");
}
if (writer.length !== bytes.length) throw new Error("small write length");
bytesEqual(writer.toUint8Array(), bytes);

for (let index = 0; index < 10000; index += 1) {
    if (writer.writeSync(empty) !== 0) throw new Error("interleaved empty write count");
}
if (writer.length !== bytes.length) throw new Error("empty writes changed the length");
bytesEqual(writer.toUint8Array(), bytes);

writer.writeSync(new Uint8Array(MAX_BYTES - bytes.length));
if (writer.length !== MAX_BYTES) throw new Error("length at the bound");
expectCode("NEO_STDIO_LIMIT", () => writer.writeSync(one));
if (writer.writeSync(empty) !== 0) throw new Error("empty write at the bound");
if (writer.length !== MAX_BYTES) throw new Error("the refused write changed the length");

const full = writer.toUint8Array();
if (full.length !== MAX_BYTES) throw new Error("output length at the bound");
for (let index = 0; index < bytes.length; index += 1) {
    if (full[index] !== bytes[index]) throw new Error(`prefix byte ${index}`);
}
if (full[MAX_BYTES - 1] !== 0) throw new Error("tail byte");

writer.close();
if (writer.length !== 0 || !writer.closed) throw new Error("close after the bound");
"#,
    ));
}

#[test]
fn host_writers_are_shared_and_do_not_close_the_process_stream() {
    run_js(&with_helpers(
        r#"
{HELPERS}
if (stdout.kind !== "stdout") throw new Error("stdout kind");
if (stderr.kind !== "stderr") throw new Error("stderr kind");
if (stdout === stderr) throw new Error("the host writers are the same object");
if (stdout.closed || stdout.ended) throw new Error("a new host writer is retired");
if (stdout.length !== 0) throw new Error("a host writer retains nothing");
expectCode("NEO_STDIO_NOT_MEMORY", () => stdout.toUint8Array());

if (stdout.writeSync("") !== 0) throw new Error("an empty host write");
if (stdout.writeSync("stdio-host\n") !== 11) throw new Error("host write count");
if (stderr.writeSync(Uint8Array.from([0x65, 0x0a])) !== 2) throw new Error("stderr count");

stdout.end();
if (!stdout.ended || stdout.closed) throw new Error("host end");
expectCode("NEO_STDIO_ENDED", () => stdout.writeSync("more"));

stdout.close();
if (!stdout.closed) throw new Error("host close");
expectCode("NEO_STDIO_CLOSED", () => stdout.writeSync("more"));
stdout.close();
stdout.end();

if (stderr.writeSync("e\n") !== 2) throw new Error("stderr survived stdout close");

// The process descriptor is still open, so the console keeps working after the
// wrapper is retired. This writes through a different path.
console.log("stdio-console-after-close");
"#,
    ));
}

#[test]
fn registry_and_import_share_classes_constants_and_streams() {
    run_js(
        r#"
import * as namespace from "neo:stdio";
import { Readable as StaticReadable, stdout as staticStdout } from "neo:stdio";

const registry = Neo.getBuiltInModule("neo:stdio");
const dynamic = await import("neo:stdio");

if (!Object.isFrozen(registry)) throw new Error("the registry module is not frozen");
if (registry.Readable !== StaticReadable) throw new Error("Readable identity");
if (registry.Readable !== namespace.Readable) throw new Error("namespace Readable identity");
if (registry.Readable !== dynamic.Readable) throw new Error("dynamic Readable identity");
if (registry.Writable !== namespace.Writable) throw new Error("Writable identity");
if (registry.MAX_BYTES !== namespace.MAX_BYTES) throw new Error("MAX_BYTES identity");
if (registry.MAX_BYTES !== 8388608) throw new Error("MAX_BYTES value");
if (staticStdout !== registry.stdout) throw new Error("stdout identity");
if (namespace.stdout !== registry.stdout) throw new Error("namespace stdout identity");
if (dynamic.stdout !== registry.stdout) throw new Error("dynamic stdout identity");
if (namespace.stderr !== registry.stderr) throw new Error("stderr identity");
if (namespace.stdout === namespace.stderr) throw new Error("stdout and stderr are one object");

const exported = Object.keys(namespace).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`exports ${exported} do not match ${registered}`);
}
if ("default" in namespace) throw new Error("stdio must not have a default export");

if (import.meta.resolve("neo:stdio") !== "neo:stdio") throw new Error("stdio resolves canonically");
if (import.meta.resolve("NEO:stdio") !== "neo:stdio") throw new Error("scheme is not case-insensitive");
if (Neo.getBuiltInModule("stdio") !== undefined) throw new Error("registry accepted a bare name");
let bareRejected = false;
try {
    await import("stdio");
} catch (error) {
    bareRejected = error instanceof TypeError;
}
if (!bareRejected) throw new Error("bare stdio import was accepted");

const reader = new registry.Readable(Uint8Array.from([1]));
if (reader.remaining !== 1) throw new Error("a class from the registry did not construct");
"#,
    );
}

#[test]
fn built_in_namespace_and_registry_are_immutable() {
    run_js(
        r#"
import * as namespace from "neo:stdio";
const registry = Neo.getBuiltInModule("neo:stdio");

let rejections = 0;
try {
    namespace.Readable = () => "namespace";
} catch (error) {
    if (!(error instanceof TypeError)) throw error;
    rejections += 1;
}
try {
    registry.Readable = () => "registry";
} catch (error) {
    if (!(error instanceof TypeError)) throw error;
    rejections += 1;
}
if (rejections !== 2) throw new Error("a module property was replaceable");
if (typeof registry.Readable !== "function") throw new Error("the class changed");
"#,
    );
}

#[test]
fn wrong_argument_counts_throw_type_error() {
    run_js(&with_helpers(
        r#"
{HELPERS}
expectTypeError(() => new Readable());
expectTypeError(() => new Readable(new Uint8Array(1), 1));
expectTypeError(() => new Writable(1));

const reader = new Readable(Uint8Array.from([1]));
const target = new Uint8Array(1);
expectTypeError(() => reader.readSync());
expectTypeError(() => reader.readSync(target, 1));

const writer = new Writable();
expectTypeError(() => writer.writeSync());
expectTypeError(() => writer.writeSync("x", 1));
expectTypeError(() => writer.end(1));
expectTypeError(() => writer.close(1));
expectTypeError(() => writer.toUint8Array(1));
"#,
    ));
}

#[test]
fn writable_to_readable_roundtrip_preserves_binary_content() {
    run_js(&with_helpers(
        r#"
{HELPERS}
const writer = new Writable();
const chunks = [
    Uint8Array.from([0, 1, 2]),
    Uint8Array.from([255, 254, 253, 252]),
    Uint8Array.from([127, 128]),
];
for (const chunk of chunks) writer.writeSync(chunk);
writer.end();

const reader = new Readable(writer.toUint8Array());
const collected = [];
const target = new Uint8Array(3);
let count;
while ((count = reader.readSync(target)) !== 0) {
    collected.push(Uint8Array.from(target.subarray(0, count)));
}
const expected = Uint8Array.from([0, 1, 2, 255, 254, 253, 252, 127, 128]);
const actual = new Uint8Array(expected.length);
let position = 0;
for (const piece of collected) {
    actual.set(piece, position);
    position += piece.length;
}
bytesEqual(actual, expected);
if (reader.remaining !== 0) throw new Error("remaining after the roundtrip");
"#,
    ));
}

#[test]
fn importing_an_unknown_name_from_stdio_fails_to_link() {
    let message = run_js_error(
        r#"
import { stdin } from "neo:stdio";
console.log(stdin);
"#,
    );
    assert!(
        message.contains("stdin"),
        "the link error must name the missing export: {message}"
    );
}
