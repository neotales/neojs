//! The JSON request/response protocol shared by both engine adapters.
//!
//! The protocol is deliberately small and strict. It is not a public transport:
//! it exists so the V8 op and the QuickJS host function call one runner without
//! duplicating validation or result shaping. A malformed request is rejected
//! without echoing its contents, and the runner re-validates every limit.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::json;

use crate::child_process::MAX_REQUEST_BYTES;
use crate::child_process::NodeOptions;
use crate::child_process::SyncRequest;
use crate::child_process::run_sync;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    file: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    options: Options,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    cwd: Option<String>,
    env: Option<BTreeMap<String, String>>,
    input: Option<String>,
    timeout: Option<u64>,
    max_buffer: Option<usize>,
}

/// Runs one JSON request under `options` and returns a JSON response.
///
/// The only operations are `spawnSync` and `execFileSync`. Both use the same
/// runner; the JavaScript wrapper decides whether a non-zero exit is returned
/// as a result or thrown. Unknown operations are rejected rather than treated
/// as an alias for a supported one.
pub fn handle_request(request: &str, options: &NodeOptions) -> String {
    if request.len() > MAX_REQUEST_BYTES {
        return error_response(
            "ERR_NODE_REQUEST_TOO_LARGE",
            "the node:child_process request exceeds the hard size limit",
        );
    }
    let parsed: Request = match serde_json::from_str(request) {
        Ok(parsed) => parsed,
        Err(_) => {
            return error_response(
                "ERR_INVALID_ARGUMENT",
                "the node:child_process request is malformed",
            );
        }
    };
    if !matches!(parsed.operation.as_str(), "spawnSync" | "execFileSync") {
        return error_response(
            "ERR_NODE_UNSUPPORTED",
            "only spawnSync and execFileSync are implemented",
        );
    }

    let request = SyncRequest {
        file: parsed.file,
        args: parsed.args,
        cwd: parsed.options.cwd,
        env: parsed.options.env,
        input: parsed.options.input,
        timeout_ms: parsed.options.timeout,
        max_buffer: parsed.options.max_buffer,
    };
    match run_sync(&request, options) {
        Ok(result) => json!({
            "ok": true,
            "result": {
                "pid": result.pid,
                "status": result.status,
                "signalName": result.signal_name,
                "stdout": String::from_utf8_lossy(&result.stdout),
                "stderr": String::from_utf8_lossy(&result.stderr),
                "timedOut": result.timed_out,
                "limitExceeded": result.limit_exceeded,
                "incomplete": result.incomplete,
                "ioError": result.io_error,
            }
        })
        .to_string(),
        Err(error) => error_response(error.code(), error.message()),
    }
}

/// Builds the one error shape both wrappers understand.
fn error_response(code: &str, message: &str) -> String {
    json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message,
        }
    })
    .to_string()
}
