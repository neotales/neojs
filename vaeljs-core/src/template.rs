//! V8/`deno_core` op adapter for `vael:template`.
//!
//! All Go `text/template` parsing, Sprig helper execution, HTML escaping,
//! layout/partial composition, and resource limits live in the engine-neutral
//! [`vaeljs_template_core`] crate. This module validates V8 arguments, converts
//! plain options and bounded JSON data without running user getters or Proxy
//! traps, stores compiled templates and template sets in `ResourceTable` under a
//! per-runtime handle cap, and returns null-prototype outcome objects to
//! [`crate::builtins`].

use std::borrow::Cow;
use std::cell::RefCell;

use deno_core::{OpState, Resource, ResourceId, op2, v8};
use vaeljs_template_core::{CompiledTemplate, TemplateError, TemplateOptions, TemplateSet};

const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
const MAX_DATA_DEPTH: usize = 128;
const MAX_DATA_NODES: usize = 100_000;
const MAX_TEMPLATE_HANDLES: usize = 4_096;

#[derive(Debug)]
enum BindingError {
    Type(&'static str, &'static str),
    Range(&'static str, &'static str),
    Template(TemplateError),
}

impl From<TemplateError> for BindingError {
    fn from(err: TemplateError) -> Self {
        Self::Template(err)
    }
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

#[derive(Default)]
struct TemplateHandleCount(usize);

struct DisallowedPrototypes {
    protos: Vec<v8::Global<v8::Value>>,
}

struct CompiledTemplateResource {
    template: CompiledTemplate,
}

impl Resource for CompiledTemplateResource {
    fn name(&self) -> Cow<'_, str> {
        "vael::template::Template".into()
    }
}

struct TemplateSetResource {
    set: RefCell<TemplateSet>,
}

impl Resource for TemplateSetResource {
    fn name(&self) -> Cow<'_, str> {
        "vael::template::TemplateSet".into()
    }
}

fn acquire_handle_slot(state: &mut OpState) -> Result<(), BindingError> {
    let count = if let Some(counter) = state.try_borrow::<TemplateHandleCount>() {
        counter.0
    } else {
        state.put(TemplateHandleCount::default());
        0
    };
    if count >= MAX_TEMPLATE_HANDLES {
        return Err(BindingError::Template(TemplateError::LimitExceeded(
            format!("open template handle count exceeds limit {MAX_TEMPLATE_HANDLES}"),
        )));
    }
    state.borrow_mut::<TemplateHandleCount>().0 = count + 1;
    Ok(())
}

fn release_handle_slot(state: &mut OpState) {
    if let Some(counter) = state.try_borrow_mut::<TemplateHandleCount>() {
        counter.0 = counter.0.saturating_sub(1);
    }
}

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
    error: BindingError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    match error {
        BindingError::Type(code, message) => {
            set_string(scope, object, "name", "TypeError");
            set_string(scope, object, "code", code);
            set_string(scope, object, "message", message);
        }
        BindingError::Range(code, message) => {
            set_string(scope, object, "name", "RangeError");
            set_string(scope, object, "code", code);
            set_string(scope, object, "message", message);
        }
        BindingError::Template(err) => {
            set_string(scope, object, "name", "TemplateError");
            set_string(scope, object, "code", template_error_code(&err));
            set_string(scope, object, "message", &err.to_string());
        }
    }
}

fn outcome_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<v8::Local<'s, v8::Value>, BindingError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(value) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(scope, object, "value", value);
        }
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn outcome_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<(), BindingError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(()) => set(scope, object, "ok", v8::Boolean::new(scope, true).into()),
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn read_strict_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    code: &'static str,
    type_msg: &'static str,
    surrogate_err: fn(String) -> TemplateError,
) -> Result<String, BindingError> {
    if !value.is_string() {
        return Err(BindingError::Type(code, type_msg));
    }
    let string =
        v8::Local::<v8::String>::try_from(value).map_err(|_| BindingError::Type(code, type_msg))?;
    let length = string.utf8_length(scope);
    let capacity = length
        .checked_add(4)
        .ok_or_else(|| TemplateError::LimitExceeded("string length overflow".to_owned()))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| TemplateError::LimitExceeded("failed to allocate string buffer".to_owned()))?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    String::from_utf8(buffer).map_err(|_| {
        BindingError::Template(surrogate_err(
            "string contains an unpaired UTF-16 surrogate".to_owned(),
        ))
    })
}

