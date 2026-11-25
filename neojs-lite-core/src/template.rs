//! Native `neo:template` bindings for QuickJS.
//!
//! All Go `text/template` parsing, Sprig helper execution, HTML escaping,
//! layout/partial composition, and resource limits live in the engine-neutral
//! [`neojs_template_core`] crate. This module builds one frozen registry
//! object per context, shares those exact functions across
//! `Neo.getBuiltInModule("neo:template")` and `import ... from "neo:template"`
//! (including the default export), and brands `Template` / `TemplateSet`
//! instances with `rquickjs::Class` and `[[DefineOwnProperty]]` so user code
//! cannot forge handles or hijack methods via `Object.prototype`.

use std::cell::RefCell;
use std::mem::MaybeUninit;

use neojs_template_core::{CompiledTemplate, TemplateError, TemplateOptions, TemplateSet};
use rquickjs::{
    Array, Class, Constructor, Ctx, Function, JsLifetime, Object, Persistent, Result,
    String as JsString, Value,
    class::{JsClass, Readable, Trace, Tracer},
    function::{Rest, This},
    module::ModuleDef,
    object::Property,
    qjs,
};

use crate::native::{freeze_native, parse_options, throw_coded_error};

/// Canonical built-in specifier for the template module.
pub(super) const SPECIFIER: &str = "neo:template";

const EXPORTS: &[&str] = &["render", "compile", "createTemplateSet", "escapeHtml"];
const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
const MAX_DATA_DEPTH: usize = 128;
const MAX_DATA_NODES: usize = 100_000;

struct TemplateBrand(CompiledTemplate);

impl<'js> Trace<'js> for TemplateBrand {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for TemplateBrand {
    type Changed<'to> = TemplateBrand;
}

impl<'js> JsClass<'js> for TemplateBrand {
    const NAME: &'static str = "Template";
    type Mutable = Readable;

    fn constructor(_ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        Ok(None)
    }
}

struct TemplateSetBrand(RefCell<TemplateSet>);

impl<'js> Trace<'js> for TemplateSetBrand {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for TemplateSetBrand {
    type Changed<'to> = TemplateSetBrand;
}

impl<'js> JsClass<'js> for TemplateSetBrand {
    const NAME: &'static str = "TemplateSet";
    type Mutable = Readable;

    fn constructor(_ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        Ok(None)
    }
}

struct PropertyEnumGuard {
    raw: *mut qjs::JSContext,
    tab: *mut qjs::JSPropertyEnum,
    len: u32,
}

impl Drop for PropertyEnumGuard {
    fn drop(&mut self) {
        if !self.tab.is_null() {
            // SAFETY: `tab` and `len` were returned by `JS_GetOwnPropertyNames`
            // on the active `JSContext`.
            unsafe { qjs::JS_FreePropertyEnum(self.raw, self.tab, self.len) };
        }
    }
}

fn template_ctor<'js>(ctx: Ctx<'js>, _args: Rest<Value<'js>>) -> Result<()> {
    Err(throw_coded_type_error(
        &ctx,
        "INVALID_DATA",
        "Template objects cannot be constructed directly; use compile()",
    ))
}

fn template_set_ctor<'js>(ctx: Ctx<'js>, _args: Rest<Value<'js>>) -> Result<()> {
    Err(throw_coded_type_error(
        &ctx,
        "INVALID_DATA",
        "TemplateSet objects cannot be constructed directly; use createTemplateSet()",
    ))
}

/// Builds the frozen `Template.prototype` with a non-constructible constructor.
pub(super) fn build_template_proto<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let ctor = Function::new(ctx.clone(), template_ctor)?
        .with_name("Template")?
        .with_length(0)?;
    freeze_native(ctx, ctor.as_object().ok_or(rquickjs::Error::Unknown)?)?;
    let proto = Object::new(ctx.clone())?;
    proto.prop(
        "constructor",
        Property::from(ctor).writable().configurable(),
    )?;
    freeze_native(ctx, &proto)?;
    Ok(proto)
}

