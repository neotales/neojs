//! Native V8 binding for `neo:strings`.
//!
//! The folded matching and trimming logic lives in the engine-neutral
//! `neojs_builtins::strings` module. This file creates V8 function objects that
//! call it, registers a native V8 synthetic module with those named exports,
//! and installs the same object in the realm's built-in registry. There is no
//! ECMAScript implementation and no ECMAScript module facade.
//!
//! The binding reads and writes UTF-16 code units directly. Converting through
//! a Rust `String` would replace a lone surrogate with U+FFFD and break the
//! module's documented identity rule for lone surrogates.

use deno_core::FastStaticString;
use deno_core::JsRuntime;
use deno_core::v8;

use neojs_builtins::strings as logic;

/// Counts engine text reads for the streaming-search tests.
#[cfg(feature = "test-support")]
static TEXT_UNITS_READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Counts engine windows read for the streaming-search tests.
#[cfg(feature = "test-support")]
static TEXT_WINDOWS_READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Returns `(units, windows)` copied from V8 strings by the streaming search.
#[cfg(feature = "test-support")]
pub(crate) fn test_reads() -> (usize, usize) {
    (
        TEXT_UNITS_READ.load(std::sync::atomic::Ordering::Relaxed),
        TEXT_WINDOWS_READ.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Resets the streaming-search read counters.
#[cfg(feature = "test-support")]
pub(crate) fn reset_test_reads() {
    TEXT_UNITS_READ.store(0, std::sync::atomic::Ordering::Relaxed);
    TEXT_WINDOWS_READ.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// The global that carries the native built-in entry to the realm shim.
///
/// The shim reads it once, builds the registry, and the runtime removes it
/// immediately afterwards. It is not a user-facing global.
pub(crate) const NATIVE_ENTRIES_GLOBAL: &str = "__neoNativeStrings";

/// Installs the native `neo:strings` object, its module, and the shim entry.
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
    js.register_native_module(logic::SPECIFIER, exports)
        .map_err(|error| error.to_string())?;
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
        .ok_or_else(|| "cannot allocate the native strings entries key".to_string())?;
    global
        .delete(scope, key.into())
        .ok_or_else(|| "cannot remove the native strings entries global".to_string())?;
    Ok(())
}

/// Creates one native function per export and returns the globals for the
/// synthetic module registration.
fn export_functions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
) -> Vec<(FastStaticString, v8::Global<v8::Value>)> {
    let mut exports = Vec::with_capacity(logic::EXPORTS.len());
    macro_rules! export {
        ($name:literal, $function:ident, $length:literal) => {{
            let function = v8::Function::new(scope, $function)
                .expect("a native strings function allocation failed");
            set_function_metadata(scope, &function, $name, $length);
            set_export(
                scope,
                object,
                &mut exports,
                deno_core::ascii_str!($name),
                function,
            );
        }};
    }
    export!("equalFold", equal_fold, 2);
    export!("startsWithFold", starts_with_fold, 2);
    export!("endsWithFold", ends_with_fold, 2);
    export!("indexOfFold", index_of_fold, 3);
    export!("lastIndexOfFold", last_index_of_fold, 3);
    export!("trim", trim, 2);
    export!("trimStart", trim_start, 2);
    export!("trimEnd", trim_end, 2);
    exports
}

/// Restores the function `name` and `length` the ECMAScript implementation had.
///
/// `v8::Function::new` creates an anonymous callback with length zero. Both are
/// configurable own properties, so the binding replaces them with the declared
/// name and parameter count. They stay non-writable and non-enumerable.
fn set_function_metadata(
    scope: &mut v8::PinScope<'_, '_>,
    function: &v8::Local<'_, v8::Function>,
    name: &str,
    length: i32,
) {
    let name_value = v8::String::new(scope, name).expect("a function name fits a V8 string");
    set_function_property(scope, function, "name", name_value.into());
    let length_value = v8::Integer::new(scope, length);
    set_function_property(scope, function, "length", length_value.into());
}

/// Defines one non-writable, non-enumerable, configurable function property.
fn set_function_property(
    scope: &mut v8::PinScope<'_, '_>,
    function: &v8::Local<'_, v8::Function>,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let key = v8::String::new(scope, key).expect("a function property key fits a V8 string");
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(value, false);
    descriptor.set_configurable(true);
    descriptor.set_enumerable(false);
    assert!(
        function.define_property(scope, key.into(), &descriptor) == Some(true),
        "setting a native function property failed"
    );
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

/// Returns a required string argument as UTF-16 code units.
///
/// A missing argument is `undefined` and is not a string, so it reports the
/// same `TypeError` as any other non-string value.
fn string_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    message: &str,
) -> Option<Vec<u16>> {
    let value = args.get(index);
    if !value.is_string() {
        throw_type(scope, message);
        return None;
    }
    let text: v8::Local<v8::String> = value.cast();
    let mut units = vec![0u16; text.length()];
    text.write_v2(&*scope, 0, &mut units, v8::WriteFlags::default());
    Some(units)
}

/// Returns a required string argument as a V8 string without copying it.
///
/// The caller builds a [`Utf16Text`] over this value so the streaming search
/// reads only the windows it needs.
fn string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    message: &str,
) -> Option<v8::Local<'s, v8::String>> {
    let value = args.get(index);
    if !value.is_string() {
        throw_type(scope, message);
        return None;
    }
    Some(value.cast())
}