fn v8_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &str,
) -> Result<v8::Local<'s, v8::Value>, BindingError> {
    v8::String::new(scope, text).map(Into::into).ok_or_else(|| {
        BindingError::Template(TemplateError::Render(
            "failed to allocate JS string".to_owned(),
        ))
    })
}

fn plain_object_prototype<'s>(
    scope: &v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Value>, BindingError> {
    let object = v8::Object::new(scope);
    object.get_prototype(scope).ok_or(BindingError::Type(
        "INVALID_DATA",
        "options must be a plain object",
    ))
}

fn own_descriptor_field<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, BindingError> {
    let Some(key) = v8::String::new(scope, name) else {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "invalid property descriptor",
        ));
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    match object.has_own_property(scope, key) {
        Some(true) => object
            .get(scope, key.into())
            .map(Some)
            .ok_or(BindingError::Type(
                "INVALID_DATA",
                "invalid property descriptor",
            )),
        Some(false) => Ok(None),
        None => Err(BindingError::Type(
            "INVALID_DATA",
            "invalid property descriptor",
        )),
    }
}

fn own_descriptor_boolean(
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Result<bool, BindingError> {
    let Some(value) = own_descriptor_field(scope, object, name)? else {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "invalid property descriptor",
        ));
    };
    if !value.is_boolean() {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "invalid property descriptor",
        ));
    }
    Ok(value.boolean_value(scope))
}

fn parse_options(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<TemplateOptions, BindingError> {
    let mut opts = TemplateOptions::default();
    if value.is_undefined() {
        return Ok(opts);
    }
    if !value.is_object() || value.is_proxy() || value.is_array() || value.is_function() {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "options must be a plain object",
        ));
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| BindingError::Type("INVALID_DATA", "options must be a plain object"))?;
    let Some(prototype) = object.get_prototype(scope) else {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "options must be a plain object",
        ));
    };
    if !prototype.is_null() && prototype != plain_object_prototype(scope)? {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "options must be a plain object",
        ));
    }

    let args = v8::GetPropertyNamesArgs {
        mode: v8::KeyCollectionMode::OwnOnly,
        property_filter: v8::PropertyFilter::ALL_PROPERTIES,
        index_filter: v8::IndexFilter::IncludeIndices,
        key_conversion: v8::KeyConversionMode::ConvertToString,
    };
    let Some(names) = object.get_own_property_names(scope, args) else {
        return Err(BindingError::Type(
            "INVALID_DATA",
            "options must be a plain object",
        ));
    };

    for index in 0..names.length() {
        let Some(name_value) = names.get_index(scope, index) else {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must be a plain object",
            ));
        };
        if name_value.is_symbol() {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must not contain symbol properties",
            ));
        }
        let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must be a plain object",
            ));
        };
        let Some(descriptor_value) = object.get_own_property_descriptor(scope, name) else {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must be a plain object",
            ));
        };
        let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor_value) else {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must be a plain object",
            ));
        };
        if !own_descriptor_boolean(scope, descriptor, "enumerable")? {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must not contain non-enumerable properties",
            ));
        }
        if own_descriptor_field(scope, descriptor, "get")?.is_some()
            || own_descriptor_field(scope, descriptor, "set")?.is_some()
        {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must not contain accessor properties",
            ));
        }
        let Ok(key_str) = v8::Local::<v8::String>::try_from(name_value) else {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must be a plain object",
            ));
        };
        let key = key_str.to_rust_string_lossy(scope);
        let Some(field) = own_descriptor_field(scope, descriptor, "value")? else {
            return Err(BindingError::Type(
                "INVALID_DATA",
                "options must be a plain object",
            ));
        };
        match key.as_str() {
            "sprig" => opt_bool(
                scope,
                field,
                &mut opts.sprig,
                "options.sprig must be a boolean",
            )?,
            "hermetic" => opt_bool(
                scope,
                field,
                &mut opts.hermetic,
                "options.hermetic must be a boolean",
            )?,
            "htmlEscape" => opt_bool(
                scope,
                field,
                &mut opts.html_escape,
                "options.htmlEscape must be a boolean",
            )?,
            "maxSourceLength" => opt_usize(
                scope,
                field,
                &mut opts.max_source_length,
                "options.maxSourceLength must be a non-negative safe integer",
            )?,
            "maxNestingDepth" => opt_usize(
                scope,
                field,
                &mut opts.max_nesting_depth,
                "options.maxNestingDepth must be a non-negative safe integer",
            )?,
            "maxOutputLength" => opt_usize(
                scope,
                field,
                &mut opts.max_output_length,
                "options.maxOutputLength must be a non-negative safe integer",
            )?,
            "maxIterations" => opt_usize(
                scope,
                field,
                &mut opts.max_iterations,
                "options.maxIterations must be a non-negative safe integer",
            )?,
            "maxFunctionCalls" => opt_usize(
                scope,
                field,
                &mut opts.max_function_calls,
                "options.maxFunctionCalls must be a non-negative safe integer",
            )?,
            _ => {
                return Err(BindingError::Type(
                    "INVALID_DATA",
                    "unknown template option",
                ));
            }
        }
    }

    Ok(opts)
}

