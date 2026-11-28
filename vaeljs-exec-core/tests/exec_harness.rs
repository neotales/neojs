use std::io::{Read as _, Write as _};
use std::time::{Duration, Instant};

use vaeljs_exec_core::{CommandSpec, ExecErrorCode, Signal, StdioMode, exec_sync, run_sync, spawn};

fn run_child_mode(mode: &str) -> ! {
    match mode {
        "exit_zero" => {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(b"line1\nline2\r\n");
            let _ = out.flush();
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(b"warn1\n");
            let _ = err.flush();
            std::process::exit(0);
        }
        "exit_42" => {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(b"boom-stderr\n");
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
        "interactive_echo" => {
            use std::io::BufRead as _;
            let stdin = std::io::stdin();
            let mut out = std::io::stdout().lock();
            for line in stdin.lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                if writeln!(out, "echo:{line}").is_err() || out.flush().is_err() {
                    break;
                }
            }
            std::process::exit(0);
        }
        "concurrent_pipes" => {
            let t_out = std::thread::spawn(|| {
                let payload = vec![b'O'; 160 * 1024];
                let mut out = std::io::stdout().lock();
                out.write_all(&payload).unwrap();
                out.flush().unwrap();
            });
            let t_err = std::thread::spawn(|| {
                let payload = vec![b'E'; 160 * 1024];
                let mut err = std::io::stderr().lock();
                err.write_all(&payload).unwrap();
                err.flush().unwrap();
            });
            t_out.join().unwrap();
            t_err.join().unwrap();
            std::process::exit(0);
        }
        "noisy_output" => {
            let payload = vec![b'A'; 128 * 1024];
            let _ = std::io::stdout().write_all(&payload);
            let _ = std::io::stdout().flush();
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
            let _grandchild = std::process::Command::new(exe)
                .env("VAELJS_EXEC_CORE_CHILD_MODE", "sleep_3s")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_secs(3));
            std::process::exit(0);
        }
        "print_args_cwd_env" => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let cwd = std::env::current_dir().unwrap();
            let custom = std::env::var("VAELJS_TEST_CUSTOM").unwrap_or_default();
            let removed = std::env::var_os("VAELJS_TEST_REMOVED").is_some();
            let path_present = std::env::var_os("PATH").is_some();
            print!(
                "args={args:?};cwd={};custom={custom};removed={removed};path={path_present}",
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

fn spec_for_self(mode: &str) -> CommandSpec {
    let exe = std::env::current_exe().unwrap();
    CommandSpec::new(exe.to_str().unwrap())
        .unwrap()
        .env("VAELJS_EXEC_CORE_CHILD_MODE", mode)
        .unwrap()
}

fn main() {
    if let Ok(mode) = std::env::var("VAELJS_EXEC_CORE_CHILD_MODE") {
        run_child_mode(&mode);
    }

    // 1. Basic exit codes + CommandOutput helpers
    let ok = exec_sync(&spec_for_self("exit_zero")).unwrap();
    assert_eq!(ok.code, 0);
    assert_eq!(ok.signal, None);
    assert!(ok.success);
    assert!(!ok.timed_out);
    assert!(!ok.truncated);
    assert_eq!(ok.text(), "line1\nline2\r\n");
    assert_eq!(ok.lines(), vec!["line1", "line2"]);
    assert_eq!(ok.error_text(), "warn1\n");
    assert_eq!(ok.error_lines(), vec!["warn1"]);

    let fail = exec_sync(&spec_for_self("exit_42")).unwrap();
    assert_eq!(fail.code, 42);
    assert!(!fail.success);
    assert_eq!(fail.error_lines(), vec!["boom-stderr"]);

    // 2. run_sync returns CommandStatus
    let run_spec = spec_for_self("exit_zero")
        .stdout(StdioMode::Null)
        .unwrap()
        .stderr(StdioMode::Null)
        .unwrap();
    let st = run_sync(&run_spec).unwrap();
    assert_eq!(st.code, 0);
    assert!(st.success);
    assert!(!st.timed_out);

    // 3. Argv-only shell-injection immunity + env_clear + env + env_remove + cwd
    let tmp = tempfile::tempdir().unwrap();
    let literal_args = ["; echo INJECTED", "$(whoami)", "| cat"];
    let env_spec = CommandSpec::new(std::env::current_exe().unwrap().to_str().unwrap())
        .unwrap()
        .args(literal_args)
        .unwrap()
        .cwd(tmp.path())
        .unwrap()
        .env("VAELJS_TEST_REMOVED", "should_be_cleared")
        .unwrap()
        .env_clear()
        .env("VAELJS_EXEC_CORE_CHILD_MODE", "print_args_cwd_env")
        .unwrap()
        .env("VAELJS_TEST_CUSTOM", "neo")
        .unwrap()
        .env("VAELJS_TEST_TEMP", "gone")
        .unwrap()
        .env_remove("VAELJS_TEST_TEMP")
        .unwrap();
    let env_out = exec_sync(&env_spec).unwrap();
    assert!(env_out.success);
    let out_str = env_out.text();
    assert!(
        out_str.contains(&format!("args={:?}", literal_args)),
        "{out_str}"
    );
    let expected_cwd = std::fs::canonicalize(tmp.path()).unwrap();
    let expected_cwd_str = expected_cwd.to_str().unwrap();
    assert!(
        out_str.contains(&format!("cwd={expected_cwd_str}"))
            || out_str.contains(&format!("cwd={}", tmp.path().display())),
        "{out_str}"
    );
    assert!(
        out_str.contains("custom=neo;removed=false;path=false"),
        "{out_str}"
    );

    // 4. Stdin piping (>128 KiB) + concurrent large stdout+stderr (>128 KiB each)
    let big_stdin = vec![b'x'; 160 * 1024];
    let echoed = exec_sync(&spec_for_self("echo_stdin").stdin_bytes(big_stdin.clone())).unwrap();
    assert!(echoed.success);
    assert_eq!(echoed.stdout, big_stdin);

    let both = exec_sync(&spec_for_self("concurrent_pipes")).unwrap();
    assert!(both.success);
    assert_eq!(both.stdout.len(), 160 * 1024);
    assert!(both.stdout.iter().all(|&b| b == b'O'));
    assert_eq!(both.stderr.len(), 160 * 1024);
    assert!(both.stderr.iter().all(|&b| b == b'E'));

    // 5. max_output_bytes enforcement -> OutputTooLarge
    let too_large_spec = spec_for_self("noisy_output")
        .max_output_bytes(1024)
        .unwrap();
    let err = exec_sync(&too_large_spec).expect_err("oversized output must fail");
    assert_eq!(err.code(), ExecErrorCode::OutputTooLarge);

    // 6a. Timeout enforcement with SIGTERM graceful termination
    let t_start = Instant::now();
    let term_spec = spec_for_self("sleep_long")
        .timeout(Duration::from_millis(80))
        .unwrap()
        .kill_signal(Signal::SigTerm)
        .kill_grace(Duration::from_millis(500))
        .unwrap();
    let term_out = exec_sync(&term_spec).unwrap();
    assert!(term_out.timed_out);
    assert!(!term_out.success);
    assert!(
        t_start.elapsed() < Duration::from_millis(450),
        "SIGTERM should terminate promptly without waiting full grace: {:?}",
        t_start.elapsed()
    );
    #[cfg(unix)]
    assert_eq!(term_out.signal.as_deref(), Some("SIGTERM"));

    // 6b. Timeout enforcement with SIGTERM -> SIGKILL escalation (~150ms total)
    #[cfg(unix)]
    {
        let esc_start = Instant::now();
        let esc_spec = CommandSpec::new("/bin/sh")
            .unwrap()
            .args(["-c", "trap '' TERM; sleep 10"])
            .unwrap()
            .timeout(Duration::from_millis(80))
            .unwrap()
            .kill_signal(Signal::SigTerm)
            .kill_grace(Duration::from_millis(70))
            .unwrap();
        let esc_out = exec_sync(&esc_spec).unwrap();
        assert!(esc_out.timed_out);
        assert!(!esc_out.success);
        assert_eq!(esc_out.signal.as_deref(), Some("SIGKILL"));
        assert!(
            esc_start.elapsed() < Duration::from_secs(2),
            "escalation took too long: {:?}",
            esc_start.elapsed()
        );
    }

    // 7. Grandchild inheriting pipes does not hang exec_sync on timeout
    let gc_start = Instant::now();
    let gc_spec = spec_for_self("grandchild_pipe_inherit")
        .timeout(Duration::from_millis(120))
        .unwrap()
        .kill_grace(Duration::from_millis(50))
        .unwrap();
    let gc_out = exec_sync(&gc_spec).unwrap();
    assert!(gc_out.timed_out);
    assert!(
        gc_start.elapsed() < Duration::from_secs(2),
        "exec_sync hung on grandchild pipe inheritance: {:?}",
        gc_start.elapsed()
    );
    assert_eq!(gc_out.stdout, b"parent-before-grandchild\n");

    // 8. SpawnedChild interactive stdin write -> stdout read -> kill(SigTerm) -> wait()
    let mut child = spawn(&spec_for_self("interactive_echo")).unwrap();
    assert!(child.pid() > 0);
    assert!(child.try_wait().unwrap().is_none());
    child.write_stdin(b"hello-exec\n").unwrap();

    let mut received = Vec::new();
    while !received.ends_with(b"\n") {
        let chunk = child.read_stdout(1024).unwrap();
        assert!(!chunk.is_empty(), "unexpected EOF before response");
        received.extend_from_slice(&chunk);
    }
    assert_eq!(received, b"echo:hello-exec\n");

    child.kill(Signal::SigTerm).unwrap();
    let child_status = child.wait().unwrap();
    assert!(!child_status.success);
    #[cfg(unix)]
    assert_eq!(child_status.signal.as_deref(), Some("SIGTERM"));
    assert!(child.try_wait().unwrap().is_some());

    // 9. SpawnedChild wait_with_output + close_stdin / take_stdin
    let mut child2 = spawn(&spec_for_self("echo_stdin")).unwrap();
    child2.write_stdin(b"streamed-payload").unwrap();
    child2.close_stdin();
    assert!(child2.take_stdin().is_none());
    let out2 = child2
        .wait_with_output(1024 * 1024, Some(Duration::from_secs(5)))
        .unwrap();
    assert!(out2.success);
    assert_eq!(out2.stdout, b"streamed-payload");

    println!("exec_harness: all child process integration scenarios passed");
}
