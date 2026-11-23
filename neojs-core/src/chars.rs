//! Native V8 binding for `neo:chars`.
//!
//! The classification and case-mapping logic lives in the engine-neutral
//! `neojs_builtins::chars` module. This file creates V8 function objects that
//! call it, registers a native V8 synthetic module with those named exports,
//! and installs the same object in the realm's built-in registry. There is no
//! ECMAScript implementation and no ECMAScript module facade.

use deno_core::FastStaticString;
use deno_core::JsRuntime;
use deno_core::v8;

use neojs_builtins::chars as logic;

/// The global that carries the native built-in entries to the realm shim.
///
/// The shim reads it once, builds the registry, and the runtime removes it
/// immediately afterwards. It is not a user-facing global.
pub(crate) const NATIVE_ENTRIES_GLOBAL: &str = "__neoNativeBuiltins";

/// Installs the native `neo:chars` object, its module, and the shim entries.
///
/// The object is frozen. Every named export is a native V8 function object and
/// the same function object is the matching registry property and module
/// export. The synthetic module is registered eagerly in the normal module map,
/// so static imports, dynamic imports, and the loader all see one module
/// instance.
pub(crate) fn install(js: &mut JsRuntime) -> Result<(), String> {
    let context = js.main_context();
    let exports = {
        v8::scope!(let scope, js.v8_isolate());
        let context_local = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context_local);
        let object = v8::Object::new(scope);
        let exports = export_functions(scope, &object);
        freeze(scope, &object)?;
        install_entries_global(scope, &object)?;
        exports
    };
    js.register_native_module(logic::SPECIFIER, exports);
    Ok(())
}

/// Removes the internal entries global after the realm shim has read it.
///
/// The registry and the native module already hold the object, so the internal
/// global is not needed after installation.
pub(crate) fn remove_entries_global(js: &mut JsRuntime) -> Result<(), String> {
    let context = js.main_context();
    v8::scope!(let scope, js.v8_isolate());
    let context_local = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, context_local);
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, NATIVE_ENTRIES_GLOBAL)
        .ok_or_else(|| "cannot allocate the native entries key".to_string())?;
    global
        .delete(scope, key.into())
        .ok_or_else(|| "cannot remove the native entries global".to_string())?;
    Ok(())
}

/// Creates one native function per export and returns the globals for the
/// synthetic module registration.
fn export_functions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
) -> Vec<(FastStaticString, v8::Global<v8::Value>)> {
    let mut exports = Vec::with_capacity(logic::EXPORTS.len());
    macro_rules! predicate {
        ($name:literal, $function:ident) => {{
            let function = v8::Function::new(scope, $function)
                .expect("a native predicate function allocation failed");
            set_export(
                scope,
                object,
                &mut exports,
                deno_core::ascii_str!($name),
                function,
            );
        }};
    }
    predicate!("isChar", is_char);
    predicate!("isAscii", is_ascii);
    predicate!("isLatin1", is_latin1);
    predicate!("isControl", is_control);
    predicate!("isDigit", is_digit);
    predicate!("isLetter", is_letter);
    predicate!("isLetterOrDigit", is_letter_or_digit);
    predicate!("isLower", is_lower);
    predicate!("isUpper", is_upper);
    predicate!("isPunc", is_punc);
    predicate!("isSpace", is_space);
    predicate!("isSymbol", is_symbol);

    let to_lower = v8::Function::new(scope, to_lower)
        .expect("a native case-mapping function allocation failed");
    set_export(
        scope,
        object,
        &mut exports,
        deno_core::ascii_str!("toLower"),
        to_lower,
    );

    let to_upper = v8::Function::new(scope, to_upper)
        .expect("a native case-mapping function allocation failed");
    set_export(
        scope,
        object,
        &mut exports,
        deno_core::ascii_str!("toUpper"),
        to_upper,
    );

    exports
}

/// Sets one named export on the object and records its global.
fn set_export<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    exports: &mut Vec<(FastStaticString, v8::Global<v8::Value>)>,
    name: FastStaticString,
    function: v8::Local<'s, v8::Function>,
) {
    let key = v8::String::new(scope, name.as_str()).expect("an export name is a valid V8 string");
    let value: v8::Local<'s, v8::Value> = function.into();
    assert!(
        object.set(scope, key.into(), value) == Some(true),
        "setting a native export property failed"
    );
    exports.push((name, v8::Global::new(scope, value)));
}

