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
use rquickjs::Object;

pub use neojs_node_builtins::{
    CHILD_PROCESS_SPECIFIER, ChildProcessError, ChildProcessPolicy, Limits, NodeOptions,
    PolicyError, SUPPORTED_MODULES, is_known_node_bare_name, module_source,
    unsupported_module_reason,
};

/// The global object the shared `node:child_process` module source reads.
pub const BRIDGE_GLOBAL: &str = "__neojsNodeChildProcess";

/// Installs the synchronous host bridge and stores the options in its closure.
///
/// The closure owns a clone of the policy. The global is an internal bridge, not
/// an isolation boundary; QuickJS code can already call any host function the
/// embedding application installed.
pub fn install(ctx: &Ctx<'_>, options: NodeOptions) -> rquickjs::Result<()> {
    let run = Function::new(ctx.clone(), move |request: String| -> String {
        neojs_node_builtins::handle_request(&request, &options)
    })?;
    let bridge = Object::new(ctx.clone())?;
    bridge.set("runSync", run)?;
    ctx.globals().set(BRIDGE_GLOBAL, bridge)?;
    Ok(())
}
