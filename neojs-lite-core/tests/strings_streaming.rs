//! Bounded engine access for the folded searches on QuickJS.
//!
//! A small `fromIndex` must not copy a million-code-unit QuickJS string. These
//! tests drive the public `neo:strings` bindings and read the test-only
//! counters that the native adapter increments when it copies a text window.
#![cfg(all(feature = "neo-strings", feature = "test-support"))]

use neojs_lite_core::{Runtime, test_support};

/// Mirrors `neojs_builtins::strings::WINDOW_UNITS`.
const WINDOW_UNITS: usize = 1024;
const TEXT_UNITS: usize = 1_000_000;

/// Serializes tests because the read counters are process-global.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_search(limit: usize) -> (usize, usize) {
    let source = format!(
        r#"
import {{ lastIndexOfFold }} from "neo:strings";
const text = "a".repeat({TEXT_UNITS});
const result = lastIndexOfFold(text, "q", {limit});
if (result !== -1) throw new Error(`unexpected result: ${{result}}`);
"#
    );
    let mut runtime = Runtime::new().expect("runtime");
    test_support::reset_strings_text_reads();
    runtime
        .run_source("memory.js", &source)
        .expect("module assertions");
    test_support::strings_text_reads()
}

#[test]
fn a_small_from_index_reads_only_a_bounded_window() {
    let _guard = TEST_LOCK.lock().expect("test lock");
    let (units, windows) = run_search(0);
    assert!(
        units <= 64,
        "limit 0 copied {units} units, expected at most 64"
    );
    assert!(
        windows <= 1,
        "limit 0 read {windows} windows, expected at most 1"
    );
}

#[test]
fn the_read_work_grows_with_the_limit_not_the_text() {
    let _guard = TEST_LOCK.lock().expect("test lock");
    let (small_units, _) = run_search(0);
    let (large_units, large_windows) = run_search(10_000);

    assert!(
        large_units > small_units,
        "the 10000 limit copied {large_units} units, not more than the zero limit's {small_units}"
    );
    assert!(
        large_units <= 10_000 + 64,
        "the 10000 limit copied {large_units} units, expected at most {}",
        10_000 + 64
    );
    assert!(
        large_windows <= 1 + 10_000 / WINDOW_UNITS + 1,
        "the 10000 limit read {large_windows} windows"
    );
    assert!(
        large_units < TEXT_UNITS,
        "the limit copied the whole {TEXT_UNITS}-unit text"
    );
}

#[test]
fn an_adversarial_append_rope_copies_only_the_bounded_prefix() {
    let _guard = TEST_LOCK.lock().expect("test lock");
    let source = r#"
import { lastIndexOfFold } from "neo:strings";
let text = "";
for (let index = 0; index < 50000; index += 1) text += "a";
const result = lastIndexOfFold(text, "q", 10000);
if (result !== -1) throw new Error(`unexpected result: ${result}`);
"#;
    let mut runtime = Runtime::new().expect("runtime");
    test_support::reset_strings_text_reads();
    runtime
        .run_source("memory.js", source)
        .expect("rope search");
    let (units, windows) = test_support::strings_text_reads();
    assert!(
        units <= 10_000 + 64,
        "rope search copied {units} units, expected at most {}",
        10_000 + 64
    );
    assert!(windows <= 11, "rope search read {windows} windows");
}

#[test]
fn a_read_failure_stops_immediately_and_reaches_the_guest() {
    let _guard = TEST_LOCK.lock().expect("test lock");
    let source = r#"
import { lastIndexOfFold } from "neo:strings";
const text = "a".repeat(1000000);
let caught;
try {
    lastIndexOfFold(text, "q", 10000);
} catch (error) {
    caught = error;
}
if (!(caught instanceof Error)) throw new Error("the injected error was not guest-visible");
if (caught.message !== "injected strings read failure") {
    throw new Error(`the original exception was masked: ${caught.message}`);
}
"#;
    let mut runtime = Runtime::new().expect("runtime");
    test_support::reset_strings_text_reads();
    test_support::fail_strings_after_loads(1);
    runtime
        .run_source("memory.js", source)
        .expect("the guest catches the injected exception");
    let (units, windows) = test_support::strings_text_reads();
    assert!(windows == 1, "the failed scan read {windows} windows");
    assert!(
        units <= WINDOW_UNITS,
        "the failed scan copied {units} units"
    );
}