/// Returns a numeric argument, or `None` for a missing or non-number value.
fn number<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
    scope: &v8::PinScope<'s, '_>,
) -> Option<f64> {
    let value = args.get(0);
    // `number_value` performs JavaScript `ToNumber`, so it would accept `null`
    // or a numeric string. The contract requires a number primitive.
    value
        .is_number()
        .then(|| value.number_value(scope))
        .flatten()
}

macro_rules! predicate_callback {
    ($name:ident, $logic:path) => {
        fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            rv.set_bool(number(&args, scope).is_some_and($logic));
        }
    };
}

predicate_callback!(is_char, logic::is_char);
predicate_callback!(is_ascii, logic::is_ascii);
predicate_callback!(is_latin1, logic::is_latin1);
predicate_callback!(is_control, logic::is_control);
predicate_callback!(is_digit, logic::is_digit);
predicate_callback!(is_letter, logic::is_letter);
predicate_callback!(is_letter_or_digit, logic::is_letter_or_digit);
predicate_callback!(is_lower, logic::is_lower);
predicate_callback!(is_upper, logic::is_upper);
predicate_callback!(is_punc, logic::is_punc);
predicate_callback!(is_space, logic::is_space);
predicate_callback!(is_symbol, logic::is_symbol);

/// Case-mapping callback for `toLower`.
fn to_lower<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    case_map(scope, &args, &mut rv, logic::to_lower_number);
}

/// Case-mapping callback for `toUpper`.
fn to_upper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    case_map(scope, &args, &mut rv, logic::to_upper_number);
}

/// Applies a case mapping with the documented `TypeError`/`RangeError`
/// distinction.
fn case_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    map: fn(f64) -> Option<u32>,
) {
    let value = args.get(0);
    let Some(number) = (value.is_number())
        .then(|| value.number_value(scope))
        .flatten()
    else {
        throw_type(scope, logic::NON_NUMBER_MESSAGE);
        return;
    };
    match map(number) {
        Some(value) => rv.set_uint32(value),
        None => throw_range(scope, logic::NON_SCALAR_MESSAGE),
    }
}

fn throw_type(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let message = v8::String::new(scope, message).expect("an error message fits a V8 string");
    let exception = v8::Exception::type_error(scope, message);
    scope.throw_exception(exception);
}

fn throw_range(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let message = v8::String::new(scope, message).expect("an error message fits a V8 string");
    let exception = v8::Exception::range_error(scope, message);
    scope.throw_exception(exception);
}

/// Freezes the public module object.
fn freeze(
    scope: &mut v8::PinScope<'_, '_>,
    object: &v8::Local<'_, v8::Object>,
) -> Result<(), String> {
    if object.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return Err("cannot freeze the native chars object".to_string());
    }
    Ok(())
}

/// Installs `globalThis.__neoNativeBuiltins` as `[[specifier, object]]`.
fn install_entries_global(
    scope: &mut v8::PinScope<'_, '_>,
    object: &v8::Local<'_, v8::Object>,
) -> Result<(), String> {
    let entry = v8::Array::new(scope, 2);
    let specifier = v8::String::new(scope, logic::SPECIFIER)
        .ok_or_else(|| "cannot allocate the chars specifier".to_string())?;
    entry
        .set_index(scope, 0, specifier.into())
        .ok_or_else(|| "cannot set the native entry specifier".to_string())?;
    entry
        .set_index(scope, 1, (*object).into())
        .ok_or_else(|| "cannot set the native entry object".to_string())?;
    let entries = v8::Array::new(scope, 1);
    entries
        .set_index(scope, 0, entry.into())
        .ok_or_else(|| "cannot set the native entry".to_string())?;
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, NATIVE_ENTRIES_GLOBAL)
        .ok_or_else(|| "cannot allocate the native entries key".to_string())?;
    global
        .set(scope, key.into(), entries.into())
        .ok_or_else(|| "cannot install the native entries global".to_string())?;
    Ok(())
}
