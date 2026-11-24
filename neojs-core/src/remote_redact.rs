//! V8/`deno_core` error redaction for known remote module URLs.
//!
//! Only URLs recorded by [`neojs_remote::RemoteClient`] are replaced, and only
//! in generated locations: stack-frame file names, frame lines after the first,
//! and the generated ` at <URL>:line:column` suffix. Script-supplied exception
//! messages and custom stacks are preserved.

use std::collections::BTreeSet;

use deno_core::error::CoreError;
use deno_core::error::CoreErrorKind;
use deno_core::error::JsError;
use deno_core::url::Url;
use deno_error::JsErrorBox;
use neojs_remote::Redactions;
use neojs_remote::diagnostic_url;

/// Redacts known remote module URLs from a V8 diagnostic without touching the
/// script's exception value.
pub(crate) fn redact_core_error(error: CoreError, redactions: &Redactions) -> CoreError {
    let urls = redactions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if urls.is_empty() {
        return error;
    }
    let CoreError(kind) = error;
    let kind = match *kind {
        CoreErrorKind::Js(js) => CoreErrorKind::Js(Box::new(redact_js_error(*js, &urls))),
        other => {
            let original = CoreError(Box::new(other));
            let text = original.to_string();
            let redacted = redact_generated_mentions(&text, &urls, true, true);
            if redacted == text {
                return original;
            }
            CoreErrorKind::JsBox(JsErrorBox::generic(redacted))
        }
    };
    CoreError(Box::new(kind))
}

fn redact_js_error(mut error: JsError, urls: &BTreeSet<String>) -> JsError {
    for frame in &mut error.frames {
        if let Some(file_name) = frame.file_name.take() {
            frame.file_name = Some(redact_known_urls(&file_name, urls));
        }
        if let Some(eval_origin) = frame.eval_origin.take() {
            frame.eval_origin = Some(redact_known_urls(&eval_origin, urls));
        }
    }
    if !error.stack_is_custom {
        // V8 puts generated locations in syntax, link, and resolution errors.
        // The name and prefix must identify a generated shape; a user value is
        // preserved even if its text happens to contain "Cannot find module" or
        // an ` at <URL>:` fragment.
        let (generated_link, generated_location) =
            generated_flags(&error.exception_message, error.name.as_deref());
        if let Some(stack) = error.stack.take() {
            error.stack = Some(redact_stack(
                &stack,
                urls,
                generated_link,
                generated_location,
            ));
        }
        error.exception_message = redact_generated_mentions(
            &error.exception_message,
            urls,
            generated_link,
            generated_location,
        );
        error.message = error.message.take().map(|message| {
            let (generated_link, generated_location) =
                generated_flags(&message, error.name.as_deref());
            redact_generated_mentions(&message, urls, generated_link, generated_location)
        });
    }
    error.cause = error
        .cause
        .take()
        .map(|cause| Box::new(redact_js_error(*cause, urls)));
    error.aggregated = error.aggregated.take().map(|aggregated| {
        aggregated
            .into_iter()
            .map(|error| redact_js_error(error, urls))
            .collect()
    });
    error
}

/// Redacts generated frame lines. The first stack line is the exception value,
/// so it is only rewritten when the error is positively identified as a
/// generated link or location diagnostic.
fn redact_stack(
    stack: &str,
    urls: &BTreeSet<String>,
    generated_link: bool,
    generated_location: bool,
) -> String {
    let mut lines = stack.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut redacted = if generated_location {
        redact_generated_mentions(first, urls, generated_link, generated_location)
    } else {
        String::from(first)
    };
    for line in lines {
        redacted.push('\n');
        redacted.push_str(&redact_known_urls(line, urls));
    }
    redacted
}

/// Replaces known module URLs in generated diagnostic positions.
///
/// It handles ` at <URL>:line:column` and a URL quoted as a module specifier in
/// a link or resolution error. Other mentions, including an exception value that
/// carries an unrelated URL, are preserved.
fn redact_generated_mentions(
    message: &str,
    urls: &BTreeSet<String>,
    generated_link: bool,
    generated_location: bool,
) -> String {
    let mut redacted = message.to_string();
    for url in urls {
        let replacement = redact_url_string(url);
        if generated_location {
            redacted = redacted.replace(&format!(" at {url}:"), &format!(" at {replacement}:"));
        }
        if generated_link {
            redacted = redacted.replace(&format!("'{url}'"), &format!("'{replacement}'"));
            redacted = redacted.replace(&format!("\"{url}\""), &format!("\"{replacement}\""));
        }
    }
    if generated_link {
        redact_quoted_specifier_queries(&redacted)
    } else {
        redacted
    }
}

/// Returns whether a message has the structure of a generated V8 link or
/// location diagnostic.
///
/// The name and message prefix must both match a generated shape. A user-thrown
/// `Error` whose text happens to contain "Cannot find module" or ` at <URL>:`
/// does not qualify.
fn generated_flags(message: &str, name: Option<&str>) -> (bool, bool) {
    let diagnostic_name = matches!(name, Some("SyntaxError") | Some("TypeError"));
    let message = message
        .strip_prefix("Uncaught SyntaxError: ")
        .or_else(|| message.strip_prefix("Uncaught TypeError: "))
        .or_else(|| message.strip_prefix("Uncaught (in promise) SyntaxError: "))
        .or_else(|| message.strip_prefix("Uncaught (in promise) TypeError: "))
        .unwrap_or(message);
    let link = diagnostic_name
        && message.starts_with("The requested module ")
        && message.contains(" does not provide an export");
    let location = diagnostic_name
        && message.contains(" at ")
        && (link
            || [
                "Unexpected token",
                "Unexpected identifier",
                "Invalid or unexpected token",
                "Missing initializer",
                "Identifier '",
                "Invalid left-hand side",
            ]
            .iter()
            .any(|prefix| message.starts_with(prefix)));
    (link, location)
}

/// Removes the query from a quoted relative module specifier in a generated
/// message, such as `'./module.js?token=secret'` in a link error. Other quoted
/// text is preserved.
fn redact_quoted_specifier_queries(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let quote = bytes[index];
        if (quote == b'\'' || quote == b'"')
            && let Some(offset) = bytes[index + 1..].iter().position(|byte| *byte == quote)
        {
            let end = index + 1 + offset;
            let inner = &text[index + 1..end];
            if is_relative_specifier_with_query(inner) {
                output.push(quote);
                output
                    .extend_from_slice(inner.split(['?', '#']).next().unwrap_or(inner).as_bytes());
                output.push(quote);
                index = end + 1;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| text.to_string())
}

fn is_relative_specifier_with_query(specifier: &str) -> bool {
    let looks_relative = specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier.starts_with("https://")
        || specifier.starts_with("http://");
    looks_relative && specifier.contains('?') && !specifier.chars().any(char::is_whitespace)
}

/// Replaces exact known module URLs, longest first so one URL cannot corrupt a
/// longer one that shares its prefix.
fn redact_known_urls(text: &str, urls: &BTreeSet<String>) -> String {
    let mut urls = urls.iter().collect::<Vec<_>>();
    urls.sort_by_key(|url| std::cmp::Reverse(url.len()));
    let mut redacted = text.to_string();
    for url in urls {
        if redacted.contains(url) {
            redacted = redacted.replace(url, &redact_url_string(url));
        }
    }
    redacted
}

fn redact_url_string(url: &str) -> String {
    match Url::parse(url) {
        Ok(url) => diagnostic_url(&url).to_string(),
        Err(_) => url.to_string(),
    }
}