/// Builds the frozen `TemplateSet.prototype` with a non-constructible constructor.
pub(super) fn build_template_set_proto<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let ctor = Function::new(ctx.clone(), template_set_ctor)?
        .with_name("TemplateSet")?
        .with_length(0)?;
    freeze_native(ctx, ctor.as_object().ok_or(rquickjs::Error::Unknown)?)?;
    let proto = Object::new(ctx.clone())?;
    proto.prop(
        "constructor",
        Property::from(ctor).writable().configurable(),
    )?;
    freeze_native(ctx, &proto)?;
    Ok(proto)
}

/// Creates the frozen public registry object for `neo:template`.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set(
        "render",
        Function::new(ctx.clone(), render)?
            .with_name("render")?
            .with_length(3)?,
    )?;
    object.set(
        "compile",
        Function::new(ctx.clone(), compile)?
            .with_name("compile")?
            .with_length(2)?,
    )?;
    object.set(
        "createTemplateSet",
        Function::new(ctx.clone(), create_template_set)?
            .with_name("createTemplateSet")?
            .with_length(1)?,
    )?;
    object.set(
        "escapeHtml",
        Function::new(ctx.clone(), escape_html)?
            .with_name("escapeHtml")?
            .with_length(1)?,
    )?;
    freeze_native(ctx, &object)?;
    Ok(object)
}

fn template_error_code(err: &TemplateError) -> &'static str {
    match err {
        TemplateError::Parse(_) => "PARSE",
        TemplateError::Render(_) => "RENDER",
        TemplateError::LimitExceeded(_) => "LIMIT_EXCEEDED",
        TemplateError::TemplateNotFound(_) => "TEMPLATE_NOT_FOUND",
        TemplateError::InvalidName(_) => "INVALID_NAME",
        TemplateError::InvalidData(_) => "INVALID_DATA",
        _ => "RENDER",
    }
}

fn throw_template_error(ctx: &Ctx<'_>, error: TemplateError) -> rquickjs::Error {
    throw_coded_error(
        ctx,
        Some("TemplateError"),
        template_error_code(&error),
        &error.to_string(),
    )
}

fn throw_coded_native_error<'js>(
    ctx: &Ctx<'js>,
    ctor: Option<&Persistent<Function<'static>>>,
    code: &str,
    message: &str,
) -> rquickjs::Error {
    let Some(ctor) = ctor else {
        return throw_coded_error(ctx, None, code, message);
    };
    let Ok(ctor_fn) = ctor.clone().restore(ctx) else {
        return throw_coded_error(ctx, None, code, message);
    };
    let Ok(err_val) = ctor_fn.call::<_, Value<'js>>((message,)) else {
        return if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        };
    };
    let Some(object) = err_val.into_object() else {
        return throw_coded_error(ctx, None, code, message);
    };
    if let Err(err) = object.prop(
        "code",
        Property::from(code).writable().enumerable().configurable(),
    ) {
        return err;
    }
    ctx.throw(object.into_value())
}

fn throw_coded_type_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    let ctor = ctx
        .userdata::<super::neo::NeoState>()
        .and_then(|s| s.type_error_ctor.clone());
    throw_coded_native_error(ctx, ctor.as_ref(), code, message)
}

fn throw_coded_range_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    let ctor = ctx
        .userdata::<super::neo::NeoState>()
        .and_then(|s| s.range_error_ctor.clone());
    throw_coded_native_error(ctx, ctor.as_ref(), code, message)
}

