//! Context-aware HTML escaping primitives and `safeHtml` / `escapeHtml` marker handling.

use std::borrow::Cow;
use std::collections::BTreeMap;

use neotales_gotmpl::{Error as GotmplError, FuncMap, Value};

/// Internal marker key stored inside `Value::Object::members` to identify
/// trusted raw HTML produced by `safeHtml`.
///
/// Because [`crate::json_to_template_value`] never produces [`Value::Object`],
/// untrusted JSON input cannot forge this marker.
const SAFE_HTML_MARKER_KEY: &str = "__vael_safe_html";

/// Internal marker key stored inside `Value::Object::members` to identify
/// HTML-entity-escaped text produced by `escapeHtml` / `htmlEscape`.
///
/// Unlike [`SAFE_HTML_MARKER_KEY`], this marker ONLY prevents double-escaping
/// in HTML text and quoted normal attribute contexts (`AUTO_ESCAPE_FN`), and
/// never bypasses URL scheme validation, unquoted attribute escaping, JS/`on*`
/// escaping, CSS filtering, or comment sanitization.
const ESCAPED_HTML_MARKER_KEY: &str = "__vael_escaped_html";

/// Internal auto-escape helper for HTML text and quoted normal attributes.
pub(crate) const AUTO_ESCAPE_FN: &str = "__vael_auto_escape";
/// Internal auto-escape helper for unquoted HTML attributes.
pub(crate) const AUTO_ESCAPE_UNQUOTED_FN: &str = "__vael_auto_escape_unquoted";
/// Internal auto-escape helper for the start of quoted URL attributes.
pub(crate) const AUTO_ESCAPE_URL_START_FN: &str = "__vael_auto_escape_url_start";
/// Internal auto-escape helper for the start of unquoted URL attributes.
pub(crate) const AUTO_ESCAPE_URL_START_UNQUOTED_FN: &str = "__vael_auto_escape_url_start_unquoted";
/// Internal auto-escape helper for subsequent actions in quoted URL attributes without a safe static prefix.
pub(crate) const AUTO_ESCAPE_URL_CONT_NO_COLON_FN: &str = "__vael_auto_escape_url_no_colon";
/// Internal auto-escape helper for subsequent actions in unquoted URL attributes without a safe static prefix.
pub(crate) const AUTO_ESCAPE_URL_CONT_NO_COLON_UNQUOTED_FN: &str =
    "__vael_auto_escape_url_no_colon_unquoted";
/// Internal auto-escape helper for JS string literals inside `<script>`.
pub(crate) const AUTO_ESCAPE_JS_STR_FN: &str = "__vael_auto_escape_js_str";
/// Internal auto-escape helper for JS template literals inside `<script>`.
pub(crate) const AUTO_ESCAPE_JS_TPL_FN: &str = "__vael_auto_escape_js_tpl";
/// Internal auto-escape helper for raw expressions inside `<script>`.
pub(crate) const AUTO_ESCAPE_JS_RAW_FN: &str = "__vael_auto_escape_js_raw";
/// Internal auto-escape helper for JS string literals inside quoted `on*` attributes.
pub(crate) const AUTO_ESCAPE_JS_ATTR_STR_FN: &str = "__vael_auto_escape_js_attr_str";
/// Internal auto-escape helper for JS template literals inside quoted `on*` attributes.
pub(crate) const AUTO_ESCAPE_JS_ATTR_TPL_FN: &str = "__vael_auto_escape_js_attr_tpl";
/// Internal auto-escape helper for raw JS expressions inside quoted `on*` attributes.
pub(crate) const AUTO_ESCAPE_JS_ATTR_RAW_FN: &str = "__vael_auto_escape_js_attr_raw";
/// Internal auto-escape helper for JS string literals inside unquoted `on*` attributes.
pub(crate) const AUTO_ESCAPE_JS_ATTR_STR_UNQUOTED_FN: &str =
    "__vael_auto_escape_js_attr_str_unquoted";
/// Internal auto-escape helper for JS template literals inside unquoted `on*` attributes.
pub(crate) const AUTO_ESCAPE_JS_ATTR_TPL_UNQUOTED_FN: &str =
    "__vael_auto_escape_js_attr_tpl_unquoted";