fn opt_bool(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    target: &mut bool,
    msg: &'static str,
) -> Result<(), BindingError> {
    if value.is_undefined() {
        return Ok(());
    }
    if !value.is_boolean() {
        return Err(BindingError::Type("INVALID_DATA", msg));
    }
    *target = value.boolean_value(scope);
    Ok(())
}

fn opt_usize(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    target: &mut usize,
    msg: &'static str,
) -> Result<(), BindingError> {
    if value.is_undefined() {
        return Ok(());
    }
    if !value.is_number() {
        return Err(BindingError::Type("INVALID_DATA", msg));
    }
    let Some(num) = value.number_value(scope) else {
        return Err(BindingError::Type("INVALID_DATA", msg));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(BindingError::Range("INVALID_DATA", msg));
    }
    #[allow(clippy::cast_precision_loss)]
    if num > usize::MAX as f64 {
        return Err(BindingError::Range("INVALID_DATA", msg));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        *target = num as usize;
    }
    Ok(())
}

fn is_disallowed_data_object(
    scope: &v8::PinScope<'_, '_>,
    state: &OpState,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    if value.is_function()
        || value.is_proxy()
        || value.is_map()
        || value.is_set()
        || value.is_weak_map()
        || value.is_weak_set()
        || value.is_arguments_object()
        || value.is_generator_object()
        || value.is_module_namespace_object()
        || value.is_map_iterator()
        || value.is_set_iterator()
        || value.is_wasm_memory_object()
        || value.is_wasm_module_object()
        || value.is_external()
        || value.is_date()
        || value.is_reg_exp()
        || value.is_promise()
        || value.is_native_error()
        || value.is_array_buffer()
        || value.is_shared_array_buffer()
        || value.is_array_buffer_view()
        || value.is_symbol_object()
        || value.is_big_int_object()
        || value.is_number_object()
        || value.is_string_object()
        || value.is_boolean_object()
    {
        return true;
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(proto) = object.get_prototype(scope)
        && let Some(disallowed) = state.try_borrow::<DisallowedPrototypes>()
    {
        for blocked in &disallowed.protos {
            let blocked_local = v8::Local::new(scope, blocked);
            if proto == blocked_local {
                return true;
            }
        }
    }
    false
}

fn js_data_to_json<'s>(
    scope: &v8::PinScope<'s, '_>,
    state: &OpState,
    value: v8::Local<'s, v8::Value>,
) -> Result<serde_json::Value, BindingError> {
    if value.is_undefined() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    if !value.is_object() || value.is_array() || is_disallowed_data_object(scope, state, value) {
        return Err(BindingError::Template(TemplateError::InvalidData(
            "template data must be a JSON object".to_owned(),
        )));
    }
    let mut nodes = 0usize;
    let mut ancestors = Vec::new();
    js_value_to_json(scope, state, value, 0, &mut nodes, &mut ancestors)
}