fn read_strict_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    code: &str,
    type_msg: &str,
    surrogate_err: fn(String) -> TemplateError,
) -> Result<String> {
    if !value.is_string() {
        return Err(throw_coded_type_error(ctx, code, type_msg));
    }
    let js_str = JsString::from_value(value.clone())
        .map_err(|_| throw_coded_type_error(ctx, code, type_msg))?;
    match js_str.to_string() {
        Ok(text) => Ok(text),
        Err(rquickjs::Error::FromJs { .. } | rquickjs::Error::Utf8(_)) => {
            Err(throw_template_error(
                ctx,
                surrogate_err("string contains an unpaired UTF-16 surrogate".to_owned()),
            ))
        }
        Err(other) => Err(other),
    }
}

fn parse_template_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
) -> Result<TemplateOptions> {
    let mut opts = TemplateOptions::default();
    let res = parse_options(
        ctx,
        value,
        "options must be a plain object",
        |ctx, key, val| match key {
            "sprig" => opt_bool(ctx, val, &mut opts.sprig, "options.sprig must be a boolean"),
            "hermetic" => opt_bool(
                ctx,
                val,
                &mut opts.hermetic,
                "options.hermetic must be a boolean",
            ),
            "htmlEscape" => opt_bool(
                ctx,
                val,
                &mut opts.html_escape,
                "options.htmlEscape must be a boolean",
            ),
            "maxSourceLength" => opt_usize(
                ctx,
                val,
                &mut opts.max_source_length,
                "options.maxSourceLength must be a non-negative safe integer",
            ),
            "maxNestingDepth" => opt_usize(
                ctx,
                val,
                &mut opts.max_nesting_depth,
                "options.maxNestingDepth must be a non-negative safe integer",
            ),
            "maxOutputLength" => opt_usize(
                ctx,
                val,
                &mut opts.max_output_length,
                "options.maxOutputLength must be a non-negative safe integer",
            ),
            "maxIterations" => opt_usize(
                ctx,
                val,
                &mut opts.max_iterations,
                "options.maxIterations must be a non-negative safe integer",
            ),
            "maxFunctionCalls" => opt_usize(
                ctx,
                val,
                &mut opts.max_function_calls,
                "options.maxFunctionCalls must be a non-negative safe integer",
            ),
            _ => Err(throw_coded_type_error(
                ctx,
                "INVALID_DATA",
                "unknown template option",
            )),
        },
    );
    match res {
        Ok(()) => Ok(opts),
        Err(rquickjs::Error::Exception) => {
            let caught = ctx.catch();
            if let Some(obj) = caught.as_object() {
                let has_code = obj.contains_key("code").unwrap_or(false);
                if !has_code {
                    let _ = obj.prop(
                        "code",
                        Property::from("INVALID_DATA")
                            .writable()
                            .enumerable()
                            .configurable(),
                    );
                }
            }
            Err(ctx.throw(caught))
        }
        Err(other) => Err(other),
    }
}

fn opt_bool<'js>(ctx: &Ctx<'js>, val: &Value<'js>, target: &mut bool, msg: &str) -> Result<()> {
    if val.is_undefined() {
        return Ok(());
    }
    let Some(b) = val.as_bool() else {
        return Err(throw_coded_type_error(ctx, "INVALID_DATA", msg));
    };
    *target = b;
    Ok(())
}

fn opt_usize<'js>(ctx: &Ctx<'js>, val: &Value<'js>, target: &mut usize, msg: &str) -> Result<()> {
    if val.is_undefined() {
        return Ok(());
    }
    if !val.is_number() {
        return Err(throw_coded_type_error(ctx, "INVALID_DATA", msg));
    }
    let Some(num) = val.as_number() else {
        return Err(throw_coded_type_error(ctx, "INVALID_DATA", msg));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(throw_coded_range_error(ctx, "INVALID_DATA", msg));
    }
    #[allow(clippy::cast_precision_loss)]
    if num > usize::MAX as f64 {
        return Err(throw_coded_range_error(ctx, "INVALID_DATA", msg));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        *target = num as usize;
    }
    Ok(())
}