/// Internal auto-escape helper for raw JS expressions inside unquoted `on*` attributes.
pub(crate) const AUTO_ESCAPE_JS_ATTR_RAW_UNQUOTED_FN: &str =
    "__vael_auto_escape_js_attr_raw_unquoted";
/// Internal auto-escape helper for `<style>` and quoted `style="..."` CSS values.
pub(crate) const AUTO_ESCAPE_CSS_FN: &str = "__vael_auto_escape_css";
/// Internal auto-escape helper for unquoted `style=...` CSS values.
pub(crate) const AUTO_ESCAPE_CSS_UNQUOTED_FN: &str = "__vael_auto_escape_css_unquoted";
/// Internal auto-escape helper for HTML comments (`<!-- ... -->`).
pub(crate) const AUTO_ESCAPE_COMMENT_FN: &str = "__vael_auto_escape_comment";

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
/// use vaeljs_template_core::escape_html;
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

/// Escapes an unquoted HTML attribute value so it cannot break out of the
/// attribute or inject new attributes.
///
/// Escapes `&`, `<`, `>`, `"`, `'`, ASCII whitespace, ASCII control chars,
/// `=`, `/`, and `` ` ``.
pub(crate) fn escape_unquoted_attr(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '=' => escaped.push_str("&#61;"),
            '/' => escaped.push_str("&#47;"),
            '`' => escaped.push_str("&#96;"),
            ' ' => escaped.push_str("&#32;"),
            '\t' => escaped.push_str("&#9;"),
            '\n' => escaped.push_str("&#10;"),
            '\x0c' => escaped.push_str("&#12;"),
            '\r' => escaped.push_str("&#13;"),
            c if c.is_ascii_control() => escaped.push_str("&#xFFFD;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn is_safe_url(trimmed: &str) -> bool {
    let mut colon_pos = None;
    for (idx, ch) in trimmed.char_indices() {
        match ch {
            ':' => {
                colon_pos = Some(idx);
                break;
            }
            '/' | '?' | '#' => return true,
            _ => {}
        }
    }
    let Some(colon) = colon_pos else {
        return true;
    };
    let scheme = &trimmed[..colon];
    if scheme.bytes().any(|b| b <= 0x20 || b == 0x7F) {
        return false;
    }
    scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("mailto")
        || scheme.eq_ignore_ascii_case("tel")
}

/// Validates a URL attribute value at its start, trimming leading ASCII
/// whitespace and control characters before checking the scheme.
///
/// Allows relative paths/fragments/queries and `http:`, `https:`, `mailto:`,
/// `tel:` (case-insensitively). Dangerous or unknown schemes are replaced with
/// `#ZgotmplZ`.
pub(crate) fn sanitize_url(input: &str) -> Cow<'_, str> {
    let trimmed =
        input.trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch.is_ascii_control());
    if is_safe_url(trimmed) {
        Cow::Borrowed(trimmed)
    } else {
        Cow::Borrowed("#ZgotmplZ")
    }
}

/// Validates a subsequent URL action when no safe static URL prefix was
/// established before a prior action (`href="{{.a}}{{.b}}"`): rejects any
/// value containing `:` (as well as unsafe schemes) to `#ZgotmplZ`.
pub(crate) fn sanitize_url_no_colon(input: &str) -> Cow<'_, str> {
    if input.contains(':') {
        Cow::Borrowed("#ZgotmplZ")
    } else {
        sanitize_url(input)
    }
}

