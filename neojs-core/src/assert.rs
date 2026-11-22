//! Private host op for the `assert` built-in's observation of a genuine
//! native Promise.
//!
//! The JavaScript assertion module must read a Promise's settlement without
//! calling a user-defined `then` and without creating a derived promise through
//! a user-defined `Symbol.species`. This op uses V8's internal Promise
//! observation, which attaches the supplied handlers directly and creates no
//! user-visible derived promise. It adds no filesystem, network, environment,
//! or permission capability.

use deno_core::op2;
use deno_core::v8;
use deno_error::JsErrorBox;

/// Attach fulfillment and rejection handlers to a genuine native Promise.
///
/// The JavaScript module calls this only after its engine-level internal-slot
/// check confirms the value is a Promise. The handlers receive the settlement
/// value or reason, and this op returns `false` instead of attaching if V8
/// declines the observation. It never reads a user `then` property and never
/// applies `Symbol.species`.
#[op2(fast)]
pub fn op_neo_assert_observe_promise<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    promise: v8::Local<'s, v8::Promise>,
    on_fulfilled: v8::Local<'s, v8::Function>,
    on_rejected: v8::Local<'s, v8::Function>,
) -> Result<(), JsErrorBox> {
    promise
        .then2(scope, on_fulfilled, on_rejected)
        .ok_or_else(|| JsErrorBox::generic("could not observe the Promise"))?;
    Ok(())
}

deno_core::extension!(
    neojs_assert,
    ops = [op_neo_assert_observe_promise],
    docs = "Private Promise-observation op behind the `assert` built-in."
);
