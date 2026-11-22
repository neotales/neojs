//! Probes how the experimental Web Crypto surface behaves under
//! `run_file_async`, with and without an ambient Tokio runtime.
//!
//! `deno_crypto` runs algorithm work through
//! `deno_core::unsync::spawn_blocking`, which is `tokio::task::spawn_blocking`.
//! With no Tokio runtime on the polling thread, that call panics inside a V8
//! callback, which cannot unwind: the process aborts. Run without an argument
//! to observe that; run with `tokio` to run the same module under a
//! current-thread runtime.
//!
//! Build and run with `--features webcrypto-proto`; `required-features` gates
//! the example.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use neojs_core::Runtime;

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

/// Polls a future with a waker that never schedules the task.
fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    future.poll(&mut context)
}

fn write_probe_module() -> std::path::PathBuf {
    let path = std::env::temp_dir().join("neojs-webcrypto-async-probe.js");
    std::fs::write(
        &path,
        r#"
const digest = await crypto.subtle.digest("SHA-256", new Uint8Array(1024 * 1024));
if (digest.byteLength !== 32) throw new Error("digest length");
"#,
    )
    .expect("write probe module");
    path
}

fn run_without_tokio(path: &std::path::Path) {
    let mut runtime = Runtime::new().expect("runtime");
    let mut future = Box::pin(runtime.run_file_async(path));
    for _ in 0..1000 {
        match poll_once(future.as_mut()) {
            Poll::Ready(Ok(())) => {
                println!("no-tokio: completed (unexpected)");
                return;
            }
            Poll::Ready(Err(error)) => {
                println!("no-tokio: module error: {error}");
                return;
            }
            Poll::Pending => {}
        }
    }
    println!("no-tokio: still pending after 1000 polls");
}

fn run_with_tokio(path: &std::path::Path) {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let mut runtime = Runtime::new().expect("runtime");
    match executor.block_on(runtime.run_file_async(path)) {
        Ok(()) => println!("tokio: completed"),
        Err(error) => println!("tokio: module error: {error}"),
    }
}

fn main() {
    let path = write_probe_module();
    match std::env::args().nth(1).as_deref() {
        Some("tokio") => run_with_tokio(&path),
        _ => run_without_tokio(&path),
    }
}