fn js_value_to_json<'s>(
    scope: &v8::PinScope<'s, '_>,
    state: &OpState,
    value: v8::Local<'s, v8::Value>,
    depth: usize,
    nodes: &mut usize,
    ancestors: &mut Vec<v8::Local<'s, v8::Object>>,
) -> Result<serde_json::Value, BindingError> {
    if depth > MAX_DATA_DEPTH {
        return Err(BindingError::Template(TemplateError::InvalidData(format!(
            "template data nesting depth exceeds maximum {MAX_DATA_DEPTH}"
        ))));
    }
    if *nodes >= MAX_DATA_NODES {
        return Err(BindingError::Template(TemplateError::InvalidData(format!(
            "template data exceeds maximum node count {MAX_DATA_NODES}"
        ))));
    }
    *nodes += 1;

    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if value.is_boolean() {
        return Ok(serde_json::Value::Bool(value.boolean_value(scope)));
    }
    if value.is_number() {
        let Some(num) = value.number_value(scope) else {
            return Err(BindingError::Template(TemplateError::InvalidData(
                "invalid number in template data".to_owned(),
            )));
        };
        if !num.is_finite() {
            return Err(BindingError::Template(TemplateError::InvalidData(
                "template data must not contain NaN or Infinity".to_owned(),
            )));
        }
        if num.fract() == 0.0 && (-9_007_199_254_740_991.0..=MAX_SAFE_INTEGER_F64).contains(&num) {
            #[allow(clippy::cast_possible_truncation)]
            let int = num as i64;
            return Ok(serde_json::Value::Number(serde_json::Number::from(int)));
        }
        let number = serde_json::Number::from_f64(num).ok_or_else(|| {
            BindingError::Template(TemplateError::InvalidData(
                "invalid floating-point number in template data".to_owned(),
            ))
        })?;
        return Ok(serde_json::Value::Number(number));
    }
    if value.is_string() {
        let text = read_strict_string(
            scope,
            value,
            "INVALID_DATA",
            "template data string expected",
            TemplateError::InvalidData,
        )?;
        return Ok(serde_json::Value::String(text));
    }
    if value.is_undefined()
        || value.is_symbol()
        || value.is_big_int()
        || is_disallowed_data_object(scope, state, value)
    {
        return Err(BindingError::Template(TemplateError::InvalidData(
            "template data contains an unsupported value".to_owned(),
        )));
    }
    if value.is_array() {
        let array = v8::Local::<v8::Array>::try_from(value).map_err(|_| {
            BindingError::Template(TemplateError::InvalidData(
                "invalid array in template data".to_owned(),
            ))
        })?;
        let obj: v8::Local<'s, v8::Object> = array.into();
        if ancestors.contains(&obj) {
            return Err(BindingError::Template(TemplateError::InvalidData(
                "template data contains a circular reference".to_owned(),
            )));
        }
        let len = array.length() as usize;
        if len > MAX_DATA_NODES - *nodes {
            return Err(BindingError::Template(TemplateError::InvalidData(format!(
                "template data array length {len} exceeds maximum node budget"
            ))));
        }
        ancestors.push(obj);
        let mut out = Vec::with_capacity(len);
        for index in 0..array.length() {
            let Some(key_str) = v8::String::new(scope, &index.to_string()) else {
                ancestors.pop();
                return Err(BindingError::Template(TemplateError::InvalidData(
                    "failed to index array".to_owned(),
                )));
            };
            let key_name: v8::Local<'s, v8::Name> = key_str.into();
            match obj.get_own_property_descriptor(scope, key_name) {
                Some(desc_val) if desc_val.is_object() => {
                    let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
                        ancestors.pop();
                        return Err(BindingError::Template(TemplateError::InvalidData(
                            "invalid array element descriptor".to_owned(),
                        )));
                    };
                    if own_descriptor_field(scope, desc, "get")?.is_some()
                        || own_descriptor_field(scope, desc, "set")?.is_some()
                    {
                        ancestors.pop();
                        return Err(BindingError::Template(TemplateError::InvalidData(
                            "template data must not contain accessor properties".to_owned(),
                        )));
                    }
                    match own_descriptor_field(scope, desc, "value")? {
                        Some(elem) if !elem.is_undefined() => {
                            let converted =
                                js_value_to_json(scope, state, elem, depth + 1, nodes, ancestors)?;
                            out.push(converted);
                        }
                        _ => {
                            if *nodes >= MAX_DATA_NODES {
                                ancestors.pop();
                                return Err(BindingError::Template(TemplateError::InvalidData(
                                    format!(
                                        "template data exceeds maximum node count {MAX_DATA_NODES}"
                                    ),
                                )));
                            }
                            *nodes += 1;
                            out.push(serde_json::Value::Null);
                        }
                    }
                }
                _ => {
                    if *nodes >= MAX_DATA_NODES {
                        ancestors.pop();
                        return Err(BindingError::Template(TemplateError::InvalidData(format!(
                            "template data exceeds maximum node count {MAX_DATA_NODES}"
                        ))));
                    }
                    *nodes += 1;
                    out.push(serde_json::Value::Null);
                }
            }
        }
        ancestors.pop();
        return Ok(serde_json::Value::Array(out));
    }

    if value.is_object() {
        let obj = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
            BindingError::Template(TemplateError::InvalidData(
                "invalid object in template data".to_owned(),
            ))
        })?;
        if ancestors.contains(&obj) {
            return Err(BindingError::Template(TemplateError::InvalidData(
                "template data contains a circular reference".to_owned(),
            )));
        }
        let args = v8::GetPropertyNamesArgs {
            mode: v8::KeyCollectionMode::OwnOnly,
            property_filter: v8::PropertyFilter::ALL_PROPERTIES,
            index_filter: v8::IndexFilter::IncludeIndices,
            key_conversion: v8::KeyConversionMode::ConvertToString,
        };
        let Some(names) = obj.get_own_property_names(scope, args) else {
            return Err(BindingError::Template(TemplateError::InvalidData(
                "failed to inspect template data properties".to_owned(),
            )));
        };
        ancestors.push(obj);
        let mut map = serde_json::Map::new();
        for index in 0..names.length() {
            let Some(name_value) = names.get_index(scope, index) else {
                ancestors.pop();
                return Err(BindingError::Template(TemplateError::InvalidData(
                    "failed to read template data property name".to_owned(),
                )));
            };
            if name_value.is_symbol() {
                continue;
            }
            let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
                continue;
            };
            let Some(desc_val) = obj.get_own_property_descriptor(scope, name) else {
                continue;
            };
            let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
                ancestors.pop();
                return Err(BindingError::Template(TemplateError::InvalidData(
                    "invalid property descriptor in template data".to_owned(),
                )));
            };
            if !own_descriptor_boolean(scope, desc, "enumerable")? {
                continue;
            }
            if own_descriptor_field(scope, desc, "get")?.is_some()
                || own_descriptor_field(scope, desc, "set")?.is_some()
            {
                ancestors.pop();
                return Err(BindingError::Template(TemplateError::InvalidData(
                    "template data must not contain accessor properties".to_owned(),
                )));
            }
            let Some(field) = own_descriptor_field(scope, desc, "value")? else {
                continue;
            };
            if field.is_undefined() {
                continue;
            }
            let key = read_strict_string(
                scope,
                name_value,
                "INVALID_DATA",
                "template data key must be a string",
                TemplateError::InvalidData,
            )?;
            let converted = js_value_to_json(scope, state, field, depth + 1, nodes, ancestors)?;
            map.insert(key, converted);
        }
        ancestors.pop();
        return Ok(serde_json::Value::Object(map));
    }

    Err(BindingError::Template(TemplateError::InvalidData(
        "template data contains an unsupported value".to_owned(),
    )))
}

