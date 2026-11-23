//! Native `neo:strings` binding for the QuickJS runtime.
//!
//! Folded matching and trimming logic lives in the engine-neutral
//! `neojs_builtins::strings` module. This file creates QuickJS function objects
//! that call it and exports them from a native QuickJS module. There is no
//! ECMAScript implementation and no ECMAScript module wrapper.
//!
//! The binding converts through QuickJS's UTF-16 accessors, not through a Rust
//! `String`. A lone surrogate is preserved, so it is compared as itself and
//! never matches half of a surrogate pair.

use std::slice;

use rquickjs::{
    Ctx, Exception, Function, Object, Result, Value, function::Opt, module::ModuleDef, qjs,
};

use neojs_builtins::strings as logic;

/// Creates the frozen public module object.
///
/// The object is created once per context. The registry and the native module
/// both use this same object, so each named export is the same function object
/// as the matching registry property.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set(
        "equalFold",
        Function::new(ctx.clone(), equal_fold)?
            .with_name("equalFold")?
            .with_length(2)?,
    )?;
    object.set(
        "startsWithFold",
        Function::new(ctx.clone(), starts_with_fold)?
            .with_name("startsWithFold")?
            .with_length(2)?,
    )?;
    object.set(
        "endsWithFold",
        Function::new(ctx.clone(), ends_with_fold)?
            .with_name("endsWithFold")?
            .with_length(2)?,
    )?;
    object.set(
        "indexOfFold",
        Function::new(ctx.clone(), index_of_fold)?
            .with_name("indexOfFold")?
            .with_length(3)?,
    )?;
    object.set(
        "lastIndexOfFold",
        Function::new(ctx.clone(), last_index_of_fold)?
            .with_name("lastIndexOfFold")?
            .with_length(3)?,
    )?;
    object.set(
        "trim",
        Function::new(ctx.clone(), trim)?
            .with_name("trim")?
            .with_length(2)?,
    )?;
    object.set(
        "trimStart",
        Function::new(ctx.clone(), trim_start)?
            .with_name("trimStart")?
            .with_length(2)?,
    )?;
    object.set(
        "trimEnd",
        Function::new(ctx.clone(), trim_end)?
            .with_name("trimEnd")?
            .with_length(2)?,
    )?;
    super::neo::freeze(ctx, &object)?;
    Ok(object)
}

/// Reads a required string argument as UTF-16 code units.
///
/// A missing argument is `undefined` and reports the same `TypeError` as any
/// other non-string value.
fn required_string<'js>(
    ctx: &Ctx<'js>,
    value: &Opt<Value<'js>>,
    message: &str,
) -> Result<Vec<u16>> {
    let Some(value) = value.0.as_ref() else {
        return Err(Exception::throw_type(ctx, message));
    };
    if !value.is_string() {
        return Err(Exception::throw_type(ctx, message));
    }
    js_units(ctx, value)
}

/// Reads an optional string argument, distinguishing `undefined` from a
/// non-string value.
fn optional_string<'js>(
    ctx: &Ctx<'js>,
    value: &Opt<Value<'js>>,
    message: &str,
) -> Result<Option<Vec<u16>>> {
    let Some(value) = value.0.as_ref() else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    required_string(ctx, &Opt(Some(value.clone())), message).map(Some)
}

/// Reads an optional numeric index argument.
fn optional_index<'js>(ctx: &Ctx<'js>, value: &Opt<Value<'js>>) -> Result<Option<f64>> {
    let Some(value) = value.0.as_ref() else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    if !value.is_number() {
        return Err(Exception::throw_type(ctx, logic::FROM_INDEX_NOT_NUMBER));
    }
    value
        .as_number()
        .map(Some)
        .ok_or_else(|| Exception::throw_type(ctx, logic::FROM_INDEX_NOT_NUMBER))
}

