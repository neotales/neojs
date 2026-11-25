//! RFC 9562 version 4 and version 7 UUID generation for `vael:uuid` on V8.
//!
//! The core RFC 9562 implementation, fallible `getrandom` + `uuid::Builder`
//! handling, version 7 clock bounds, and stable error codes live in the
//! engine-neutral [`vaeljs_builtins::uuid`] module. This file registers the two
//! private V8 host ops (`op_vael_uuid_v4` and `op_vael_uuid_v7`) and converts
//! results into null-prototype response objects for [`crate::builtins`].

use deno_core::op2;
use deno_core::v8;
use vaeljs_builtins::uuid::{UuidError, UuidErrorCode};

/// Test-only, deterministically consumed injection for the next UUID entropy
/// draw. It is compiled only with the off-by-default `test-support` feature, so
/// a normal build has no injection point.
#[cfg(feature = "test-support")]
static FAIL_NEXT_UUID_RANDOM: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(super) fn fail_next_uuid_random() {
    FAIL_NEXT_UUID_RANDOM.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_uuid_random_failure() -> bool {
    FAIL_NEXT_UUID_RANDOM.swap(false, std::sync::atomic::Ordering::SeqCst)
}

#[cfg(feature = "test-support")]
fn fill_random_or_inject(bytes: &mut [u8]) -> Result<(), UuidError> {
    if take_uuid_random_failure() {
        return Err(UuidError::unexpected_random());
    }
    vaeljs_builtins::uuid::fill_random(bytes)
}

fn generate_v4() -> Result<String, UuidError> {
    #[cfg(feature = "test-support")]
    {
        vaeljs_builtins::uuid::v4_with_entropy(fill_random_or_inject)
    }
    #[cfg(not(feature = "test-support"))]
    {
        vaeljs_builtins::uuid::v4()
    }
}

fn generate_v7() -> Result<String, UuidError> {
    #[cfg(feature = "test-support")]
    {
        vaeljs_builtins::uuid::v7_now_with_entropy(fill_random_or_inject)
    }
    #[cfg(not(feature = "test-support"))]
    {
        vaeljs_builtins::uuid::v7()
    }
}

/// Creates the null-prototype result object the JavaScript wrapper reads.
///
/// A null prototype means a script cannot intercept these property writes or
/// reads through `Object.prototype`.
fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

fn set_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    set(scope, object, name, value.into());
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: UuidError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    set_string(scope, object, "name", "Error");
    set_string(scope, object, "code", error.code().as_str());
    set_string(scope, object, "message", error.code().message());
    // Only a fixed diagnostic is exposed. `diagnostic` never returns the
    // `getrandom` Display text, entropy bytes, or a clock value.
    if let Some(cause) = error.diagnostic() {
        set_string(scope, object, "cause", &cause);
    }
}

/// Turns a Rust string result into `{ ok: true, value }` or `{ ok: false, ... }`.
fn outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<String, UuidError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(value) => match v8::String::new(scope, &value) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_failure(scope, object, UuidError::new(UuidErrorCode::Internal)),
        },
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

#[op2]
fn op_vael_uuid_v4<'s, 'i>(scope: &mut v8::PinScope<'s, 'i>) -> v8::Local<'s, v8::Value> {
    outcome(scope, generate_v4())
}

#[op2]
fn op_vael_uuid_v7<'s, 'i>(scope: &mut v8::PinScope<'s, 'i>) -> v8::Local<'s, v8::Value> {
    outcome(scope, generate_v7())
}

deno_core::extension!(
    vaeljs_uuid,
    ops = [op_vael_uuid_v4, op_vael_uuid_v7],
    docs =
        "RFC 9562 UUID version 4 and 7 generation behind `Vael.getBuiltInModule(\"vael:uuid\")`."
);

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn shared_generators_return_canonical_uuids() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for text in [generate_v4().unwrap(), generate_v7().unwrap()] {
            assert_eq!(text.len(), 36);
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn injected_entropy_failure_is_single_shot() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fail_next_uuid_random();
        let error = generate_v4().expect_err("injected random failure");
        assert_eq!(error.code(), UuidErrorCode::Random);
        assert_eq!(
            error.diagnostic().as_deref(),
            Some("getrandom: unexpected error")
        );
        assert!(generate_v4().is_ok());
    }
}
