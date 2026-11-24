//! Deterministic test-support injection for the QuickJS UUID entropy draw.
//!
//! Compiled only when both `neo-uuid` and the off-by-default `test-support`
//! feature are enabled.
#![cfg(all(feature = "neo-uuid", feature = "test-support"))]

use std::sync::Mutex;

use neojs_lite_core::{Runtime, test_support};

static LOCK: Mutex<()> = Mutex::new(());

fn run_js(source: &str) {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source("memory.js", source)
        .expect("module assertions");
}

#[test]
fn v4_preserves_an_injected_entropy_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    test_support::fail_next_uuid_random();
    run_js(
        r#"
import { v4 } from "neo:uuid";

let code = "none";
let cause = "none";
let causeEnumerable = null;
try {
    v4();
} catch (error) {
    code = error.code;
    cause = error.cause;
    const descriptor = Object.getOwnPropertyDescriptor(error, "cause");
    causeEnumerable = descriptor === undefined ? null : descriptor.enumerable;
}
if (code !== "NEO_UUID_RANDOM") throw new Error("unexpected code: " + code);
if (cause !== "getrandom: unexpected error") throw new Error("unexpected cause: " + cause);
if (causeEnumerable !== false) throw new Error("cause must be non-enumerable");

const value = v4();
if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)) {
    throw new Error("v4 after injected failure: " + value);
}
"#,
    );
}

#[test]
fn v7_preserves_an_injected_entropy_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    test_support::fail_next_uuid_random();
    run_js(
        r#"
import { v7 } from "neo:uuid";

let code = "none";
let cause = "none";
let causeEnumerable = null;
try {
    v7();
} catch (error) {
    code = error.code;
    cause = error.cause;
    const descriptor = Object.getOwnPropertyDescriptor(error, "cause");
    causeEnumerable = descriptor === undefined ? null : descriptor.enumerable;
}
if (code !== "NEO_UUID_RANDOM") throw new Error("unexpected code: " + code);
if (cause !== "getrandom: unexpected error") throw new Error("unexpected cause: " + cause);
if (causeEnumerable !== false) throw new Error("cause must be non-enumerable");

const value = v7();
if (!/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)) {
    throw new Error("v7 after injected failure: " + value);
}
"#,
    );
}

#[test]
fn hostile_error_and_object_prototypes_do_not_mask_uuid_error() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    test_support::fail_next_uuid_random();
    run_js(
        r#"
import { v4 } from "neo:uuid";

let setterCalls = 0;
Object.defineProperty(Error.prototype, "message", {
    configurable: true,
    get() {
        return "";
    },
    set() {
        setterCalls += 1;
        throw new Error("hostile Error.prototype.message setter");
    },
});
Object.defineProperty(Object.prototype, "code", {
    configurable: true,
    set() {
        setterCalls += 1;
        throw new Error("hostile Object.prototype.code setter");
    },
});
Object.freeze(Error.prototype);

let caught;
try {
    v4();
} catch (error) {
    caught = error;
}
if (setterCalls !== 0) throw new Error(`prototype setter ran ${setterCalls} times`);
if (!(caught instanceof Error)) throw new Error(`expected Error, got ${caught}`);
if (caught.code !== "NEO_UUID_RANDOM") throw new Error(`unexpected code: ${caught.code}`);
if (caught.message !== "operating system random number generator failed") {
    throw new Error(`unexpected message: ${caught.message}`);
}
if (caught.cause !== "getrandom: unexpected error") {
    throw new Error(`unexpected cause: ${caught.cause}`);
}
"#,
    );
}
