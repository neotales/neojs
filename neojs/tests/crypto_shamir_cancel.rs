//! Deterministic cancellation tests for asynchronous Shamir secret sharing.
//!
//! A test-only gate holds a real blocking worker after it starts. One test drops
//! the runtime while a running worker is held, releases it, and proves the
//! worker's `WorkerResult` is dropped and wiped without ever reaching V8. A
//! second test saturates the four-thread pool with five jobs and proves only
//! the queued job's reservation is released on runtime teardown.
//!
//! The gate is compiled only with `neojs-core`'s off-by-default `test-support`
//! feature, which `neojs` enables for its own dev tests. All waits are bounded,
//! and a release guard releases the gate on panic or timeout.

use std::fs;
use std::future::Future;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use neojs_core::Runtime;
use neojs_core::test_support::CancellationGate;
use neojs_core::test_support::with_cancellation_gate;

const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

/// The test binary's counters and gate are process-global; keep the two tests
/// from overlapping.
static LOCK: Mutex<()> = Mutex::new(());

/// Releases the gate on any exit path, including panic or an assertion failure.
struct GateRelease(Option<std::sync::Arc<CancellationGate>>);

impl Drop for GateRelease {
    fn drop(&mut self) {
        if let Some(gate) = self.0.take() {
            gate.release();
        }
    }
}

/// Poll the entry future once, returning whether it completed.
fn poll_entry(
    future: &mut Pin<Box<impl Future<Output = Result<(), neojs_core::Error>>>>,
    context: &mut std::task::Context<'_>,
) -> bool {
    future.as_mut().poll(context).is_ready()
}

fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn dropping_the_runtime_while_a_shamir_worker_runs_wipes_its_result() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { shamirSplitAsync } from "neo:crypto";
await shamirSplitAsync(Uint8Array.of(1, 2, 3, 4), 2, 3);
"#,
    )
    .expect("write source");

    let results_baseline = neojs_core::test_support::zeroized_shamir_worker_results();
    let v8_baseline = neojs_core::test_support::zeroized_shamir_buffers();
    let gate = CancellationGate::new();
    let _release_on_exit = GateRelease(Some(gate.clone()));
    let mut runtime = with_cancellation_gate(gate.clone(), || Runtime::new().expect("runtime"));

    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("caller runtime");

    let (started, before_cancel) = caller.block_on(async {
        let mut future = Box::pin(runtime.run_file_async(&file));
        let wait = tokio::time::timeout(WAIT_TIMEOUT, async {
            loop {
                if gate.is_join_started() && gate.is_started() {
                    return true;
                }
                let completed =
                    poll_fn(|context| Poll::Ready(poll_entry(&mut future, context))).await;
                if completed {
                    return false;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        let before = if matches!(wait, Ok(true)) {
            gate.budget()
        } else {
            (0, 0)
        };
        drop(future);
        drop(runtime);
        (wait, before)
    });

    match started {
        Ok(true) => {}
        Ok(false) => panic!("entry completed before the join op and worker started"),
        Err(error) => panic!("timed out waiting for the join and worker: {error}"),
    }
    assert_eq!(
        before_cancel.0, 1,
        "the held worker owns one job before cancellation"
    );

    // The worker is still held after the runtime is gone. Releasing it lets the
    // detached task produce a WorkerResult with no JoinHandle owner; the result
    // must be dropped and its Zeroizing data cleared.
    gate.release();
    assert!(
        gate.wait_finished_timeout(WAIT_TIMEOUT),
        "the held Shamir worker did not finish"
    );

    assert!(
        wait_until(|| {
            neojs_core::test_support::zeroized_shamir_worker_results() > results_baseline
        }),
        "the abandoned running worker result was not cleared"
    );
    assert_eq!(
        neojs_core::test_support::zeroized_shamir_buffers(),
        v8_baseline,
        "an abandoned worker result must not be transferred to V8"
    );
    assert_eq!(gate.budget(), (0, 0));

    drop(caller);
}

#[test]
fn dropping_the_runtime_cancels_only_the_queued_shamir_job() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    const JOBS: usize = 5;

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { shamirSplitAsync } from "neo:crypto";

const jobs = [];
for (let index = 0; index < 5; index += 1) {
    jobs.push(shamirSplitAsync(Uint8Array.of(1, 2, 3, 4), 2, 3));
}
await Promise.all(jobs);
"#,
    )
    .expect("write source");

    let gate = CancellationGate::new();
    let _release_on_exit = GateRelease(Some(gate.clone()));
    let mut runtime = with_cancellation_gate(gate.clone(), || Runtime::new().expect("runtime"));

    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("caller runtime");

    let (saturated, before_cancel) = caller.block_on(async {
        let mut future = Box::pin(runtime.run_file_async(&file));
        let wait = tokio::time::timeout(WAIT_TIMEOUT, async {
            loop {
                if gate.started_count() >= 4 && gate.budget().0 == JOBS {
                    return true;
                }
                let completed =
                    poll_fn(|context| Poll::Ready(poll_entry(&mut future, context))).await;
                if completed {
                    return false;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        let before = if matches!(wait, Ok(true)) {
            gate.budget()
        } else {
            (0, 0)
        };
        drop(future);
        drop(runtime);
        (wait, before)
    });

    match saturated {
        Ok(true) => {}
        Ok(false) => panic!("entry completed before the gate was saturated"),
        Err(error) => panic!("timed out waiting for the gate: {error}"),
    }
    assert_eq!(
        before_cancel.0, JOBS,
        "all five Shamir jobs are admitted before teardown"
    );

    assert!(
        wait_until(|| gate.budget().0 == 4),
        "the queued Shamir job's reservation was not released: {:?}",
        gate.budget()
    );
    assert_eq!(
        gate.budget().0,
        4,
        "only the queued Shamir job is released; four running jobs stay charged"
    );

    gate.release();
    assert!(
        gate.wait_finished_count_timeout(4, WAIT_TIMEOUT),
        "the four running Shamir workers did not finish"
    );
    assert_eq!(gate.budget(), (0, 0));

    drop(caller);
}
