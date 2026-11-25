use std::io::{Read as _, Write as _};
use std::time::Duration;

use neojs_node_core::child_process::{SpawnOptions, spawn_sync};

fn run_child_mode(mode: &str) -> ! {
    match mode {
        "exit_zero" => {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(b"ok-stdout");
            let _ = out.flush();
            std::process::exit(0);
        }
        "exit_42" => {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(b"boom-stderr");
            let _ = err.flush();
            std::process::exit(42);
        }
        "echo_stdin" => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).unwrap();
            let mut out = std::io::stdout().lock();
            out.write_all(&buf).unwrap();
            out.flush().unwrap();
            std::process::exit(0);
        }
        "concurrent_pipes" => {
            let t_out = std::thread::spawn(|| {
                let payload = vec![b'O'; 128 * 1024];
                let mut out = std::io::stdout().lock();
                out.write_all(&payload).unwrap();
                out.flush().unwrap();
            });
            let t_err = std::thread::spawn(|| {
                let payload = vec![b'E'; 128 * 1024];
                let mut err = std::io::stderr().lock();
                err.write_all(&payload).unwrap();
                err.flush().unwrap();
            });
            t_out.join().unwrap();
            t_err.join().unwrap();
            std::process::exit(0);
        }
        "sleep_long" => {
            std::thread::sleep(Duration::from_secs(10));
            std::process::exit(0);
        }
        "sleep_3s" => {
            std::thread::sleep(Duration::from_secs(3));
            std::process::exit(0);
        }
        "grandchild_pipe_inherit" => {
            let exe = std::env::current_exe().unwrap();
            let mut out = std::io::stdout().lock();
            out.write_all(b"parent-before-grandchild\n").unwrap();
            out.flush().unwrap();
            let mut _grandchild = std::process::Command::new(exe)
                .env("NEOJS_NODE_CORE_CHILD_MODE", "sleep_3s")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_secs(3));
            std::process::exit(0);
        }
        "noisy_truncate" => {
            let out_payload = vec![b'A'; 256 * 1024];
            let err_payload = vec![b'B'; 256 * 1024];
            std::io::stdout().write_all(&out_payload).unwrap();
            std::io::stderr().write_all(&err_payload).unwrap();
            std::process::exit(0);
        }
        "print_args_cwd_env" => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let cwd = std::env::current_dir().unwrap();
            let custom = std::env::var("NEOJS_TEST_CUSTOM").unwrap_or_default();
            let path_present = std::env::var_os("PATH").is_some();
            print!(
                "args={args:?};cwd={};custom={custom};path={path_present}",
                cwd.display()
            );
            std::process::exit(0);
        }
        other => {
            eprintln!("unknown child mode: {other}");
            std::process::exit(99);
        }
    }
}

fn spawn_self(
    mode: &str,
    args: &[String],
    mut opts: SpawnOptions,
) -> neojs_node_core::child_process::SpawnOutput {
    let exe = std::env::current_exe().unwrap();
    let exe_str = exe.to_str().unwrap();
    let mut env = opts.env.take().unwrap_or_default();
    env.push(("NEOJS_NODE_CORE_CHILD_MODE".to_owned(), mode.to_owned()));
    opts.env = Some(env);
    spawn_sync(exe_str, args, &opts).unwrap()
}

fn main() {
    if let Ok(mode) = std::env::var("NEOJS_NODE_CORE_CHILD_MODE") {
        run_child_mode(&mode);
    }

    // 1. Exit code 0 and non-zero
    let ok = spawn_self("exit_zero", &[], SpawnOptions::default());
    assert_eq!(ok.status, Some(0));
    assert_eq!(ok.signal, None);
    assert_eq!(ok.stdout, b"ok-stdout");
    assert!(ok.stderr.is_empty());
    assert!(!ok.timed_out);
    assert!(!ok.truncated);

    let fail = spawn_self("exit_42", &[], SpawnOptions::default());
    assert_eq!(fail.status, Some(42));
    assert_eq!(fail.stderr, b"boom-stderr");
    assert!(!fail.timed_out);

    // 2. Stdin pipe (> OS pipe buffer to verify concurrent write + read)
    let big_stdin = vec![b'x'; 128 * 1024];
    let echoed = spawn_self(
        "echo_stdin",
        &[],
        SpawnOptions {
            stdin: Some(big_stdin.clone()),
            ..SpawnOptions::default()
        },
    );
    assert_eq!(echoed.status, Some(0));
    assert_eq!(echoed.stdout, big_stdin);

    // 3. Concurrent large stdout + stderr capture without pipe deadlock
    let both = spawn_self("concurrent_pipes", &[], SpawnOptions::default());
    assert_eq!(both.status, Some(0));
    assert_eq!(both.stdout.len(), 128 * 1024);
    assert!(both.stdout.iter().all(|&b| b == b'O'));
    assert_eq!(both.stderr.len(), 128 * 1024);
    assert!(both.stderr.iter().all(|&b| b == b'E'));

    // 4. Timeout kills and reaps long-running child
    let start = std::time::Instant::now();
    let timed = spawn_self(
        "sleep_long",
        &[],
        SpawnOptions {
            timeout_ms: Some(100),
            ..SpawnOptions::default()
        },
    );
    assert!(timed.timed_out);
    assert!(start.elapsed() < Duration::from_secs(5));
    #[cfg(unix)]
    assert_eq!(timed.signal.as_deref(), Some("SIGKILL"));

    // 5. max_buffer cap truncates cleanly without deadlocking child
    let truncated = spawn_self(
        "noisy_truncate",
        &[],
        SpawnOptions {
            max_buffer: 1024,
            ..SpawnOptions::default()
        },
    );
    assert_eq!(truncated.status, Some(0));
    assert!(truncated.truncated);
    assert_eq!(truncated.stdout.len(), 1024);
    assert_eq!(truncated.stderr.len(), 1024);

    // 6. Shell-free literal args + cwd + clear_env
    let tmp = tempfile::tempdir().unwrap();
    let literal_arg = "; echo MUST_NOT_RUN && $(whoami)".to_owned();
    let env_test = spawn_self(
        "print_args_cwd_env",
        std::slice::from_ref(&literal_arg),
        SpawnOptions {
            cwd: Some(tmp.path().to_path_buf()),
            env: Some(vec![("NEOJS_TEST_CUSTOM".to_owned(), "neo".to_owned())]),
            clear_env: true,
            ..SpawnOptions::default()
        },
    );
    assert_eq!(env_test.status, Some(0));
    let out_str = String::from_utf8(env_test.stdout).unwrap();
    assert!(
        out_str.contains(&format!("args=[{literal_arg:?}]")),
        "{out_str}"
    );
    assert!(out_str.contains("custom=neo;path=false"), "{out_str}");

    // 7. Grandchild inheriting stdout/stderr pipes does not hang spawn_sync on timeout
    let gc_start = std::time::Instant::now();
    let gc = spawn_self(
        "grandchild_pipe_inherit",
        &[],
        SpawnOptions {
            timeout_ms: Some(150),
            ..SpawnOptions::default()
        },
    );
    assert!(gc.timed_out);
    assert!(
        gc_start.elapsed() < Duration::from_millis(1_500),
        "spawn_sync hung on grandchild pipe inheritance: {:?}",
        gc_start.elapsed()
    );
    assert_eq!(gc.stdout, b"parent-before-grandchild\n");

    println!("spawn_harness: all 7 child_process integration scenarios passed");
}
