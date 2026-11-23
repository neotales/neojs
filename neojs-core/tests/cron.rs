//! Integration tests for the `cron` built-in, driven through `Runtime`.
//!
//! The scheduler's deterministic clock lives in `neojs/tests/cron.rs`, where the
//! `test-support` feature is enabled. This file covers the module identity and
//! the parser/scheduler surface that does not need a fake clock.

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
fn cron_module_identity_and_exports() {
    run_js(
        r#"
import * as cron from "neo:cron";
import * as upper from "NEO:cron";

const registry = Neo.getBuiltInModule("neo:cron");
if (registry !== Neo.getBuiltInModule("neo:cron")) {
    throw new Error("cron lookup is not cached");
}
if (!Object.isFrozen(registry)) throw new Error("cron is not frozen");
if (Neo.getBuiltInModule("cron") !== undefined) {
    throw new Error("the registry accepted a bare name");
}
if (upper !== cron) throw new Error("the scheme is not case-insensitive");
if (import.meta.resolve("neo:cron") !== "neo:cron") {
    throw new Error("cron did not resolve canonically");
}
if (import.meta.resolve("NEO:cron") !== "neo:cron") {
    throw new Error("the scheme is not case-insensitive");
}
let bareRejected = false;
try {
    await import("cron");
} catch (error) {
    bareRejected = error instanceof TypeError;
}
if (!bareRejected) throw new Error("bare cron import was accepted");

const exported = Object.keys(cron).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`export list ${exported} does not match ${registered}`);
}
const expected = ["CronError", "cron", "next"];
if (JSON.stringify(registered) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exports: ${registered}`);
}
for (const name of expected) {
    if (registry[name] !== cron[name]) {
        throw new Error(`${name} is not identity-equal`);
    }
}
"#,
    );
}

#[test]
fn cron_next_returns_utc_occurrences() {
    run_js(
        r#"
import { next } from "neo:cron";

const iso = (value) => {
    if (!(value instanceof Date)) throw new Error("next did not return a Date");
    return value.toISOString();
};

if (iso(next("* * * * *", new Date("2024-01-01T00:00:00.000Z"))) !==
    "2024-01-01T00:01:00.000Z") {
    throw new Error("every-minute next");
}
if (iso(next("*/15 * * * *", new Date("2024-01-01T00:00:00.000Z"))) !==
    "2024-01-01T00:15:00.000Z") {
    throw new Error("step next");
}
// Strictly after: an occurrence itself is not returned again.
if (iso(next("0 0 * * *", new Date("2024-01-01T00:00:00.000Z"))) !==
    "2024-01-02T00:00:00.000Z") {
    throw new Error("strict after");
}
// A number is accepted as the starting point.
if (iso(next("0 0 29 2 *", Date.UTC(2023, 2, 1))) !==
    "2024-02-29T00:00:00.000Z") {
    throw new Error("leap-day next");
}
// Both day fields restricted: the 13th OR any Tuesday.
if (iso(next("0 0 13 * TUE", new Date("2024-05-13T01:00:00.000Z"))) !==
    "2024-05-14T00:00:00.000Z") {
    throw new Error("day-of-month/day-of-week OR rule");
}
// 0 and 7 are both Sunday.
if (iso(next("0 0 * * 7", new Date("2024-01-05T00:00:00.000Z"))) !==
    "2024-01-07T00:00:00.000Z") {
    throw new Error("Sunday alias");
}
// Month and weekday names, short and full.
if (iso(next("30 9 * * MON-FRI", new Date("2024-01-05T09:30:00.000Z"))) !==
    "2024-01-08T09:30:00.000Z") {
    throw new Error("named weekdays");
}
if (iso(next("0 0 1 January Monday", new Date("2024-01-02T00:00:00.000Z"))) !==
    "2024-01-08T00:00:00.000Z") {
    throw new Error("full names and OR rule");
}
// A valid expression with no occurrence in the search window is null.
if (next("0 0 31 2 *", 0) !== null) {
    throw new Error("February 31st should have no occurrence");
}
// The result is always a representable Date. The largest Date instant has no
// strict successor, and the smallest still has a next minute.
const MAX_MS = 8.64e15;
if (next("* * * * *", MAX_MS) !== null) {
    throw new Error("the last Date instant should have no next occurrence");
}
if (next("* * * * *", MAX_MS - 1).getTime() !== MAX_MS) {
    throw new Error("the last Date instant should be reachable");
}
const first = next("* * * * *", -MAX_MS);
if (!(first instanceof Date) || first.getTime() !== -MAX_MS + 60_000) {
    throw new Error("the first representable minute is wrong");
}
"#,
    );
}

#[test]
fn cron_errors_carry_stable_codes() {
    run_js(
        r#"
import { CronError, cron, next } from "neo:cron";

const expectCode = (code, callback) => {
    try {
        callback();
    } catch (error) {
        if (!(error instanceof CronError)) {
            throw new Error(`expected CronError, got ${error}`);
        }
        if (error.name !== "CronError") throw new Error(`bad name: ${error.name}`);
        if (error.code !== code) throw new Error(`expected ${code}, got ${error.code}`);
        if (typeof error.message !== "string" || error.message.length === 0) {
            throw new Error("empty error message");
        }
        return;
    }
    throw new Error(`expected ${code}`);
};

expectCode("NEO_CRON_EXPRESSION", () => next("* * * *"));
expectCode("NEO_CRON_EXPRESSION", () => next("60 * * * *"));
expectCode("NEO_CRON_EXPRESSION", () => next("5-1 * * * *"));
expectCode("NEO_CRON_EXPRESSION", () => next("0 0 L * *"));
expectCode("NEO_CRON_EXPRESSION", () => next("0 0 ? * *"));
expectCode("NEO_CRON_LIMIT", () => next("1".repeat(513)));
expectCode("NEO_CRON_NO_OCCURRENCE", () => cron("0 0 31 2 *", () => {}));

const typeError = (callback) => {
    try {
        callback();
    } catch (error) {
        if (!(error instanceof TypeError)) {
            throw new Error(`expected TypeError, got ${error}`);
        }
        return;
    }
    throw new Error("expected TypeError");
};
typeError(() => next(1));
typeError(() => next("* * * * *", "2024-01-01"));
typeError(() => next("* * * * *", {}));
typeError(() => cron("* * * * *"));
typeError(() => cron("* * * * *", () => {}, null));
typeError(() => cron("* * * * *", () => {}, { onEvent: 1 }));
typeError(() => cron("* * * * *", () => {}, { timeout: 1 }));

const rangeError = (callback) => {
    try {
        callback();
    } catch (error) {
        if (!(error instanceof RangeError)) {
            throw new Error(`expected RangeError, got ${error}`);
        }
        return;
    }
    throw new Error("expected RangeError");
};
rangeError(() => next("* * * * *", new Date(NaN)));
rangeError(() => next("* * * * *", NaN));
rangeError(() => next("* * * * *", Infinity));
"#,
    );
}

#[test]
fn cron_controller_is_frozen_and_dispose_is_idempotent() {
    run_js(
        r#"
import { cron } from "neo:cron";

const job = cron("0 0 1 1 *", () => {});
if (!Object.isFrozen(job)) throw new Error("controller is not frozen");
if (typeof job.dispose !== "function") throw new Error("dispose is missing");
if (Object.prototype.propertyIsEnumerable.call(job, "dispose") !== true) {
    throw new Error("dispose should be a plain own property");
}
if (typeof Symbol.dispose === "symbol") {
    if (job[Symbol.dispose] !== job.dispose) {
        throw new Error("Symbol.dispose does not call dispose");
    }
}
job.dispose();
job.dispose();
if (typeof Symbol.dispose === "symbol") job[Symbol.dispose]();
"#,
    );
}

#[test]
fn cron_job_keeps_the_loop_alive_until_dispose() {
    // The module returns only after disposing, so the runtime must not wait for
    // the far-future occurrence. A leaked wait would hang the test.
    run_js(
        r#"
import { cron } from "neo:cron";

const job = cron("0 0 1 1 *", () => {
    throw new Error("the callback must not run");
});
job.dispose();
console.log("disposed");
"#,
    );
}
