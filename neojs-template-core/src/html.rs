//! HTML escaping primitives and `safeHtml` marker handling.

use std::collections::BTreeMap;

use neotales_gotmpl::{Error as GotmplError, FuncMap, Value};

/// Internal marker key stored inside `Value::Object::members` to identify
/// trusted raw HTML produced by `safeHtml` or `escapeHtml`.
///
/// Because [`crate::json_to_template_value`] never produces [`Value::Object`],
/// untrusted JSON input cannot forge this marker.
const SAFE_HTML_MARKER_KEY: &str = "__neo_safe_html";

/// Internal auto-escape pipeline function name injected when
/// [`crate::TemplateOptions::html_escape`] is enabled.
pub(crate) const AUTO_ESCAPE_FN: &str = "__neo_auto_escape";

/// Escapes HTML special characters (`&`, `<`, `>`, `"`, `'`) in `input`.
///
/// Replacements performed:
/// - `&` -> `&amp;`
/// - `<` -> `&lt;`
/// - `>` -> `&gt;`
/// - `"` -> `&quot;`
/// - `'` -> `&#39;`
///
/// ```
/// use neojs_template_core::escape_html;
///
/// assert_eq!(
///     escape_html(r#"<a href="x" title='y'>a & b</a>"#),
///     "&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;a &amp; b&lt;/a&gt;"
/// );
/// ```
#[must_use]
pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Reports whether `value` carries the internal `safeHtml` marker.
pub(crate) fn is_safe_html(value: &Value) -> bool {
    matches!(
        value,
        Value::Object { members, .. }
            if members.get(SAFE_HTML_MARKER_KEY) == Some(&Value::Bool(true))
    )
}

/// Wraps `html` as a marked safe-HTML value that bypasses auto-escaping.
fn mark_safe_html(html: String) -> Value {
    Value::Object {
        display: html,
        members: BTreeMap::from([(SAFE_HTML_MARKER_KEY.to_owned(), Value::Bool(true))]),
    }
}

fn single_rendered_arg(name: &str, arguments: &[Value]) -> Result<String, GotmplError> {
    if arguments.len() != 1 {
        return Err(GotmplError::Function {
            name: name.to_owned(),
            message: format!("{name} expects 1 argument, received {}", arguments.len()),
        });
    }
    Ok(arguments[0].render())
}

/// Registers `escapeHtml`, `htmlEscape`, `safeHtml`, and (when `html_escape`
/// is `true`) the auto-escape helper into `functions`.
///
/// When `html_escape` is `true`, `escapeHtml` and `htmlEscape` return a
/// `safeHtml`-marked value so auto-escaping does not double-escape their output;
/// when `html_escape` is `false`, they return a plain `Value::String`.
pub(crate) fn register_html_helpers(functions: &mut FuncMap, html_escape: bool) {
    for alias in ["escapeHtml", "htmlEscape"] {
        let name = alias.to_owned();
        functions.register(alias, move |arguments: &[Value]| {
            let text = single_rendered_arg(&name, arguments)?;
            let escaped = escape_html(&text);
            if html_escape {
                Ok(mark_safe_html(escaped))
            } else {
                Ok(Value::String(escaped))
            }
        });
    }

    functions.register("safeHtml", |arguments: &[Value]| {
        if arguments.len() != 1 {
            return Err(GotmplError::Function {
                name: "safeHtml".to_owned(),
                message: format!("safeHtml expects 1 argument, received {}", arguments.len()),
            });
        }
        match &arguments[0] {
            Value::Object { display, members }
                if members.get(SAFE_HTML_MARKER_KEY) == Some(&Value::Bool(true)) =>
            {
                Ok(mark_safe_html(display.clone()))
            }
            other => Ok(mark_safe_html(other.render())),
        }
    });

    if html_escape {
        functions.register(AUTO_ESCAPE_FN, |arguments: &[Value]| {
            let Some(value) = arguments.first() else {
                return Ok(Value::String(String::new()));
            };
            if is_safe_html(value) {
                Ok(value.clone())
            } else {
                Ok(Value::String(escape_html(&value.render())))
            }
        });
    }
}
