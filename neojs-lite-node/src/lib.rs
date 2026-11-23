#![forbid(unsafe_code)]
//! QuickJS adapter for opt-in Node compatibility.
//!
//! `neojs_lite_core::Runtime::with_node_compat` installs this bridge and builds
//! its loader with Node specifiers enabled. The bridge is synchronous: it runs
//! the shared child-process core on the thread that owns the QuickJS context.
//! QuickJS has no host-asynchrony loop in this runtime, so no asynchronous
//! `ChildProcess` surface is offered or imitated.
//!
//! The bridge has no other authority. It is not a sandbox, and code in the
//! context can reach the global directly.

use rquickjs::Ctx;
use rquickjs::Function;

pub use neojs_node_builtins::{
    CHILD_PROCESS_SPECIFIER, ChildProcessError, ChildProcessPolicy, Limits, MAX_REQUEST_BYTES,
    NodeOptions, PolicyError, SUPPORTED_MODULES, is_known_node_bare_name, module_source,
    unsupported_module_reason,
};

/// The global object the shared `node:child_process` module source reads.
pub const BRIDGE_GLOBAL: &str = "__neojsNodeChildProcess";

/// The temporary global that holds the raw host function during installation.
///
/// The installer deletes it after capturing the function in the protected
/// wrapper's closure, so guest code cannot call the raw host function directly.
const RAW_GLOBAL: &str = "__neojsNodeRawRun";

/// JavaScript that installs a non-writable bridge over a captured raw host
/// function.
///
/// The raw function is deleted from the global object. The only remaining
/// entry point checks that the request is a string and below the hard length
/// bound before invoking the raw function. A huge direct call is therefore
/// rejected without copying or parsing it. The bridge object and its `runSync`
/// function cannot be replaced after installation.
fn bridge_shim(max_request_chars: usize) -> String {
    format!(
        r#"
(() => {{
    "use strict";
    const raw = globalThis.{raw_global};
    if (typeof raw !== "function") throw new Error("node:child_process raw bridge is missing");
    delete globalThis.{raw_global};
    if (globalThis.{raw_global} !== undefined) throw new Error("node:child_process raw bridge could not be removed");

    const stringify = JSON.stringify;
    const defineProperty = Object.defineProperty;
    const freeze = Object.freeze;
    const create = Object.create;
    const setPrototypeOf = Object.setPrototypeOf;
    const isArray = Array.isArray;
    const isInteger = Number.isInteger;
    const hasOwn = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
    const max = {max_request_chars};

    function failure(code, message) {{
        return stringify({{ ok: false, error: {{ code, message }} }});
    }}

    function record() {{
        return create(null);
    }}

    function list() {{
        const value = [];
        setPrototypeOf(value, null);
        return value;
    }}

    const runSync = freeze(function runSync(request) {{
        if (typeof request !== "string") {{
            return failure("ERR_INVALID_ARG_TYPE", "the node:child_process request must be a string");
        }}
        if (request.length > max) {{
            return failure("ERR_NODE_REQUEST_TOO_LARGE", "the node:child_process request exceeds the hard size limit");
        }}
        return raw(request);
    }});

    defineProperty(globalThis, "{bridge_global}", {{
        value: freeze({{ runSync, stringify, record, list, isArray, isInteger, hasOwn }}),
        writable: false,
        enumerable: false,
        configurable: false,
    }});
}})();
"#,
        raw_global = RAW_GLOBAL,
        bridge_global = BRIDGE_GLOBAL,
        max_request_chars = max_request_chars,
    )
}

/// Installs the protected synchronous host bridge.
///
/// The closure owns a clone of the policy. The raw function is not retained on
/// the global object; only the length-checking wrapper can reach it. The bridge
/// is an internal path to that wrapper, not an isolation boundary.
pub fn install(ctx: &Ctx<'_>, options: NodeOptions) -> rquickjs::Result<()> {
    let run = Function::new(ctx.clone(), move |request: String| -> String {
        neojs_node_builtins::handle_request(&request, &options)
    })?;
    ctx.globals().set(RAW_GLOBAL, run)?;
    ctx.eval::<(), _>(bridge_shim(MAX_REQUEST_BYTES).as_str())?;
    Ok(())
}
