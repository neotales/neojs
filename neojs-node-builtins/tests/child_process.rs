//! Real synchronous process tests for the engine-neutral core.
//!
//! These use only absolute system programs that exist on the Linux test
//! machines. A missing program fails the test instead of skipping it, so a green
//! run always proves the runner executed something.

#![cfg(unix)]

use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use neojs_node_builtins::ChildProcessPolicy;
use neojs_node_builtins::Limits;
use neojs_node_builtins::NodeOptions;
use neojs_node_builtins::handle_request;
use serde_json::Value;
use serde_json::json;

fn require(path: &str) {
    assert!(Path::new(path).exists(), "test requires {path}");
}

fn policy(programs: &[&str]) -> ChildProcessPolicy {
    let mut policy = ChildProcessPolicy::new();
    for program in programs {
        policy = policy.allow_program(program).expect("allow program");
    }
    policy
}

fn request(
    policy: &ChildProcessPolicy,
    operation: &str,
    file: &str,
    args: &[&str],
    options: Value,
) -> Value {
    let payload = json!({
        "operation": operation,
        "file": file,
        "args": args,
        "options": options,
    });
    let response = handle_request(&payload.to_string(), &NodeOptions::new(policy.clone()));
    serde_json::from_str(&response).expect("valid response JSON")
}

fn ok_result(response: &Value) -> &Value {
    assert_eq!(response["ok"], true, "{response}");
    &response["result"]
}

#[test]
fn spawn_sync_really_runs_a_program_and_returns_stdout() {
    require("/bin/echo");
    let policy = policy(&["/bin/echo"]);
    let response = request(&policy, "spawnSync", "/bin/echo", &["hello"], json!({}));
    let result = ok_result(&response);
    assert_eq!(result["status"], 0);
    assert_eq!(result["stdout"], "hello\n");
    assert_eq!(result["stderr"], "");
    assert_eq!(result["timedOut"], false);
    assert_eq!(result["limitExceeded"], false);
    assert!(result["pid"].as_u64().expect("pid") > 0);
}

#[test]
fn standard_input_is_written_to_the_child() {
    require("/bin/cat");
    let policy = policy(&["/bin/cat"]);
    let response = request(
        &policy,
        "execFileSync",
        "/bin/cat",
        &[],
        json!({ "input": "abc" }),
    );
    let result = ok_result(&response);
    assert_eq!(result["status"], 0);
    assert_eq!(result["stdout"], "abc");
}

#[test]
fn a_working_directory_is_applied() {
    require("/bin/pwd");
    let directory = tempfile::tempdir().expect("temp dir");
    let expected = std::fs::canonicalize(directory.path()).expect("canonical temp dir");
    let policy = policy(&["/bin/pwd"]);
    let response = request(
        &policy,
        "spawnSync",
        "/bin/pwd",
        &[],
        json!({ "cwd": directory.path().to_str().expect("UTF-8 temp path") }),
    );
    let result = ok_result(&response);
    assert_eq!(result["status"], 0);
    assert_eq!(
        result["stdout"].as_str().expect("stdout").trim(),
        expected.to_str().expect("UTF-8 canonical path")
    );
}

#[test]
fn the_environment_is_empty_unless_explicitly_provided() {
    require("/bin/sh");
    let policy = policy(&["/bin/sh"]);
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "printf %s \"${HOME-unset}\""],
        json!({}),
    );
    let result = ok_result(&response);
    assert_eq!(result["stdout"], "unset");

    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "printf %s \"$NEOJS_TEST\""],
        json!({ "env": { "NEOJS_TEST": "explicit" } }),
    );
    let result = ok_result(&response);
    assert_eq!(result["stdout"], "explicit");
}

#[test]
fn arguments_are_not_interpreted_by_a_shell() {
    require("/bin/echo");
    let policy = policy(&["/bin/echo"]);
    let response = request(&policy, "spawnSync", "/bin/echo", &["a;b"], json!({}));
    let result = ok_result(&response);
    assert_eq!(result["stdout"], "a;b\n");
}

#[test]
fn a_non_zero_exit_keeps_its_status() {
    require("/bin/sh");
    let policy = policy(&["/bin/sh"]);
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "exit 7"],
        json!({}),
    );
    let result = ok_result(&response);
    assert_eq!(result["status"], 7);
    assert_eq!(result["timedOut"], false);
}

