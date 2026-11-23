#![forbid(unsafe_code)]
//! V8 adapter for opt-in Node compatibility.
//!
//! The runtime creates this extension only from
//! `neojs_core::Runtime::with_node_compat`. The extension registers one
//! synchronous op behind `node:child_process`; it does not add the module to
//! the `Neo` registry, and it does not make `node:` imports resolve unless the
//! loader was built with Node compatibility enabled.
//!
//! The op blocks the isolate thread while the shared runner waits for the
//! direct child. That matches Node's synchronous `spawnSync`/`execFileSync`
//! contract and is bounded by the policy's timeout. It does not drive V8 or the
//! Tokio event loop, and it does not create a fake Promise.

use deno_core::OpState;
use deno_core::op2;
use deno_core::v8;

pub use neojs_node_builtins::{
    CHILD_PROCESS_SPECIFIER, ChildProcessError, ChildProcessPolicy, Limits, MAX_REQUEST_BYTES,
    NodeOptions, PolicyError, SUPPORTED_MODULES, is_known_node_bare_name, module_source,
    unsupported_module_reason,
};

/// JavaScript installed after the normal neojs shims when Node compatibility is
/// enabled.
///
/// The bridge object is non-writable, non-enumerable, and non-configurable. It
/// is an internal path to the private op, not an isolation boundary: code in the
/// realm can already reach `Deno.core.ops` directly. The module wrapper captures
/// the function when it is evaluated.
pub const BRIDGE_SHIM: &str = r#"
(() => {
    "use strict";
    const runSync = Deno.core.ops.op_neojs_node_child_process_run;
    const stringify = JSON.stringify;
    const create = Object.create;
    const setPrototypeOf = Object.setPrototypeOf;
    const isArray = Array.isArray;
    const isInteger = Number.isInteger;
    const hasOwn = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
    const record = () => create(null);
    const list = () => {
        const value = [];
        setPrototypeOf(value, null);
        return value;
    };
    Object.defineProperty(globalThis, "__neojsNodeChildProcess", {
        value: Object.freeze({ runSync, stringify, record, list, isArray, isInteger, hasOwn }),
        writable: false,
        enumerable: false,
        configurable: false,
    });
})();
"#;

struct NodeState {
    options: NodeOptions,
}

/// The response for a direct bridge call whose argument is not a string.
const NOT_A_STRING: &str = r#"{"ok":false,"error":{"code":"ERR_INVALID_ARG_TYPE","message":"the node:child_process request must be a string"}}"#;

/// The response for a direct bridge call above the hard request bound.
const TOO_LARGE: &str = r#"{"ok":false,"error":{"code":"ERR_NODE_REQUEST_TOO_LARGE","message":"the node:child_process request exceeds the hard size limit"}}"#;

/// Returns a JavaScript string value for `text`.
fn js_string<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
    match v8::String::new(scope, text) {
        Some(value) => value.into(),
        None => v8::undefined(scope).into(),
    }
}

/// Runs one JSON request through the shared, engine-neutral child-process core.
///
/// This is a synchronous op, so it blocks the isolate thread. The response is a
/// JSON string built by the shared crate; the JavaScript wrapper converts it to
/// Node-shaped results and errors. The argument is inspected as a V8 string
/// before it is copied to Rust, so a direct `Deno.core.ops` call cannot make the
/// host copy an unbounded string.
#[op2]
fn op_neojs_node_child_process_run<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return js_string(scope, NOT_A_STRING);
    };
    if string.utf8_length(scope) > MAX_REQUEST_BYTES {
        return js_string(scope, TOO_LARGE);
    }
    let request = string.to_rust_string_lossy(scope);
    let options = state.borrow::<NodeState>().options.clone();
    let response = neojs_node_builtins::handle_request(&request, &options);
    js_string(scope, &response)
}

deno_core::extension!(
    neojs_node,
    ops = [op_neojs_node_child_process_run],
    options = { options: NodeOptions },
    state = |state, config| state.put(NodeState {
        options: config.options,
    }),
    docs = "Opt-in synchronous node:child_process support backed by neojs-node-builtins."
);

/// Returns the deno_core extension for a runtime with the given policy.
///
/// The generated extension type has the same name as this crate, so this
/// wrapper gives callers one stable path and keeps the policy parameter in the
/// function signature.
pub fn extension(options: NodeOptions) -> deno_core::Extension {
    neojs_node::init(options)
}
