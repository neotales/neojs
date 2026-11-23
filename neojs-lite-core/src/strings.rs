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

use rquickjs::function::This;
use rquickjs::{
    Ctx, Exception, Function, Object, Result, Value, function::Opt, module::ModuleDef, qjs,
};

use neojs_builtins::strings as logic;

/// Counts engine text units copied for the streaming-search tests.
#[cfg(feature = "test-support")]
static TEXT_UNITS_READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Counts engine windows copied for the streaming-search tests.
#[cfg(feature = "test-support")]
static TEXT_WINDOWS_READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Counts every source read attempt, including the one that fails.
#[cfg(feature = "test-support")]
static TEXT_READ_ATTEMPTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Number of successful windows before an injected read failure.
#[cfg(feature = "test-support")]
static FAIL_AFTER_LOADS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Sentinel for no injected read failure.
#[cfg(feature = "test-support")]
const NO_INJECTED_FAILURE: usize = usize::MAX;

/// Returns `(units, windows)` copied from QuickJS strings by the streaming
/// search.
#[cfg(feature = "test-support")]
pub(crate) fn test_reads() -> (usize, usize) {
    (
        TEXT_UNITS_READ.load(std::sync::atomic::Ordering::Relaxed),
        TEXT_WINDOWS_READ.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Returns the number of source read attempts, including a failed one.
#[cfg(feature = "test-support")]
pub(crate) fn test_attempts() -> usize {
    TEXT_READ_ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Resets the streaming-search read counters and injected failure.
#[cfg(feature = "test-support")]
pub(crate) fn reset_test_reads() {
    TEXT_UNITS_READ.store(0, std::sync::atomic::Ordering::Relaxed);
    TEXT_WINDOWS_READ.store(0, std::sync::atomic::Ordering::Relaxed);
    TEXT_READ_ATTEMPTS.store(0, std::sync::atomic::Ordering::Relaxed);
    FAIL_AFTER_LOADS.store(NO_INJECTED_FAILURE, std::sync::atomic::Ordering::Relaxed);
}

/// Makes the read after `loads` successful windows fail with a guest-visible
/// exception.
#[cfg(feature = "test-support")]
pub(crate) fn fail_after_loads(loads: usize) {
    FAIL_AFTER_LOADS.store(loads, std::sync::atomic::Ordering::Relaxed);
}

/// Returns whether the current load should be the injected failure.
#[cfg(feature = "test-support")]
fn should_fail_load() -> bool {
    let remaining = FAIL_AFTER_LOADS.load(std::sync::atomic::Ordering::Relaxed);
    if remaining == NO_INJECTED_FAILURE {
        return false;
    }
    if remaining == 0 {
        FAIL_AFTER_LOADS.store(NO_INJECTED_FAILURE, std::sync::atomic::Ordering::Relaxed);
        return true;
    }
    FAIL_AFTER_LOADS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    false
}

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

/// Reads a required string argument without copying it.
fn required_string_value<'js>(
    ctx: &Ctx<'js>,
    value: &Opt<Value<'js>>,
    message: &str,
) -> Result<Value<'js>> {
    let Some(value) = value.0.as_ref() else {
        return Err(Exception::throw_type(ctx, message));
    };
    if !value.is_string() {
        return Err(Exception::throw_type(ctx, message));
    }
    Ok(value.clone())
}

/// Returns the UTF-16 length of a string primitive without converting it.
fn js_length<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<usize> {
    let raw = unsafe {
        qjs::JS_GetProperty(
            ctx.as_raw().as_ptr(),
            value.as_raw(),
            qjs::JS_ATOM_length as qjs::JSAtom,
        )
    };
    if raw.tag == i64::from(qjs::JS_TAG_EXCEPTION) {
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        });
    }
    let length = unsafe { Value::from_raw(ctx.clone(), raw) };
    length
        .as_number()
        .map(|length| length as usize)
        .ok_or(rquickjs::Error::Unknown)
}