/// A forward-readable V8 string that copies bounded windows on demand.
///
/// The buffer is allocated once with [`logic::WINDOW_UNITS`] capacity. A search
/// with a small limit therefore reads a small prefix instead of the whole
/// string, and a full search keeps only one window alive.
struct Utf16Text<'a, 's, 'i> {
    scope: &'a mut v8::PinScope<'s, 'i>,
    text: v8::Local<'s, v8::String>,
    length: usize,
    read_limit: usize,
    window_start: usize,
    window: Vec<u16>,
}

impl<'a, 's, 'i> Utf16Text<'a, 's, 'i> {
    fn new(scope: &'a mut v8::PinScope<'s, 'i>, text: v8::Local<'s, v8::String>) -> Self {
        Self {
            scope,
            text,
            length: text.length(),
            read_limit: usize::MAX,
            window_start: 0,
            window: Vec::new(),
        }
    }

    fn load(&mut self, start: usize) {
        let end = (start + logic::WINDOW_UNITS)
            .min(self.length)
            .min(self.read_limit.max(start + 1));
        self.window.resize(end - start, 0);
        self.text.write_v2(
            &*self.scope,
            start as u32,
            &mut self.window,
            v8::WriteFlags::default(),
        );
        self.window_start = start;
        #[cfg(feature = "test-support")]
        {
            TEXT_UNITS_READ.fetch_add(self.window.len(), std::sync::atomic::Ordering::Relaxed);
            TEXT_WINDOWS_READ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl logic::Utf16Source for Utf16Text<'_, '_, '_> {
    fn len(&self) -> usize {
        self.length
    }

    fn set_read_limit(&mut self, end: usize) {
        self.read_limit = end;
    }

    fn unit(&mut self, index: usize) -> u16 {
        if index < self.window_start || index >= self.window_start + self.window.len() {
            self.load(index);
        }
        self.window[index - self.window_start]
    }
}

/// Returns an optional string argument, distinguishing `undefined` from a
/// non-string value.
fn optional_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    message: &str,
) -> Result<Option<Vec<u16>>, ()> {
    if args.get(index).is_undefined() {
        return Ok(None);
    }
    string_argument(scope, args, index, message)
        .map(Some)
        .ok_or(())
}

/// Returns an optional numeric index argument.
fn optional_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<f64>, ()> {
    let value = args.get(index);
    if value.is_undefined() {
        return Ok(None);
    }
    if !value.is_number() {
        throw_type(scope, logic::FROM_INDEX_NOT_NUMBER);
        return Err(());
    }
    match value.number_value(&*scope) {
        Some(number) => Ok(Some(number)),
        None => {
            throw_type(scope, logic::FROM_INDEX_NOT_NUMBER);
            Err(())
        }
    }
}

/// Sets a found index, or `-1` when nothing matched.
fn set_index(rv: &mut v8::ReturnValue<'_, v8::Value>, index: Option<usize>) {
    match index {
        Some(index) => rv.set_double(index as f64),
        None => rv.set_int32(-1),
    }
}

/// Sets a string result from UTF-16 code units, preserving lone surrogates.
fn set_string(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    units: &[u16],
) {
    let text = v8::String::new_from_two_byte(scope, units, v8::NewStringType::Normal)
        .expect("a strings result fits a V8 string");
    rv.set(text.into());
}

fn equal_fold<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_argument(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let Some(other) = string_argument(scope, &args, 1, logic::OTHER_MESSAGE) else {
        return;
    };
    rv.set_bool(logic::equal_fold(&text, &other));
}

fn starts_with_fold<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_argument(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let Some(prefix) = string_argument(scope, &args, 1, logic::PREFIX_MESSAGE) else {
        return;
    };
    rv.set_bool(logic::starts_with_fold(&text, &prefix));
}

fn ends_with_fold<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_argument(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let Some(suffix) = string_argument(scope, &args, 1, logic::SUFFIX_MESSAGE) else {
        return;
    };
    rv.set_bool(logic::ends_with_fold(&text, &suffix));
}

fn index_of_fold<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_value(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let Some(search) = string_argument(scope, &args, 1, logic::SEARCH_MESSAGE) else {
        return;
    };
    let from = match optional_index(scope, &args, 2) {
        Ok(from) => from,
        Err(()) => return,
    };
    let result = {
        let mut source = Utf16Text::new(scope, text);
        logic::index_of_fold_source(&mut source, &search, from)
    };
    match result {
        Ok(index) => set_index(&mut rv, index),
        Err(error) => throw_range(scope, error.message()),
    }
}

fn last_index_of_fold<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_value(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let Some(search) = string_argument(scope, &args, 1, logic::SEARCH_MESSAGE) else {
        return;
    };
    let from = match optional_index(scope, &args, 2) {
        Ok(from) => from,
        Err(()) => return,
    };
    let result = {
        let mut source = Utf16Text::new(scope, text);
        logic::last_index_of_fold_source(&mut source, &search, from)
    };
    match result {
        Ok(index) => set_index(&mut rv, index),
        Err(error) => throw_range(scope, error.message()),
    }
}

fn trim<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_argument(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let chars = match optional_string(scope, &args, 1, logic::CHARS_MESSAGE) {
        Ok(chars) => chars,
        Err(()) => return,
    };
    let result = logic::trim(&text, chars.as_deref());
    set_string(scope, &mut rv, &result);
}

fn trim_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_argument(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let chars = match optional_string(scope, &args, 1, logic::CHARS_MESSAGE) {
        Ok(chars) => chars,
        Err(()) => return,
    };
    let result = logic::trim_start(&text, chars.as_deref());
    set_string(scope, &mut rv, &result);
}

fn trim_end<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(text) = string_argument(scope, &args, 0, logic::TEXT_MESSAGE) else {
        return;
    };
    let chars = match optional_string(scope, &args, 1, logic::CHARS_MESSAGE) {
        Ok(chars) => chars,
        Err(()) => return,
    };
    let result = logic::trim_end(&text, chars.as_deref());
    set_string(scope, &mut rv, &result);
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
        return Err("cannot freeze the native strings object".to_string());
    }
    Ok(())
}

/// Installs `globalThis.__neoNativeStrings` as `[specifier, object]`.
fn install_entries_global(
    scope: &mut v8::PinScope<'_, '_>,
    object: &v8::Local<'_, v8::Object>,
) -> Result<(), String> {
    let entry = v8::Array::new(scope, 2);
    let specifier = v8::String::new(scope, logic::SPECIFIER)
        .ok_or_else(|| "cannot allocate the strings specifier".to_string())?;
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