#[test]
fn a_timeout_kills_the_direct_child_and_reports_it() {
    require("/bin/sleep");
    let policy = policy(&["/bin/sleep"]);
    let started = Instant::now();
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sleep",
        &["5"],
        json!({ "timeout": 50 }),
    );
    let result = ok_result(&response);
    assert_eq!(result["timedOut"], true);
    assert_eq!(result["status"], Value::Null);
    assert_eq!(result["signalName"], "SIGKILL");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "timeout did not terminate the direct child promptly"
    );
}

#[test]
fn simultaneous_stdin_and_stdout_do_not_deadlock() {
    require("/bin/sh");
    let policy = policy(&["/bin/sh"]);
    let input = "0123456789".repeat(8_192);
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "cat; printf err >&2"],
        json!({ "input": input, "maxBuffer": 1_048_576 }),
    );
    let result = ok_result(&response);
    assert_eq!(result["status"], 0);
    assert_eq!(result["stdout"].as_str().expect("stdout"), input);
    assert_eq!(result["stderr"], "err");
}

#[test]
fn an_output_overflow_is_bounded_and_reported() {
    require("/bin/sh");
    let policy = policy(&["/bin/sh"]);
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "printf 0123456789"],
        json!({ "maxBuffer": 4 }),
    );
    let result = ok_result(&response);
    assert_eq!(result["limitExceeded"], true);
    assert_eq!(result["stdout"], "0123");
    assert_eq!(result["status"], Value::Null);
}

#[test]
fn an_empty_policy_denies_execution_before_spawning() {
    require("/bin/sh");
    let directory = tempfile::tempdir().expect("temp dir");
    let marker = directory.path().join("must-not-exist");
    let policy = ChildProcessPolicy::new();
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", &format!("touch {}", marker.display())],
        json!({}),
    );
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "ERR_NODE_CHILD_PROCESS_DENIED");
    assert!(!marker.exists(), "a denied program created a side effect");
}

#[test]
fn a_relative_program_is_rejected_before_a_path_search() {
    let policy = policy(&["/bin/echo"]);
    let response = request(&policy, "spawnSync", "echo", &["hello"], json!({}));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "ERR_NODE_COMMAND_NOT_ABSOLUTE");
}

#[test]
fn limits_are_enforced_before_spawning() {
    require("/bin/sh");
    let policy = policy(&["/bin/sh"]).with_limits(
        Limits::default()
            .with_max_timeout(Duration::from_millis(100))
            .with_max_output_bytes(8),
    );
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "true"],
        json!({ "timeout": 101 }),
    );
    assert_eq!(response["error"]["code"], "ERR_NODE_LIMIT");
    let response = request(
        &policy,
        "spawnSync",
        "/bin/sh",
        &["-c", "true"],
        json!({ "maxBuffer": 9 }),
    );
    assert_eq!(response["error"]["code"], "ERR_NODE_LIMIT");
}

#[test]
fn only_the_two_synchronous_operations_are_accepted() {
    let policy = ChildProcessPolicy::new();
    let response = request(&policy, "spawn", "/bin/echo", &[], json!({}));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "ERR_NODE_UNSUPPORTED");
}

#[test]
fn malformed_requests_are_rejected_without_echoing_their_contents() {
    let policy = ChildProcessPolicy::new();
    let response = handle_request(
        r#"{"operation":"spawnSync","file":"/bin/echo","args":[],"options":{"secret":"do-not-echo"}}"#,
        &NodeOptions::new(policy),
    );
    assert!(response.contains("ERR_INVALID_ARGUMENT"), "{response}");
    assert!(!response.contains("do-not-echo"), "{response}");
}

#[test]
fn environment_values_are_not_in_errors() {
    require("/bin/echo");
    let policy = ChildProcessPolicy::new();
    let payload = json!({
        "operation": "spawnSync",
        "file": "/bin/echo",
        "args": [],
        "options": { "env": { "NEOJS_SECRET": "do-not-echo" } },
    });
    let response = handle_request(&payload.to_string(), &NodeOptions::new(policy));
    assert!(!response.contains("do-not-echo"), "{response}");
}