/// A forward-readable QuickJS string that copies bounded windows on demand.
///
/// QuickJS has no public per-code-unit accessor. The adapter calls the
/// captured `String.prototype.slice` intrinsic and converts only one bounded
/// window at a time, so a small `fromIndex` does not copy the whole string.
struct Utf16Text<'js> {
    ctx: Ctx<'js>,
    text: Value<'js>,
    slice: Function<'js>,
    length: usize,
    read_limit: usize,
    window_start: usize,
    window: Vec<u16>,
}

impl<'js> Utf16Text<'js> {
    fn new(ctx: &Ctx<'js>, text: &Value<'js>) -> Result<Self> {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let slice = state
            .slice
            .as_ref()
            .ok_or(rquickjs::Error::Unknown)?
            .clone()
            .restore(ctx)?;
        let length = js_length(ctx, text)?;
        Ok(Self {
            ctx: ctx.clone(),
            text: text.clone(),
            slice,
            length,
            read_limit: usize::MAX,
            window_start: 0,
            window: Vec::new(),
        })
    }

    fn load(&mut self, start: usize) -> Result<()> {
        #[cfg(feature = "test-support")]
        if should_fail_load() {
            return Err(Exception::throw_message(
                &self.ctx,
                "injected strings read failure",
            ));
        }
        let end = (start + logic::WINDOW_UNITS)
            .min(self.length)
            .min(self.read_limit.max(start + 1));
        let piece: Value<'js> = self.slice.call((This(self.text.clone()), start, end))?;
        let units = js_units(&self.ctx, &piece)?;
        self.window = units;
        self.window_start = start;
        #[cfg(feature = "test-support")]
        {
            TEXT_UNITS_READ.fetch_add(self.window.len(), std::sync::atomic::Ordering::Relaxed);
            TEXT_WINDOWS_READ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }
}

impl logic::Utf16Source for Utf16Text<'_> {
    type Error = rquickjs::Error;

    fn len(&self) -> usize {
        self.length
    }

    fn set_read_limit(&mut self, end: usize) -> std::result::Result<(), Self::Error> {
        self.read_limit = end;
        Ok(())
    }

    fn unit(&mut self, index: usize) -> std::result::Result<u16, Self::Error> {
        #[cfg(feature = "test-support")]
        TEXT_READ_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if index < self.window_start || index >= self.window_start + self.window.len() {
            self.load(index)?;
        }
        self.window
            .get(index - self.window_start)
            .copied()
            .ok_or(rquickjs::Error::Unknown)
    }
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
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        });
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
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        });
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
    let text = required_string_value(&ctx, &text, logic::TEXT_MESSAGE)?;
    let search = required_string(&ctx, &search, logic::SEARCH_MESSAGE)?;
    let from_index = optional_index(&ctx, &from_index)?;
    let mut source = Utf16Text::new(&ctx, &text)?;
    match logic::index_of_fold_source(&mut source, &search, from_index) {
        Ok(index) => Ok(index_value(index)),
        Err(logic::SearchError::Index(error)) => Err(Exception::throw_range(&ctx, error.message())),
        Err(logic::SearchError::Source(error)) => Err(error),
    }
}

fn last_index_of_fold<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    search: Opt<Value<'js>>,
    from_index: Opt<Value<'js>>,
) -> Result<f64> {
    let text = required_string_value(&ctx, &text, logic::TEXT_MESSAGE)?;
    let search = required_string(&ctx, &search, logic::SEARCH_MESSAGE)?;
    let from_index = optional_index(&ctx, &from_index)?;
    let mut source = Utf16Text::new(&ctx, &text)?;
    match logic::last_index_of_fold_source(&mut source, &search, from_index) {
        Ok(index) => Ok(index_value(index)),
        Err(logic::SearchError::Index(error)) => Err(Exception::throw_range(&ctx, error.message())),
        Err(logic::SearchError::Source(error)) => Err(error),
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
