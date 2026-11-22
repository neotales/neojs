//! Deterministic cancellation test for asynchronous PBKDF2.
//!
//! A test-only gate holds a real blocking worker after it starts. The tests
//! drive a Tokio current-thread caller so deno_core's real async-op driver
//! polls the join op. One starts five join ops and drops the runtime, so the
//! join-op futures are dropped and their abort guards run. Another leaves the
//! fifth job in the resource table unjoined and drops the runtime, so the
//! resource's `Drop` aborts it. Both saturate the four-thread pool so the fifth
//! job is queued. The queued job must be aborted and release its reservation,
//! while the four running jobs stay charged until the gate is released. This
//! proves password jobs use the same shared budget as hashing and distinguishes
//! queued from running cancellation. Dropping only the public entry future
//! would not test cancellation: the runtime still owns the op futures.
//!
//! All waits are bounded. A release guard releases the gate on panic, timeout,
//! or any other early exit so a failed test cannot leave a worker blocked.

use std::fs;
use std::future::Future;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use neojs_core::Runtime;
use neojs_core::test_support::CancellationGate;
use neojs_core::test_support::with_cancellation_gate;

const WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const PASSWORD_BYTES: usize = 512 * 1024;
const SALT_BYTES: usize = 4;

struct GateRelease(Option<std::sync::Arc<CancellationGate>>);

impl Drop for GateRelease {
    fn drop(&mut self) {
        if let Some(gate) = self.0.take() {
            gate.release();
        }
    }
}

fn poll_entry(
    future: &mut Pin<Box<impl Future<Output = Result<(), neojs_core::Error>>>>,
    context: &mut std::task::Context<'_>,
) -> bool {
    future.as_mut().poll(context).is_ready()
}

#[test]
fn cancelled_password_job_keeps_its_shared_reservation_until_the_worker_finishes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        format!(
            r#"
import {{ pbkdf2Async }} from "neo:crypto";
await pbkdf2Async("sha256", new Uint8Array({PASSWORD_BYTES}), "salt", 1, 16);
"#
        ),
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
        (1, PASSWORD_BYTES + SALT_BYTES),
        "the held worker owns one job and both input buffers before cancellation"
    );

    assert_eq!(
        gate.budget(),
        (1, PASSWORD_BYTES + SALT_BYTES),
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

/// The blocking pool has four threads. Four gated jobs occupy it and a fifth
/// stays queued with five join ops pending in the runtime. Dropping the runtime
/// drops those join-op futures, and the queued join's abort guard must release
/// its reservation while the four running jobs remain charged until the gate is
/// released.
#[test]
fn dropping_runtime_pending_password_join_futures_aborts_only_the_queued_job() {
    const JOBS: usize = 5;
    const PER_JOB: usize = 1024 + SALT_BYTES;

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { pbkdf2Async } from "neo:crypto";

const jobs = [];
for (let index = 0; index < 5; index += 1) {
    jobs.push(pbkdf2Async("sha256", new Uint8Array(1024), "salt", 1, 16));
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
                // Four blocking threads have entered the gate and the fifth
                // job is still submitted but queued.
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
        // The public entry future owns polling; the join-op futures live in
        // the runtime. Dropping the runtime drops them and runs their abort
        // guards. Dropping only the entry future would leave the runtime's op
        // futures alive.
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
        "five jobs are admitted before cancellation"
    );

    // The queued job must be aborted and its reservation released. The four
    // running jobs cannot be stopped and must remain charged.
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let after_cancel = loop {
        let current = gate.budget();
        if current.0 == 4 {
            break current;
        }
        if Instant::now() >= deadline {
            panic!("queued cancellation did not release its reservation: {current:?}");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(
        after_cancel,
        (4, 4 * PER_JOB),
        "only the queued job's reservation is released on cancellation"
    );

    gate.release();
    assert!(
        gate.wait_finished_count_timeout(4, WAIT_TIMEOUT),
        "the four running workers did not finish within the bounded wait"
    );
    assert_eq!(
        gate.budget(),
        (0, 0),
        "running workers release their reservations when they finish"
    );

    drop(caller);
}

/// The fifth job is submitted through the raw host op and left unjoined, so it
/// remains in the resource table. Dropping the runtime must abort that queued
/// resource and release its reservation, while the four running joined jobs
/// remain charged until the gate is released.
#[test]
fn dropping_an_unjoined_password_resource_aborts_only_the_queued_job() {
    const JOBS: usize = 5;
    const PER_JOB: usize = 1024 + SALT_BYTES;

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { pbkdf2Async } from "neo:crypto";

const running = [];
for (let index = 0; index < 4; index += 1) {
    running.push(pbkdf2Async("sha256", new Uint8Array(1024), "salt", 1, 16));
}
const raw = Deno.core.ops.op_neo_crypto_pbkdf2_async_submit(
    "sha256",
    new Uint8Array(1024),
    "salt",
    1,
    16,
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
        "five jobs are admitted before runtime teardown"
    );

    let deadline = Instant::now() + WAIT_TIMEOUT;
    let after_drop = loop {
        let current = gate.budget();
        if current.0 == 4 {
            break current;
        }
        if Instant::now() >= deadline {
            panic!("unjoined-resource drop did not release its reservation: {current:?}");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(
        after_drop,
        (4, 4 * PER_JOB),
        "only the unjoined queued resource is released on runtime teardown"
    );

    gate.release();
    assert!(
        gate.wait_finished_count_timeout(4, WAIT_TIMEOUT),
        "the four running workers did not finish within the bounded wait"
    );
    assert_eq!(gate.budget(), (0, 0));
    drop(caller);
}