fn object_class_id(ctx: &Ctx<'_>) -> Result<qjs::JSClassID> {
    let state = ctx
        .userdata::<super::neo::NeoState>()
        .ok_or(rquickjs::Error::Unknown)?;
    Ok(state.object_class_id)
}

fn js_data_to_json<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<serde_json::Value> {
    let Some(value) = value else {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    };
    if value.is_undefined() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    let obj_class_id = object_class_id(ctx)?;
    if !value.is_object()
        || value.is_proxy()
        || unsafe { qjs::JS_IsArray(value.as_raw()) }
        || unsafe { qjs::JS_GetClassID(value.as_raw()) } != obj_class_id
    {
        return Err(throw_template_error(
            ctx,
            TemplateError::InvalidData("template data must be a JSON object".to_owned()),
        ));
    }
    let mut nodes = 0usize;
    let mut ancestors = Vec::new();
    js_value_to_json(ctx, value, obj_class_id, 0, &mut nodes, &mut ancestors)
}

fn js_value_to_json<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    obj_class_id: qjs::JSClassID,
    depth: usize,
    nodes: &mut usize,
    ancestors: &mut Vec<Value<'js>>,
) -> Result<serde_json::Value> {
    if depth > MAX_DATA_DEPTH {
        return Err(throw_template_error(
            ctx,
            TemplateError::InvalidData(format!(
                "template data nesting depth exceeds maximum {MAX_DATA_DEPTH}"
            )),
        ));
    }
    if *nodes >= MAX_DATA_NODES {
        return Err(throw_template_error(
            ctx,
            TemplateError::InvalidData(format!(
                "template data exceeds maximum node count {MAX_DATA_NODES}"
            )),
        ));
    }
    *nodes += 1;

    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if let Some(b) = value.as_bool() {
        return Ok(serde_json::Value::Bool(b));
    }
    if value.is_number() {
        let num = value.as_number().ok_or_else(|| {
            throw_template_error(
                ctx,
                TemplateError::InvalidData("invalid number in template data".to_owned()),
            )
        })?;
        if !num.is_finite() {
            return Err(throw_template_error(
                ctx,
                TemplateError::InvalidData(
                    "template data must not contain NaN or Infinity".to_owned(),
                ),
            ));
        }
        if num.fract() == 0.0 && (-9_007_199_254_740_991.0..=MAX_SAFE_INTEGER_F64).contains(&num) {
            #[allow(clippy::cast_possible_truncation)]
            let int = num as i64;
            return Ok(serde_json::Value::Number(serde_json::Number::from(int)));
        }
        let number = serde_json::Number::from_f64(num).ok_or_else(|| {
            throw_template_error(
                ctx,
                TemplateError::InvalidData(
                    "invalid floating-point number in template data".to_owned(),
                ),
            )
        })?;
        return Ok(serde_json::Value::Number(number));
    }
    if value.is_string() {
        let text = read_strict_string(
            ctx,
            value,
            "INVALID_DATA",
            "template data string expected",
            TemplateError::InvalidData,
        )?;
        return Ok(serde_json::Value::String(text));
    }
    if value.is_undefined() || value.is_symbol() || value.is_big_int() || value.is_proxy() {
        return Err(throw_template_error(
            ctx,
            TemplateError::InvalidData("template data contains an unsupported value".to_owned()),
        ));
    }
    if unsafe { qjs::JS_IsArray(value.as_raw()) } {
        if ancestors.iter().any(|seen| seen == value) {
            return Err(throw_template_error(
                ctx,
                TemplateError::InvalidData(
                    "template data contains a circular reference".to_owned(),
                ),
            ));
        }
        let array = Array::from_value(value.clone()).map_err(|_| {
            throw_template_error(
                ctx,
                TemplateError::InvalidData("invalid array in template data".to_owned()),
            )
        })?;
        let len = array.len();
        if len > MAX_DATA_NODES - *nodes {
            return Err(throw_template_error(
                ctx,
                TemplateError::InvalidData(format!(
                    "template data array length {len} exceeds maximum node budget"
                )),
            ));
        }
        ancestors.push(value.clone());
        let raw = ctx.as_raw().as_ptr();
        let mut out = Vec::with_capacity(len);
        for index in 0..len {
            let atom = unsafe { qjs::JS_NewAtomUInt32(raw, index as u32) };
            if atom == qjs::JS_ATOM_NULL {
                ancestors.pop();
                return Err(rquickjs::Error::Unknown);
            }
            let mut desc = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
            let found =
                unsafe { qjs::JS_GetOwnProperty(raw, desc.as_mut_ptr(), value.as_raw(), atom) };
            unsafe { qjs::JS_FreeAtom(raw, atom) };
            if found < 0 {
                if ctx.has_exception() {
                    let _ = ctx.catch();
                }
                ancestors.pop();
                return Err(throw_template_error(
                    ctx,
                    TemplateError::InvalidData("failed to inspect array element".to_owned()),
                ));
            }
            if found == 0 {
                out.push(serde_json::Value::Null);
                continue;
            }
            let desc = unsafe { desc.assume_init() };
            let elem = unsafe { Value::from_raw(ctx.clone(), desc.value) };
            let _getter = unsafe { Value::from_raw(ctx.clone(), desc.getter) };
            let _setter = unsafe { Value::from_raw(ctx.clone(), desc.setter) };
            if (desc.flags as u32 & qjs::JS_PROP_GETSET) != 0 {
                ancestors.pop();
                return Err(throw_template_error(
                    ctx,
                    TemplateError::InvalidData(
                        "template data must not contain accessor properties".to_owned(),
                    ),
                ));
            }
            if elem.is_undefined() {
                out.push(serde_json::Value::Null);
            } else {
                let converted =
                    js_value_to_json(ctx, &elem, obj_class_id, depth + 1, nodes, ancestors)?;
                out.push(converted);
            }
        }
        ancestors.pop();
        return Ok(serde_json::Value::Array(out));
    }

    if value.is_object() && unsafe { qjs::JS_GetClassID(value.as_raw()) } == obj_class_id {
        if ancestors.iter().any(|seen| seen == value) {
            return Err(throw_template_error(
                ctx,
                TemplateError::InvalidData(
                    "template data contains a circular reference".to_owned(),
                ),
            ));
        }
        let raw = ctx.as_raw().as_ptr();
        let mut tab: *mut qjs::JSPropertyEnum = std::ptr::null_mut();
        let mut len: u32 = 0;
        let flags = (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_SYMBOL_MASK) as std::ffi::c_int;
        let rc =
            unsafe { qjs::JS_GetOwnPropertyNames(raw, &mut tab, &mut len, value.as_raw(), flags) };
        let _guard = PropertyEnumGuard { raw, tab, len };
        if rc < 0 {
            if ctx.has_exception() {
                let _ = ctx.catch();
            }
            return Err(throw_template_error(
                ctx,
                TemplateError::InvalidData("failed to inspect template data properties".to_owned()),
            ));
        }

        ancestors.push(value.clone());
        let mut map = serde_json::Map::new();
        for i in 0..len {
            let atom = unsafe { (*tab.add(i as usize)).atom };
            let atom_val = unsafe { Value::from_raw(ctx.clone(), qjs::JS_AtomToValue(raw, atom)) };
            if atom_val.is_symbol() {
                continue;
            }
            let mut desc = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
            let found =
                unsafe { qjs::JS_GetOwnProperty(raw, desc.as_mut_ptr(), value.as_raw(), atom) };
            if found < 0 {
                if ctx.has_exception() {
                    let _ = ctx.catch();
                }
                ancestors.pop();
                return Err(throw_template_error(
                    ctx,
                    TemplateError::InvalidData(
                        "failed to inspect template data property".to_owned(),
                    ),
                ));
            }
            if found == 0 {
                continue;
            }
            let desc = unsafe { desc.assume_init() };
            let field_val = unsafe { Value::from_raw(ctx.clone(), desc.value) };
            let _getter = unsafe { Value::from_raw(ctx.clone(), desc.getter) };
            let _setter = unsafe { Value::from_raw(ctx.clone(), desc.setter) };

            if (desc.flags as u32 & qjs::JS_PROP_ENUMERABLE) == 0 {
                continue;
            }
            if (desc.flags as u32 & qjs::JS_PROP_GETSET) != 0 {
                ancestors.pop();
                return Err(throw_template_error(
                    ctx,
                    TemplateError::InvalidData(
                        "template data must not contain accessor properties".to_owned(),
                    ),
                ));
            }
            if field_val.is_undefined() {
                continue;
            }
            let key = read_strict_string(
                ctx,
                &atom_val,
                "INVALID_DATA",
                "template data key must be a string",
                TemplateError::InvalidData,
            )
            .inspect_err(|_| {
                ancestors.pop();
            })?;
            let converted =
                js_value_to_json(ctx, &field_val, obj_class_id, depth + 1, nodes, ancestors)?;
            map.insert(key, converted);
        }
        ancestors.pop();
        return Ok(serde_json::Value::Object(map));
    }

    Err(throw_template_error(
        ctx,
        TemplateError::InvalidData("template data contains an unsupported value".to_owned()),
    ))
}

