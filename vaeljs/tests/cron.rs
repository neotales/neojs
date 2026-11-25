//! Deterministic scheduler tests for `vael:cron`.
//!
//! `test-support` installs a per-thread clock. The entry module moves it with
//! the test-only host op and yields to the event loop with a real 1 ms timer, so
//! every cron tick is decided by simulated time rather than by waiting a minute.
//! The 1 ms timers are only an event-loop turn mechanism; they do not affect
//! cron scheduling, which reads the fake clock. A normal build has neither the
//! clock nor the op.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use vaeljs_core::Runtime;
use vaeljs_core::test_support::CronTestClock;

/// A start time that is not minute-aligned, so the first occurrence is a real
/// boundary rather than the starting value.
const START_MS: f64 = 1_700_000_000_000.0;

/// The last instant a JavaScript `Date` represents.
const MAX_DATE_MS: f64 = 8.64e15;

fn write_module(dir: &tempfile::TempDir, name: &str, source: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, source).expect("write source");
    path
}

/// Runs a module with a deterministic clock and a bounded wall-clock deadline.
///
/// The entry future is polled on this thread, which is also where the clock and
/// the runtime live. An undisposed job must not be left behind, because the
/// deadline would then fail the test instead of hanging it.
fn run_with_clock(source: &str) {
    run_with_clock_at(source, START_MS);
}

/// Runs a module with the deterministic clock installed at `start_ms`.
fn run_with_clock_at(source: &str, start_ms: f64) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", source);
    let clock = CronTestClock::install(start_ms);
    let mut runtime = Runtime::new().expect("runtime");
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("caller runtime");
    caller.block_on(async {
        match tokio::time::timeout(Duration::from_secs(10), runtime.run_file_async(&file)).await {
            Ok(result) => result.expect("module assertions"),
            Err(_) => panic!("the cron scheduler did not settle within 10 seconds"),
        }
    });
    drop(clock);
}

/// The event-loop turn helper and assertions shared by every module.
const PRELUDE: &str = r#"
const advance = Deno.core.ops.op_vael_cron_test_advance;
function turn() {
    return new Promise((resolve) => Deno.core.createTimer(resolve, 1));
}
async function until(label, predicate) {
    for (let index = 0; index < 400; index += 1) {
        if (predicate()) return;
        await turn();
    }
    throw new Error(`timed out waiting for ${label}`);
}
function move(milliseconds) {
    if (advance(milliseconds) !== true) {
        throw new Error("the test clock is not installed");
    }
}
"#;

#[test]
fn runs_three_consecutive_ticks_then_disposes() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
let ticks = 0;
let finish;
const done = new Promise((resolve) => {{ finish = resolve; }});
const job = cron("* * * * *", () => {{
    ticks += 1;
    if (ticks === 3) {{
        job.dispose();
        finish();
    }}
}});
for (let expected = 1; expected <= 3; expected += 1) {{
    move(60_000);
    await until(`tick ${{expected}}`, () => ticks === expected);
}}
await done;
if (ticks !== 3) throw new Error(`expected 3 ticks, got ${{ticks}}`);
"#
    );
    run_with_clock(&source);
}

#[test]
fn dispatches_the_last_representable_occurrence_then_stops() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
let ticks = 0;
const job = cron("* * * * *", () => {{
    ticks += 1;
}});
move(60_000);
await until("final tick", () => ticks === 1);
// The job has no following occurrence, so it must release the event loop
// instead of waiting again. More turns must not produce a second tick.
await turn();
await turn();
if (ticks !== 1) throw new Error(`expected 1 tick, got ${{ticks}}`);
"#
    );
    run_with_clock_at(&source, MAX_DATE_MS - 60_000.0);
}

#[test]
fn reports_overlap_and_releases_at_the_last_occurrence() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
const max = {MAX_DATE_MS};
let ticks = 0;
let overlap = null;
let release;
let finish;
const done = new Promise((resolve) => {{ finish = resolve; }});
const job = cron(
    "* * * * *",
    () => {{
        ticks += 1;
        if (ticks === 1) {{
            return new Promise((resolve) => {{ release = resolve; }});
        }}
        throw new Error("the final occurrence ran while the previous callback was in flight");
    }},
    {{
        onEvent(event) {{
            if (event.type === "overlap") {{
                overlap = event;
                finish();
            }}
        }},
    }},
);
move(60_000);
await until("first tick", () => ticks === 1);
move(60_000);
await until("final overlap", () => overlap !== null);
await done;
if (ticks !== 1) throw new Error(`the final occurrence ran: ${{ticks}}`);
if (overlap.scheduledTime.getTime() !== max) {{
    throw new Error(`wrong overlap time: ${{overlap.scheduledTime.toISOString()}}`);
}}
if (overlap.wokeAt.getTime() !== max) {{
    throw new Error(`wrong wake time: ${{overlap.wokeAt.toISOString()}}`);
}}
release();
await turn();
// The job is released despite the overlap, so advancing the clock must not
// schedule anything further.
move(60_000);
await turn();
await turn();
if (ticks !== 1) throw new Error(`a released job ticked again: ${{ticks}}`);
"#
    );
    run_with_clock_at(&source, MAX_DATE_MS - 120_000.0);
}

