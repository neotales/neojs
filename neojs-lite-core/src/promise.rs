//! Native QuickJS Promise inspection and observation helpers.
//!
//! Both `neo:assert` and `neo:test` must distinguish a genuine native Promise
//! (by its internal QuickJS class id, not its prototype chain) and observe its
//! settlement without reading a user-overridden `then` property or invoking
//! `Symbol.species`.

use rquickjs::{Ctx, Exception, Result, Value, function::Opt, qjs};

/// Returns whether `value` is a genuine native Promise.
///
/// Checks QuickJS's internal `JS_CLASS_PROMISE` tag without reading any
/// properties, consulting `Promise.prototype`, or running Proxy traps.
pub(super) fn is_native_promise(value: Opt<Value<'_>>) -> bool {
    value
        .0
        .as_ref()
        .is_some_and(|value| unsafe { qjs::JS_IsPromise(value.as_raw()) })
}

/// Attaches fulfillment and rejection handlers directly to a native Promise.
///
/// Uses QuickJS's `JS_PromiseThen` C API, which bypasses `Promise.prototype.then`,
/// `constructor`, and `Symbol.species` getters. Marks the unused derived
/// promise as handled before freeing it so callback errors cannot create a
/// spurious unhandled rejection.
pub(super) fn observe_promise<'js>(
    ctx: Ctx<'js>,
    promise: Opt<Value<'js>>,
    on_fulfilled: Opt<Value<'js>>,
    on_rejected: Opt<Value<'js>>,
) -> Result<()> {
    let Some(promise) = promise.0 else {
        return Err(Exception::throw_type(&ctx, "expected a Promise"));
    };
    if unsafe { !qjs::JS_IsPromise(promise.as_raw()) } {
        return Err(Exception::throw_type(&ctx, "expected a Promise"));
    }
    let Some(on_fulfilled) = on_fulfilled.0.and_then(|value| value.into_function()) else {
        return Err(Exception::throw_type(
            &ctx,
            "onFulfilled must be a function",
        ));
    };
    let Some(on_rejected) = on_rejected.0.and_then(|value| value.into_function()) else {
        return Err(Exception::throw_type(&ctx, "onRejected must be a function"));
    };

    let raw_ctx = ctx.as_raw().as_ptr();
    // SAFETY: `raw_ctx` is live for this call and `promise`, `on_fulfilled`,
    // and `on_rejected` are rooted `Value`/`Function` handles for this call.
    let derived = unsafe {
        qjs::JS_PromiseThen(
            raw_ctx,
            promise.as_raw(),
            on_fulfilled.as_raw(),
            on_rejected.as_raw(),
        )
    };
    if unsafe { qjs::JS_IsException(derived) } {
        return Err(rquickjs::Error::Exception);
    }
    unsafe {
        qjs::JS_PromiseMarkAsHandled(raw_ctx, derived);
        qjs::JS_FreeValue(raw_ctx, derived);
    }
    Ok(())
}