fn render<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    let source = match args.0.first() {
        Some(v) => read_strict_string(
            &ctx,
            v,
            "INVALID_DATA",
            "source must be a string",
            TemplateError::InvalidData,
        )?,
        None => {
            return Err(throw_coded_type_error(
                &ctx,
                "INVALID_DATA",
                "source must be a string",
            ));
        }
    };
    let options = parse_template_options(&ctx, args.0.get(2))?;
    let data = js_data_to_json(&ctx, args.0.get(1))?;
    neojs_template_core::render(&source, &data, &options)
        .map_err(|err| throw_template_error(&ctx, err))
}

fn compile<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let source = match args.0.first() {
        Some(v) => read_strict_string(
            &ctx,
            v,
            "INVALID_DATA",
            "source must be a string",
            TemplateError::InvalidData,
        )?,
        None => {
            return Err(throw_coded_type_error(
                &ctx,
                "INVALID_DATA",
                "source must be a string",
            ));
        }
    };
    let options = parse_template_options(&ctx, args.0.get(1))?;
    let compiled = CompiledTemplate::compile(&source, &options)
        .map_err(|err| throw_template_error(&ctx, err))?;

    let proto = {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        state
            .template_proto
            .as_ref()
            .ok_or(rquickjs::Error::Unknown)?
            .clone()
            .restore(&ctx)?
    };
    let cls = Class::instance_proto(TemplateBrand(compiled), proto)?;
    let render_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<String> {
            let cls = this
                .0
                .as_object()
                .and_then(Class::<TemplateBrand>::from_object)
                .ok_or_else(|| {
                    throw_coded_type_error(&ctx, "INVALID_DATA", "invalid Template receiver")
                })?;
            let data = js_data_to_json(&ctx, args.0.first())?;
            let brand = cls.borrow();
            brand
                .0
                .render(&data)
                .map_err(|err| throw_template_error(&ctx, err))
        },
    )?
    .with_name("render")?
    .with_length(1)?;
    cls.prop("render", Property::from(render_fn).configurable())?;
    freeze_native(&ctx, &cls)?;
    Ok(cls.into_inner())
}

