//! Deterministic cancellation test for asynchronous `crypto` hashing.
//!
//! A test-only gate holds a real blocking worker after it starts. The tests
//! drive a Tokio current-thread caller so deno_core's real async-op driver
//! polls the join op. One test drops the runtime while a running worker is
//! held. Two saturate the four-thread pool with five join ops (or four joined
//! plus one unjoined raw resource) and drop the runtime, covering both the
//! join-op abort guard and the resource `Drop` for a queued job while four
//! running jobs stay charged. The queued job's reservation must disappear while
//! the running jobs keep theirs until the gate is released. Dropping only the
//! entry future would leave the runtime's op futures alive and is not
//! cancellation.
//!
//! All waits are bounded. A release guard releases the gate on panic, timeout,
//! or any other early exit so a failed test cannot leave a worker blocked.
//!
//! The gate is compiled only with `vaeljs-core`'s off-by-default `test-support`
//! feature, which `vaeljs` enables for its own dev tests. A normal build has no
//! gate, barrier, or probe.

use std::fs;
use std::future::Future;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use vaeljs_core::Runtime;
use vaeljs_core::test_support::CancellationGate;
use vaeljs_core::test_support::with_cancellation_gate;

const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

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
    future: &mut Pin<Box<impl Future<Output = Result<(), vaeljs_core::Error>>>>,
    context: &mut std::task::Context<'_>,
) -> bool {
    future.as_mut().poll(context).is_ready()
}

#[test]
fn cancellation_keeps_the_reservation_until_the_worker_finishes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { hashAsync } from "vael:crypto";
await hashAsync("sha256", new Uint8Array(1024 * 1024), "hex");
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

    let (both_started, before_cancel) = caller.block_on(async {
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
                if gate.is_join_started() && gate.is_started() {
                    return true;
                }
                // Let the caller runtime run deno_core's async-op driver.
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

    match both_started {
        Ok(true) => {}
        Ok(false) => panic!("entry completed before both the join op and the worker started"),
        Err(error) => panic!("timed out waiting for join and worker: {error}"),
    }

    assert_eq!(
        before_cancel,
        (1, 1024 * 1024),
        "the held worker owns one job and its input bytes before cancellation"
    );

    // Both the real join op and the real worker were held. Dropping the entry
    // future and runtime cancels the join without touching the worker's
    // reservation.
    assert_eq!(
        gate.budget(),
        (1, 1024 * 1024),
        "cancelling the join future must not release the worker's reservation"
    );

    gate.release();
    assert!(
        gate.wait_finished_timeout(WAIT_TIMEOUT),
        "the worker did not finish within the bounded wait"
    );

    assert_eq!(
        gate.budget(),
        (0, 0),
        "the worker releases the job and byte reservations when it finishes"
    );

    drop(caller);
}

/// A fifth hash job is submitted through the raw host op and left unjoined, so
/// it stays in the resource table behind the four gated workers. Dropping the
/// runtime must abort that queued resource and release its reservation; the
/// four running jobs stay charged until the gate is released.
#[test]
fn dropping_an_unjoined_hash_resource_aborts_only_the_queued_job() {
    const JOBS: usize = 5;
    const PER_JOB: usize = 1024;

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { hashAsync } from "vael:crypto";

const running = [];
for (let index = 0; index < 4; index += 1) {
    running.push(hashAsync("sha256", new Uint8Array(1024), "hex"));
}
const raw = Deno.core.ops.op_vael_crypto_async_submit(
    "sha256",
    0,
    undefined,
    new Uint8Array(1024),
    1,
);
if (!raw.ok || typeof raw.id !== "number") throw new Error("raw submit failed");

// Keep the entry future pending; raw.id is deliberately never joined.
await new Promise(() => {});
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

    let outcome = caller.block_on(async {
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

    let (wait, before) = outcome;
    match wait {
        Ok(true) => {}
        Ok(false) => panic!("entry completed before the gate was saturated"),
        Err(error) => panic!("timed out waiting for the gate: {error}"),
    }
    assert_eq!(
        before,
        (JOBS, JOBS * PER_JOB),
        "five hash jobs are admitted before runtime teardown"
    );

    let deadline = Instant::now() + WAIT_TIMEOUT;
    let after_drop = loop {
        let current = gate.budget();
        if current.0 == 4 {
            break current;
        }
        if Instant::now() >= deadline {
            panic!("unjoined hash resource drop did not release its reservation: {current:?}");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(
        after_drop,
        (4, 4 * PER_JOB),
        "only the unjoined queued hash resource is released on runtime teardown"
    );

    gate.release();
    assert!(
        gate.wait_finished_count_timeout(4, WAIT_TIMEOUT),
        "the four running hash workers did not finish within the bounded wait"
    );
    assert_eq!(gate.budget(), (0, 0));
    drop(caller);
}

/// Five hash join ops are started and the runtime is dropped. The four gated
/// workers cannot be stopped and stay charged; the fifth queued job's join-op
/// abort guard must release its reservation.
#[test]
fn dropping_runtime_pending_hash_join_futures_aborts_only_the_queued_job() {
    const JOBS: usize = 5;
    const PER_JOB: usize = 1024;

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { hashAsync } from "vael:crypto";

const jobs = [];
for (let index = 0; index < 5; index += 1) {
    jobs.push(hashAsync("sha256", new Uint8Array(1024), "hex"));
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

    let outcome = caller.block_on(async {
        let mut future = Box::pin(runtime.run_file_async(&file));
        let wait = tokio::time::timeout(WAIT_TIMEOUT, async {
            loop {
                if gate.join_started_count() >= JOBS
                    && gate.started_count() >= 4
                    && gate.budget().0 == JOBS
                {
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

    let (wait, before) = outcome;
    match wait {
        Ok(true) => {}
        Ok(false) => panic!("entry completed before the gate was saturated"),
        Err(error) => panic!("timed out waiting for the gate: {error}"),
    }
    assert_eq!(
        before,
        (JOBS, JOBS * PER_JOB),
        "five hash jobs are admitted before runtime teardown"
    );

    let deadline = Instant::now() + WAIT_TIMEOUT;
    let after_drop = loop {
        let current = gate.budget();
        if current.0 == 4 {
            break current;
        }
        if Instant::now() >= deadline {
            panic!("queued hash join cancellation did not release its reservation: {current:?}");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(
        after_drop,
        (4, 4 * PER_JOB),
        "only the queued hash join's reservation is released on runtime teardown"
    );

    gate.release();
    assert!(
        gate.wait_finished_count_timeout(4, WAIT_TIMEOUT),
        "the four running hash workers did not finish within the bounded wait"
    );
    assert_eq!(gate.budget(), (0, 0));
    drop(caller);
}
