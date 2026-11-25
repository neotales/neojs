//! Value conversion between `serde_json`, `neotales_gotmpl`, and `neotales_sprig`.

use std::collections::BTreeMap;

use neotales_gotmpl::{Error as GotmplError, FuncMap, Value as GotmplValue};
use neotales_sprig::Value as SprigValue;

use crate::{TemplateError, TemplateOptions, html};

/// Converts a [`serde_json::Value`] into a [`neotales_gotmpl::Value`].
///
/// JSON integers (`i64` / `u64`) map to [`neotales_gotmpl::Value::Number`],
/// floating-point JSON numbers map to [`neotales_gotmpl::Value::Float`],
/// preserving Go `text/template` and Sprig `kindOf` semantics. Arrays map to
/// [`neotales_gotmpl::Value::List`] and objects map to
/// [`neotales_gotmpl::Value::Map`].
///
/// ```
/// use vaeljs_template_core::json_to_template_value;
/// use neotales_gotmpl::Value;
/// use serde_json::json;
///
/// assert_eq!(json_to_template_value(&json!(null)), Value::Null);
/// assert_eq!(json_to_template_value(&json!(true)), Value::Bool(true));
/// assert_eq!(json_to_template_value(&json!(42)), Value::Number(42.0));
/// assert_eq!(json_to_template_value(&json!(3.5)), Value::Float(3.5));
/// assert_eq!(
///     json_to_template_value(&json!("hi")),
///     Value::String("hi".to_owned())
/// );
/// ```
#[must_use]
pub fn json_to_template_value(value: &serde_json::Value) -> GotmplValue {
    match value {
        serde_json::Value::Null => GotmplValue::Null,
        serde_json::Value::Bool(b) => GotmplValue::Bool(*b),
        serde_json::Value::Number(n) =>
        {
            #[allow(clippy::cast_precision_loss)]
            if let Some(i) = n.as_i64() {
                GotmplValue::Number(i as f64)
            } else if let Some(u) = n.as_u64() {
                GotmplValue::Number(u as f64)
            } else if let Some(f) = n.as_f64() {
                GotmplValue::Float(f)
            } else {
                GotmplValue::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => GotmplValue::String(s.clone()),
        serde_json::Value::Array(items) => {
            GotmplValue::List(items.iter().map(json_to_template_value).collect())
        }
        serde_json::Value::Object(entries) => GotmplValue::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), json_to_template_value(v)))
                .collect(),
        ),
    }
}

/// Converts a root JSON object into a template context map.
///
/// Returns [`TemplateError::InvalidData`] if `data` is not a JSON object.
pub(crate) fn json_object_to_map(
    data: &serde_json::Value,
) -> Result<BTreeMap<String, GotmplValue>, TemplateError> {
    let serde_json::Value::Object(entries) = data else {
        return Err(TemplateError::InvalidData(
            "template data must be a JSON object".to_owned(),
        ));
    };
    Ok(entries
        .iter()
        .map(|(k, v)| (k.clone(), json_to_template_value(v)))
        .collect())
}

/// Dispatches a `neotales-sprig` helper from [`neotales_gotmpl`].
///
/// Honors [`neotales_sprig::takes_rendered_strings`] so text-oriented helpers
/// receive rendered string arguments while collection, numeric, and inspection
/// helpers receive typed values.
///
/// # Errors
///
/// Returns a [`neotales_gotmpl::Error`] if `neotales_sprig::call` rejects the
/// helper name, argument count, or argument values.
///
/// ```
/// use vaeljs_template_core::sprig_call;
/// use neotales_gotmpl::Value;
///
/// let out = sprig_call("upper", &[Value::String("hello".to_owned())]).unwrap();
/// assert_eq!(out, Value::String("HELLO".to_owned()));
/// ```
pub fn sprig_call(name: &str, arguments: &[GotmplValue]) -> Result<GotmplValue, GotmplError> {
    let sprig_args: Vec<SprigValue> = if neotales_sprig::takes_rendered_strings(name) {
        arguments
            .iter()
            .map(|arg| SprigValue::String(arg.render()))
            .collect()
    } else {
        arguments.iter().map(gotmpl_to_sprig_value).collect()
    };

    neotales_sprig::call(name, &sprig_args)
        .map(sprig_to_gotmpl_value)
        .map_err(|err| GotmplError::from(err.to_string()))
}

fn gotmpl_to_sprig_value(value: &GotmplValue) -> SprigValue {
    match value {
        GotmplValue::String(s) => SprigValue::String(s.clone()),
        GotmplValue::Bool(b) => SprigValue::Bool(*b),
        GotmplValue::Number(n) => SprigValue::Number(*n),
        GotmplValue::Float(f) => SprigValue::Float(*f),
        GotmplValue::Null => SprigValue::Null,
        GotmplValue::List(items) => {
            SprigValue::List(items.iter().map(gotmpl_to_sprig_value).collect())
        }
        GotmplValue::Map(entries) => SprigValue::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), gotmpl_to_sprig_value(v)))
                .collect(),
        ),
        GotmplValue::Object { display, members } => SprigValue::Object {
            display: display.clone(),
            members: members
                .iter()
                .map(|(k, v)| (k.clone(), gotmpl_to_sprig_value(v)))
                .collect(),
        },
        GotmplValue::Method { helper, receiver } => SprigValue::Method {
            helper: helper.clone(),
            receiver: Box::new(gotmpl_to_sprig_value(receiver)),
        },
    }
}

fn sprig_to_gotmpl_value(value: SprigValue) -> GotmplValue {
    match value {
        SprigValue::String(s) => GotmplValue::String(s),
        SprigValue::Bool(b) => GotmplValue::Bool(b),
        SprigValue::Number(n) => GotmplValue::Number(n),
        SprigValue::Float(f) => GotmplValue::Float(f),
        SprigValue::Null => GotmplValue::Null,
        SprigValue::List(items) => {
            GotmplValue::List(items.into_iter().map(sprig_to_gotmpl_value).collect())
        }
        SprigValue::Map(entries) => GotmplValue::Map(
            entries
                .into_iter()
                .map(|(k, v)| (k, sprig_to_gotmpl_value(v)))
                .collect(),
        ),
        SprigValue::Object { display, members } => GotmplValue::Object {
            display,
            members: members
                .into_iter()
                .map(|(k, v)| (k, sprig_to_gotmpl_value(v)))
                .collect(),
        },
        SprigValue::Method { helper, receiver } => GotmplValue::Method {
            helper,
            receiver: Box::new(sprig_to_gotmpl_value(*receiver)),
        },
    }
}

/// Builds the [`FuncMap`] corresponding to `options`.
pub(crate) fn build_func_map(options: &TemplateOptions) -> FuncMap {
    let mut functions = FuncMap::new();
    if options.sprig {
        if options.hermetic {
            let names = neotales_sprig::hermetic_names();
            functions
                .register_group("sprig-hermetic", &names, sprig_call)
                .expect("group name is non-empty");
        } else {
            let names = neotales_sprig::helper_names();
            functions
                .register_group("sprig", &names, sprig_call)
                .expect("group name is non-empty");
        }
    }
    html::register_html_helpers(&mut functions, options.html_escape);
    functions
}