fn create_template_set<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let options = parse_template_options(&ctx, args.0.first())?;
    let set = TemplateSet::new(options);

    let proto = {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        state
            .template_set_proto
            .as_ref()
            .ok_or(rquickjs::Error::Unknown)?
            .clone()
            .restore(&ctx)?
    };
    let cls = Class::instance_proto(TemplateSetBrand(RefCell::new(set)), proto)?;

    let add_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
            let cls = this
                .0
                .as_object()
                .and_then(Class::<TemplateSetBrand>::from_object)
                .ok_or_else(|| {
                    throw_coded_type_error(&ctx, "INVALID_DATA", "invalid TemplateSet receiver")
                })?;
            let name = match args.0.first() {
                Some(v) => read_strict_string(
                    &ctx,
                    v,
                    "INVALID_NAME",
                    "template name must be a string",
                    TemplateError::InvalidName,
                )?,
                None => {
                    return Err(throw_coded_type_error(
                        &ctx,
                        "INVALID_NAME",
                        "template name must be a string",
                    ));
                }
            };
            let source = match args.0.get(1) {
                Some(v) => read_strict_string(
                    &ctx,
                    v,
                    "INVALID_DATA",
                    "template source must be a string",
                    TemplateError::InvalidData,
                )?,
                None => {
                    return Err(throw_coded_type_error(
                        &ctx,
                        "INVALID_DATA",
                        "template source must be a string",
                    ));
                }
            };
            cls.borrow()
                .0
                .borrow_mut()
                .add(&name, &source)
                .map_err(|err| throw_template_error(&ctx, err))?;
            Ok(this.0)
        },
    )?
    .with_name("add")?
    .with_length(2)?;

    let render_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<String> {
            let cls = this
                .0
                .as_object()
                .and_then(Class::<TemplateSetBrand>::from_object)
                .ok_or_else(|| {
                    throw_coded_type_error(&ctx, "INVALID_DATA", "invalid TemplateSet receiver")
                })?;
            let name = match args.0.first() {
                Some(v) => read_strict_string(
                    &ctx,
                    v,
                    "INVALID_NAME",
                    "template name must be a string",
                    TemplateError::InvalidName,
                )?,
                None => {
                    return Err(throw_coded_type_error(
                        &ctx,
                        "INVALID_NAME",
                        "template name must be a string",
                    ));
                }
            };
            let data = js_data_to_json(&ctx, args.0.get(1))?;
            let brand = cls.borrow();
            let set_ref = brand.0.borrow();
            set_ref
                .render(&name, &data)
                .map_err(|err| throw_template_error(&ctx, err))
        },
    )?
    .with_name("render")?
    .with_length(2)?;

    let names_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>| -> Result<Array<'js>> {
            let cls = this
                .0
                .as_object()
                .and_then(Class::<TemplateSetBrand>::from_object)
                .ok_or_else(|| {
                    throw_coded_type_error(&ctx, "INVALID_DATA", "invalid TemplateSet receiver")
                })?;
            let names = cls.borrow().0.borrow().names();
            let array = Array::new(ctx.clone())?;
            for (index, name) in names.iter().enumerate() {
                array.set(index, name.as_str())?;
            }
            Ok(array)
        },
    )?
    .with_name("names")?
    .with_length(0)?;

    cls.prop("add", Property::from(add_fn).configurable())?;
    cls.prop("render", Property::from(render_fn).configurable())?;
    cls.prop("names", Property::from(names_fn).configurable())?;
    freeze_native(&ctx, &cls)?;
    Ok(cls.into_inner())
}

fn escape_html<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    let input = match args.0.first() {
        Some(v) => read_strict_string(
            &ctx,
            v,
            "INVALID_DATA",
            "input must be a string",
            TemplateError::InvalidData,
        )?,
        None => {
            return Err(throw_coded_type_error(
                &ctx,
                "INVALID_DATA",
                "input must be a string",
            ));
        }
    };
    Ok(neojs_template_core::escape_html(&input))
}

/// The native QuickJS module served for `neo:template`.
pub(super) struct TemplateModule;

impl ModuleDef for TemplateModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in EXPORTS {
            declarations.declare(*name)?;
        }
        declarations.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(template) = state.template.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = template.clone().restore(ctx)?;
        for name in EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        exports.export("default", object)?;
        Ok(())
    }
}