/// Converts a JavaScript string to UTF-16 code units without replacing lone
/// surrogates.
fn js_units<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Vec<u16>> {
    let mut length: qjs::size_t = 0;
    let raw =
        unsafe { qjs::JS_ToCStringLenUTF16(ctx.as_raw().as_ptr(), &mut length, value.as_raw()) };
    if raw.is_null() {
        return Err(rquickjs::Error::Unknown);
    }
    // SAFETY: QuickJS returns `length` initialized UTF-16 code units that stay
    // valid until `JS_FreeCStringUTF16`.
    let units = unsafe { slice::from_raw_parts(raw, length as usize) }.to_vec();
    unsafe { qjs::JS_FreeCStringUTF16(ctx.as_raw().as_ptr(), raw) };
    Ok(units)
}

/// Converts UTF-16 code units to a JavaScript string, preserving lone
/// surrogates.
fn js_string<'js>(ctx: &Ctx<'js>, units: &[u16]) -> Result<Value<'js>> {
    let raw = unsafe {
        qjs::JS_NewStringUTF16(
            ctx.as_raw().as_ptr(),
            units.as_ptr(),
            units.len() as qjs::size_t,
        )
    };
    if raw.tag == i64::from(qjs::JS_TAG_EXCEPTION) {
        return Err(rquickjs::Error::Unknown);
    }
    // SAFETY: the value is a fresh string owned by this context.
    Ok(unsafe { Value::from_raw(ctx.clone(), raw) })
}

/// Returns a found index, or `-1` when nothing matched.
fn index_value(index: Option<usize>) -> f64 {
    index.map_or(-1.0, |index| index as f64)
}

fn equal_fold<'js>(ctx: Ctx<'js>, text: Opt<Value<'js>>, other: Opt<Value<'js>>) -> Result<bool> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let other = required_string(&ctx, &other, logic::OTHER_MESSAGE)?;
    Ok(logic::equal_fold(&text, &other))
}

fn starts_with_fold<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    prefix: Opt<Value<'js>>,
) -> Result<bool> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let prefix = required_string(&ctx, &prefix, logic::PREFIX_MESSAGE)?;
    Ok(logic::starts_with_fold(&text, &prefix))
}

fn ends_with_fold<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    suffix: Opt<Value<'js>>,
) -> Result<bool> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let suffix = required_string(&ctx, &suffix, logic::SUFFIX_MESSAGE)?;
    Ok(logic::ends_with_fold(&text, &suffix))
}

fn index_of_fold<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    search: Opt<Value<'js>>,
    from_index: Opt<Value<'js>>,
) -> Result<f64> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let search = required_string(&ctx, &search, logic::SEARCH_MESSAGE)?;
    let from_index = optional_index(&ctx, &from_index)?;
    match logic::index_of_fold(&text, &search, from_index) {
        Ok(index) => Ok(index_value(index)),
        Err(error) => Err(Exception::throw_range(&ctx, error.message())),
    }
}

fn last_index_of_fold<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    search: Opt<Value<'js>>,
    from_index: Opt<Value<'js>>,
) -> Result<f64> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let search = required_string(&ctx, &search, logic::SEARCH_MESSAGE)?;
    let from_index = optional_index(&ctx, &from_index)?;
    match logic::last_index_of_fold(&text, &search, from_index) {
        Ok(index) => Ok(index_value(index)),
        Err(error) => Err(Exception::throw_range(&ctx, error.message())),
    }
}

fn trim<'js>(ctx: Ctx<'js>, text: Opt<Value<'js>>, chars: Opt<Value<'js>>) -> Result<Value<'js>> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let chars = optional_string(&ctx, &chars, logic::CHARS_MESSAGE)?;
    let result = logic::trim(&text, chars.as_deref());
    js_string(&ctx, &result)
}

fn trim_start<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    chars: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let chars = optional_string(&ctx, &chars, logic::CHARS_MESSAGE)?;
    let result = logic::trim_start(&text, chars.as_deref());
    js_string(&ctx, &result)
}

fn trim_end<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    chars: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let text = required_string(&ctx, &text, logic::TEXT_MESSAGE)?;
    let chars = optional_string(&ctx, &chars, logic::CHARS_MESSAGE)?;
    let result = logic::trim_end(&text, chars.as_deref());
    js_string(&ctx, &result)
}

/// The native QuickJS module served for `neo:strings`.
pub(super) struct StringsModule;

impl ModuleDef for StringsModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in logic::EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(strings) = state.strings.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = strings.clone().restore(ctx)?;
        for name in logic::EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}