#[op2(fast)]
fn op_vael_template_register_disallowed_protos(
    scope: &mut v8::PinScope<'_, '_>,
    state: &mut OpState,
    protos: v8::Local<'_, v8::Value>,
) {
    if state.try_borrow::<DisallowedPrototypes>().is_some() {
        return;
    }
    let Ok(array) = v8::Local::<v8::Array>::try_from(protos) else {
        return;
    };
    let mut globals = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        if let Some(item) = array.get_index(scope, index) {
            globals.push(v8::Global::new(scope, item));
        }
    }
    state.put(DisallowedPrototypes { protos: globals });
}

#[op2]
fn op_vael_template_render<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &OpState,
    source: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let source = read_strict_string(
            scope,
            source,
            "INVALID_DATA",
            "source must be a string",
            TemplateError::InvalidData,
        )?;
        let options = parse_options(scope, options)?;
        let data = js_data_to_json(scope, state, data)?;
        let rendered = vaeljs_template_core::render(&source, &data, &options)?;
        v8_string(scope, &rendered)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_template_compile<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    source: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let source = read_strict_string(
            scope,
            source,
            "INVALID_DATA",
            "source must be a string",
            TemplateError::InvalidData,
        )?;
        let options = parse_options(scope, options)?;
        let compiled = CompiledTemplate::compile(&source, &options)?;
        acquire_handle_slot(state)?;
        let rid = state
            .resource_table
            .add(CompiledTemplateResource { template: compiled });
        Ok(v8::Integer::new_from_unsigned(scope, rid).into())
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_template_compiled_render<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    data: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<CompiledTemplateResource>(id)
            .map_err(|_| {
                BindingError::Template(TemplateError::Render(
                    "invalid compiled template handle".to_owned(),
                ))
            })?;
        let data = js_data_to_json(scope, state, data)?;
        let rendered = resource.template.render(&data)?;
        v8_string(scope, &rendered)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_template_set_create<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let options = parse_options(scope, options)?;
        let set = TemplateSet::new(options);
        acquire_handle_slot(state)?;
        let rid = state.resource_table.add(TemplateSetResource {
            set: RefCell::new(set),
        });
        Ok(v8::Integer::new_from_unsigned(scope, rid).into())
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_template_set_add<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    name: v8::Local<'s, v8::Value>,
    source: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<TemplateSetResource>(id)
            .map_err(|_| {
                BindingError::Template(TemplateError::Render(
                    "invalid template set handle".to_owned(),
                ))
            })?;
        let name = read_strict_string(
            scope,
            name,
            "INVALID_NAME",
            "template name must be a string",
            TemplateError::InvalidName,
        )?;
        let source = read_strict_string(
            scope,
            source,
            "INVALID_DATA",
            "template source must be a string",
            TemplateError::InvalidData,
        )?;
        resource.set.borrow_mut().add(&name, &source)?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_template_set_render<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    name: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<TemplateSetResource>(id)
            .map_err(|_| {
                BindingError::Template(TemplateError::Render(
                    "invalid template set handle".to_owned(),
                ))
            })?;
        let name = read_strict_string(
            scope,
            name,
            "INVALID_NAME",
            "template name must be a string",
            TemplateError::InvalidName,
        )?;
        let data = js_data_to_json(scope, state, data)?;
        let rendered = resource.set.borrow().render(&name, &data)?;
        v8_string(scope, &rendered)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_template_set_names<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<TemplateSetResource>(id)
            .map_err(|_| {
                BindingError::Template(TemplateError::Render(
                    "invalid template set handle".to_owned(),
                ))
            })?;
        let names = resource.set.borrow().names();
        let mut elements = Vec::with_capacity(names.len());
        for name in &names {
            elements.push(v8_string(scope, name)?);
        }
        Ok(v8::Array::new_with_elements(scope, &elements).into())
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_template_escape_html<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    input: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let input = read_strict_string(
            scope,
            input,
            "INVALID_DATA",
            "input must be a string",
            TemplateError::InvalidData,
        )?;
        let escaped = vaeljs_template_core::escape_html(&input);
        v8_string(scope, &escaped)
    })();
    outcome_value(scope, result)
}

#[op2(fast)]
fn op_vael_template_release(state: &mut OpState, #[smi] id: ResourceId) {
    if state
        .resource_table
        .take::<CompiledTemplateResource>(id)
        .is_ok()
    {
        release_handle_slot(state);
    }
}

#[op2(fast)]
fn op_vael_template_set_release(state: &mut OpState, #[smi] id: ResourceId) {
    if state.resource_table.take::<TemplateSetResource>(id).is_ok() {
        release_handle_slot(state);
    }
}

deno_core::extension!(
    vaeljs_template,
    ops = [
        op_vael_template_register_disallowed_protos,
        op_vael_template_render,
        op_vael_template_compile,
        op_vael_template_compiled_render,
        op_vael_template_set_create,
        op_vael_template_set_add,
        op_vael_template_set_render,
        op_vael_template_set_names,
        op_vael_template_escape_html,
        op_vael_template_release,
        op_vael_template_set_release,
    ],
    docs =
        "Go text/template and Sprig operations behind `Vael.getBuiltInModule(\"vael:template\")`."
);