/// Escapes JavaScript string characters (`\`, `'`, `"`, `` ` ``, `<`, `>`, `&`,
/// `=`, `/`, `\n`, `\r`, `\u2028`, `\u2029`, and ASCII control chars) to
/// `\uXXXX` sequences.
///
/// When `escape_dollar` is `true`, `$` is also escaped to `\u0024` so
/// template-literal `${...}` interpolation cannot be injected.
pub(crate) fn escape_js_string(input: &str, escape_dollar: bool) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\u005c"),
            '\'' => out.push_str("\\u0027"),
            '"' => out.push_str("\\u0022"),
            '`' => out.push_str("\\u0060"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '=' => out.push_str("\\u003d"),
            '/' => out.push_str("\\u002f"),
            '\n' => out.push_str("\\u000a"),
            '\r' => out.push_str("\\u000d"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            '$' if escape_dollar => out.push_str("\\u0024"),
            c if c.is_ascii_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Safely serializes a template value as a JavaScript expression outside quotes.
pub(crate) fn escape_js_raw_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(true) => "true".to_owned(),
        Value::Bool(false) => "false".to_owned(),
        Value::Number(n) | Value::Float(n) => {
            if n.is_finite() {
                if *n == 0.0 && n.is_sign_negative() {
                    "0".to_owned()
                } else {
                    value.render()
                }
            } else {
                "null".to_owned()
            }
        }
        Value::String(s) => format!("\"{}\"", escape_js_string(s, false)),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(escape_js_raw_value).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    format!(
                        "\"{}\":{}",
                        escape_js_string(k, false),
                        escape_js_raw_value(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Object { .. } | Value::Method { .. } => {
            format!("\"{}\"", escape_js_string(&value.render(), false))
        }
    }
}

/// Allows only safe CSS property values (`[A-Za-z0-9#.%,-+_ ]`) without CSS
/// injection tokens; unsafe inputs are replaced with `ZgotmplZ`.
pub(crate) fn escape_css_value(input: &str) -> Cow<'_, str> {
    let all_chars_safe = input.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '#' | '%' | '.' | ',' | '-' | '+' | '_' | ' ')
    });
    if !all_chars_safe {
        return Cow::Borrowed("ZgotmplZ");
    }
    let lower = input.to_ascii_lowercase();
    for forbidden in [
        "expression",
        "url(",
        "@import",
        "behavior",
        "-moz-binding",
        "</style",
        "/*",
    ] {
        if lower.contains(forbidden) {
            return Cow::Borrowed("ZgotmplZ");
        }
    }
    Cow::Borrowed(input)
}

