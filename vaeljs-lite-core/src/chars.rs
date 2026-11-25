//! Native `vael:chars` binding for the QuickJS runtime.
//!
//! The classification and case-mapping logic lives in the engine-neutral
//! `vaeljs_builtins::chars` module. This file creates QuickJS function objects
//! that call it and exports them from a native QuickJS module. There is no
//! ECMAScript implementation and no ECMAScript module wrapper.

use rquickjs::{Ctx, Exception, Function, Object, Result, Value, function::Opt, module::ModuleDef};

use vaeljs_builtins::chars as logic;

/// Creates the frozen public module object.
///
/// The object is created once per context. The registry and the native module
/// both use this same object, so each named export is the same function object
/// as the matching registry property.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("isChar", Function::new(ctx.clone(), is_char))?;
    object.set("isAscii", Function::new(ctx.clone(), is_ascii))?;
    object.set("isLatin1", Function::new(ctx.clone(), is_latin1))?;
    object.set("isControl", Function::new(ctx.clone(), is_control))?;
    object.set("isDigit", Function::new(ctx.clone(), is_digit))?;
    object.set("isLetter", Function::new(ctx.clone(), is_letter))?;
    object.set(
        "isLetterOrDigit",
        Function::new(ctx.clone(), is_letter_or_digit),
    )?;
    object.set("isLower", Function::new(ctx.clone(), is_lower))?;
    object.set("isUpper", Function::new(ctx.clone(), is_upper))?;
    object.set("isPunc", Function::new(ctx.clone(), is_punc))?;
    object.set("isSpace", Function::new(ctx.clone(), is_space))?;
    object.set("isSymbol", Function::new(ctx.clone(), is_symbol))?;
    object.set("toLower", Function::new(ctx.clone(), to_lower)?)?;
    object.set("toUpper", Function::new(ctx.clone(), to_upper)?)?;
    super::vael::freeze(ctx, &object)?;
    Ok(object)
}

fn is_char(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_char)
}

fn is_ascii(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_ascii)
}

fn is_latin1(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_latin1)
}

fn is_control(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_control)
}

fn is_digit(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_digit)
}

fn is_letter(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_letter)
}

fn is_letter_or_digit(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_letter_or_digit)
}

fn is_lower(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_lower)
}

fn is_upper(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_upper)
}

fn is_punc(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_punc)
}

fn is_space(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_space)
}

fn is_symbol(value: Opt<Value<'_>>) -> bool {
    number(&value).is_some_and(logic::is_symbol)
}

/// Returns the numeric value of an argument, or `None` for a missing argument
/// or a value that is not a JavaScript number.
fn number(value: &Opt<Value<'_>>) -> Option<f64> {
    value.0.as_ref().and_then(Value::as_number)
}

fn to_lower<'js>(ctx: Ctx<'js>, value: Opt<Value<'js>>) -> Result<u32> {
    case_map(&ctx, &value, logic::to_lower_number)
}

fn to_upper<'js>(ctx: Ctx<'js>, value: Opt<Value<'js>>) -> Result<u32> {
    case_map(&ctx, &value, logic::to_upper_number)
}

/// Applies a case mapping with the documented `TypeError`/`RangeError`
/// distinction.
fn case_map<'js>(
    ctx: &Ctx<'js>,
    value: &Opt<Value<'js>>,
    map: fn(f64) -> Option<u32>,
) -> Result<u32> {
    let Some(number) = value.0.as_ref().and_then(Value::as_number) else {
        return Err(Exception::throw_type(ctx, logic::NON_NUMBER_MESSAGE));
    };
    map(number).ok_or_else(|| Exception::throw_range(ctx, logic::NON_SCALAR_MESSAGE))
}

/// The native QuickJS module served for `vael:chars`.
pub(super) struct CharsModule;

impl ModuleDef for CharsModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in logic::EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(chars) = state.chars.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = chars.clone().restore(ctx)?;
        for name in logic::EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}