#[test]
fn skips_a_tick_while_the_previous_callback_is_in_flight() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
let ticks = 0;
let release;
const events = [];
let finish;
const done = new Promise((resolve) => {{ finish = resolve; }});
const job = cron(
    "* * * * *",
    () => {{
        ticks += 1;
        if (ticks === 1) {{
            return new Promise((resolve) => {{ release = resolve; }});
        }}
        if (ticks === 2) {{
            job.dispose();
            finish();
        }}
    }},
    {{
        onEvent(event) {{
            events.push(event.type);
            if (event.type === "overlap") release();
        }},
    }},
);
move(60_000);
await until("first tick", () => ticks === 1);
move(60_000);
await until("overlap", () => events.includes("overlap"));
move(60_000);
await until("second tick", () => ticks === 2);
await done;
if (ticks !== 2) throw new Error(`overlap ran a callback twice: ${{ticks}}`);
if (events.filter((type) => type === "overlap").length !== 1) {{
    throw new Error(`unexpected events: ${{events}}`);
}}
"#
    );
    run_with_clock(&source);
}

#[test]
fn reports_missed_ticks_and_does_not_run_a_stale_occurrence() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
let ticks = 0;
let missed = 0;
let finish;
const done = new Promise((resolve) => {{ finish = resolve; }});
const job = cron(
    "* * * * *",
    () => {{
        ticks += 1;
    }},
    {{
        onEvent(event) {{
            if (event.type === "missed") {{
                missed += 1;
                job.dispose();
                finish();
            }}
        }},
    }},
);
// The first wait is for the next minute. Moving five minutes at once means the
// following occurrence is already due when the sleep resolves.
move(300_000);
await until("missed event", () => missed === 1);
await done;
if (ticks !== 0) throw new Error("the stale occurrence ran");
"#
    );
    run_with_clock(&source);
}

#[test]
fn reports_sync_and_async_callback_errors_and_keeps_running() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
let ticks = 0;
const messages = [];
const fatal = [];
let finish;
const done = new Promise((resolve) => {{ finish = resolve; }});
const job = cron(
    "* * * * *",
    () => {{
        ticks += 1;
        if (ticks === 1) return Promise.reject(new Error("async boom"));
        if (ticks === 2) throw new Error("sync boom");
        if (ticks === 3) {{
            job.dispose();
            finish();
        }}
    }},
    {{
        onEvent(event) {{
            if (event.type === "error") {{
                messages.push(`${{event.error.name}}: ${{event.error.message}}`);
                // An observer throw is discarded by the module, so this
                // value is asserted outside the callback instead.
                fatal.push(event.fatal);
            }}
        }},
    }},
);
move(60_000);
await until("async rejection", () => messages.includes("Error: async boom"));
move(60_000);
await until("sync throw", () => messages.includes("Error: sync boom"));
move(60_000);
await until("third tick", () => ticks === 3);
await done;
if (messages.length !== 2) throw new Error(`unexpected errors: ${{messages}}`);
if (fatal.length !== 2 || fatal.some((value) => value !== false)) {{
    throw new Error(`a callback error must not be fatal: ${{fatal}}`);
}}
"#
    );
    run_with_clock(&source);
}

#[test]
fn a_throwing_observer_does_not_stop_the_schedule() {
    let source = format!(
        r#"
import {{ cron }} from "vael:cron";
{PRELUDE}
let ticks = 0;
let observed = 0;
const observedTypes = [];
let finish;
const done = new Promise((resolve) => {{ finish = resolve; }});
const job = cron(
    "* * * * *",
    () => {{
        ticks += 1;
        // The first tick fails, which gives the observer a real event to
        // receive. The observer throws while handling it.
        if (ticks === 1) throw new Error("callback failure");
        if (ticks === 2) {{
            job.dispose();
            finish();
        }}
    }},
    {{
        onEvent(event) {{
            observed += 1;
            // Record before throwing. The throw is the behaviour under test
            // and is discarded by the module, so assertions are outside.
            observedTypes.push(event.type);
            throw new Error("observer failure");
        }},
    }},
);
move(60_000);
await until("first tick and observer", () => ticks === 1 && observed === 1);
move(60_000);
await until("second tick", () => ticks === 2);
await done;
if (observed !== 1) throw new Error(`the observer ran ${{observed}} times`);
if (observedTypes.length !== 1 || observedTypes[0] !== "error") {{
    throw new Error(`unexpected observer events: ${{observedTypes}}`);
}}
"#
    );
    run_with_clock(&source);
}