/// Sanitizes HTML comment contents so `--`, `!`, `<`, `>`, and leading/trailing
/// `-` cannot break out of `<!-- ... -->` or `<!-- -...>` via `-->` / `--!>`.
pub(crate) fn escape_comment(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    for (idx, ch) in input.char_indices() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '!' => out.push_str("&#33;"),
            '-' => {
                let at_start = idx == 0;
                let prev_hyphen = idx > 0 && bytes[idx - 1] == b'-';
                let next_hyphen = idx + 1 < bytes.len() && bytes[idx + 1] == b'-';
                let at_end = idx + 1 == bytes.len();
                if at_start || prev_hyphen || next_hyphen || at_end {
                    out.push_str("&#45;");
                } else {
                    out.push('-');
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Reports whether `value` carries the internal `safeHtml` marker.
pub(crate) fn is_safe_html(value: &Value) -> bool {
    matches!(
        value,
        Value::Object { members, .. }
            if members.get(SAFE_HTML_MARKER_KEY) == Some(&Value::Bool(true))
    )
}

/// Reports whether `value` carries the internal `escapeHtml` marker.
fn is_escaped_html(value: &Value) -> bool {
    matches!(
        value,
        Value::Object { members, .. }
            if members.get(ESCAPED_HTML_MARKER_KEY) == Some(&Value::Bool(true))
    )
}

/// Wraps `html` as a marked safe-HTML value that bypasses all auto-escaping.
fn mark_safe_html(html: String) -> Value {
    Value::Object {
        display: html,
        members: BTreeMap::from([(SAFE_HTML_MARKER_KEY.to_owned(), Value::Bool(true))]),
    }
}

/// Wraps `html` as an HTML-entity-escaped value that only suppresses
/// double-escaping in HTML text and quoted normal attributes.
fn mark_escaped_html(html: String) -> Value {
    Value::Object {
        display: html,
        members: BTreeMap::from([(ESCAPED_HTML_MARKER_KEY.to_owned(), Value::Bool(true))]),
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

fn register_context_escape<F>(
    functions: &mut FuncMap,
    name: &str,
    allow_escaped_html_bypass: bool,
    escape_fn: F,
) where
    F: Fn(&Value) -> String + Send + Sync + 'static,
{
    functions.register(name, move |arguments: &[Value]| {
        let Some(value) = arguments.first() else {
            return Ok(Value::String(String::new()));
        };
        if is_safe_html(value) || (allow_escaped_html_bypass && is_escaped_html(value)) {
            Ok(value.clone())
        } else {
            Ok(Value::String(escape_fn(value)))
        }
    });
}

/// Registers `escapeHtml`, `htmlEscape`, `safeHtml`, and (when `html_escape`
/// is `true`) the context-aware auto-escape helpers into `functions`.
///
/// When `html_escape` is `true`, `escapeHtml` and `htmlEscape` return an
/// `__vael_escaped_html`-marked value so HTML text and quoted normal attribute
/// auto-escaping do not double-escape their output, while specialized contexts
/// (URL, unquoted attributes, JS/`on*`, CSS, comments) still apply their own
/// sanitization.
pub(crate) fn register_html_helpers(functions: &mut FuncMap, html_escape: bool) {
    for alias in ["escapeHtml", "htmlEscape"] {
        let name = alias.to_owned();
        functions.register(alias, move |arguments: &[Value]| {
            let text = single_rendered_arg(&name, arguments)?;
            let escaped = escape_html(&text);
            if html_escape {
                Ok(mark_escaped_html(escaped))
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
        register_context_escape(functions, AUTO_ESCAPE_FN, true, |v| {
            escape_html(&v.render())
        });
        register_context_escape(functions, AUTO_ESCAPE_UNQUOTED_FN, false, |v| {
            escape_unquoted_attr(&v.render())
        });
        register_context_escape(functions, AUTO_ESCAPE_URL_START_FN, false, |v| {
            escape_html(&sanitize_url(&v.render()))
        });
        register_context_escape(functions, AUTO_ESCAPE_URL_START_UNQUOTED_FN, false, |v| {
            escape_unquoted_attr(&sanitize_url(&v.render()))
        });
        register_context_escape(functions, AUTO_ESCAPE_URL_CONT_NO_COLON_FN, false, |v| {
            escape_html(&sanitize_url_no_colon(&v.render()))
        });
        register_context_escape(
            functions,
            AUTO_ESCAPE_URL_CONT_NO_COLON_UNQUOTED_FN,
            false,
            |v| escape_unquoted_attr(&sanitize_url_no_colon(&v.render())),
        );
        register_context_escape(functions, AUTO_ESCAPE_JS_STR_FN, false, |v| {
            escape_js_string(&v.render(), false)
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_TPL_FN, false, |v| {
            escape_js_string(&v.render(), true)
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_RAW_FN, false, escape_js_raw_value);
        register_context_escape(functions, AUTO_ESCAPE_JS_ATTR_STR_FN, false, |v| {
            escape_html(&escape_js_string(&v.render(), false))
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_ATTR_TPL_FN, false, |v| {
            escape_html(&escape_js_string(&v.render(), true))
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_ATTR_RAW_FN, false, |v| {
            escape_html(&escape_js_raw_value(v))
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_ATTR_STR_UNQUOTED_FN, false, |v| {
            escape_unquoted_attr(&escape_js_string(&v.render(), false))
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_ATTR_TPL_UNQUOTED_FN, false, |v| {
            escape_unquoted_attr(&escape_js_string(&v.render(), true))
        });
        register_context_escape(functions, AUTO_ESCAPE_JS_ATTR_RAW_UNQUOTED_FN, false, |v| {
            escape_unquoted_attr(&escape_js_raw_value(v))
        });
        register_context_escape(functions, AUTO_ESCAPE_CSS_FN, false, |v| {
            escape_html(&escape_css_value(&v.render()))
        });
        register_context_escape(functions, AUTO_ESCAPE_CSS_UNQUOTED_FN, false, |v| {
            escape_unquoted_attr(&escape_css_value(&v.render()))
        });
        register_context_escape(functions, AUTO_ESCAPE_COMMENT_FN, false, |v| {
            escape_comment(&v.render())
        });
    }
}
